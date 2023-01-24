use std::fmt::Debug;
use std::time::{Duration, Instant};
use std::{io::Read, net::SocketAddr};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::Stream;
use postcard::experimental::max_size::MaxSize;
use s2n_quic::Connection;
use s2n_quic::{client::Connect, Client};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::blobs::Blobs;
use crate::protocol::{read_lp_data, write_lp, AuthToken, Handshake, Request, Res, Response};
use crate::tls::{self, Keypair, PeerId};

const MAX_DATA_SIZE: usize = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Options {
    pub addr: SocketAddr,
    pub peer_id: Option<PeerId>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            addr: "127.0.0.1:4433".parse().unwrap(),
            peer_id: None,
        }
    }
}

/// Setup a QUIC connection to the provided address.
async fn setup(opts: Options) -> Result<(Client, Connection)> {
    let keypair = Keypair::generate();

    let client_config = tls::make_client_config(&keypair, opts.peer_id)?;
    let tls = s2n_quic::provider::tls::rustls::Client::from(client_config);

    let client = Client::builder()
        .with_tls(tls)?
        .with_io("0.0.0.0:0")?
        .start()
        .map_err(|e| anyhow!("{:?}", e))?;

    debug!("connecting to {}", opts.addr);
    let connect = Connect::new(opts.addr).with_server_name("localhost");
    let mut connection = client.connect(connect).await?;

    connection.keep_alive(true)?;
    Ok((client, connection))
}

/// Stats about the transfer.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub data_len: usize,
    pub elapsed: Duration,
    pub mbits: f64,
}

/// The events that are emitted while running a transfer.
pub enum Event {
    /// The connection to the provider was established.
    Connected,
    /// The provider has the content.
    Requested {
        /// The size of the requested content.
        size: usize,
    },
    /// Content is being received.
    Receiving {
        /// The hash of the content we received.
        hash: bao::Hash,
        /// The actual data we are receiving.
        reader: Box<dyn AsyncRead + Unpin + Sync + Send + 'static>,
        /// An optional name associated with the data
        name: Option<String>,
    },
    /// The transfer is done.
    Done(Stats),
}

impl Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => write!(f, "Event::Connected"),
            Self::Requested { size } => write!(f, "Event::Requested {{ {size} }}"),
            Self::Receiving { hash, name, .. } => {
                write!(
                    f,
                    "Event::Receiving {{ hash: {hash}, reader: Box<AsyncReader>, name: {:#?} }}",
                    name
                )
            }
            Self::Done(s) => write!(f, "Event::Done({:#?})", s),
        }
    }
}

pub fn run(hash: bao::Hash, token: AuthToken, opts: Options) -> impl Stream<Item = Result<Event>> {
    async_stream::try_stream! {
        let now = Instant::now();
        let (_client, mut connection) = setup(opts).await?;

        let stream = connection.open_bidirectional_stream().await?;
        let (mut reader, mut writer) = stream.split();

        yield Event::Connected;


        let mut out_buffer = BytesMut::zeroed(std::cmp::max(
            Request::POSTCARD_MAX_SIZE,
            Handshake::POSTCARD_MAX_SIZE,
        ));

        // 1. Send Handshake
        {
            debug!("sending handshake");
            let handshake = Handshake::new(token);
            let used = postcard::to_slice(&handshake, &mut out_buffer)?;
            write_lp(&mut writer, used).await?;
        }

        // 2. Send Request
        {
            debug!("sending request");
            let req = Request {
                id: 1,
                name: hash.into(),
            };

            let used = postcard::to_slice(&req, &mut out_buffer)?;
            write_lp(&mut writer, used).await?;
        }

        // 3. Read response
        {
            debug!("reading response");
            let mut in_buffer = BytesMut::with_capacity(1024);

            // track total amount of blob data transfered
            let mut data_len = 0;
            // read next message
            match read_lp_data(&mut reader, &mut in_buffer).await? {
                Some(response_buffer) => {
                    let response: Response = postcard::from_bytes(&response_buffer)?;
                    match response.data {
                        // server is sending over a collection of blobs
                        Res::FoundCollection { size, outboard, raw_transfer_size } => {
                            if raw_transfer_size > MAX_DATA_SIZE {

                                Err(anyhow!("size too large: {} > {}", raw_transfer_size, MAX_DATA_SIZE))?;
                            }
                            data_len = raw_transfer_size;

                            yield Event::Requested { size: raw_transfer_size };

                            // decode the collection
                            let collection = read_and_decode_collection_data(size, outboard, hash, &mut reader, &mut in_buffer).await?;

                            for blob in collection.blobs {
                                // read next message
                                match read_lp_data(&mut reader, &mut in_buffer).await? {
                                    Some(response_buffer) => {
                                        let response: Response = postcard::from_bytes(&response_buffer)?;
                                        match response.data {
                                            // unexpected message
                                            Res::FoundCollection { .. } => {
                                                Err(anyhow!("unexpected message from server. ending transfer early"))?;
                                            },
                                            // blob data not found
                                            Res::NotFound => {
                                                Err(anyhow!("data for {} not found", bao::Hash::from(blob.hash).to_hex()))?;
                                             },
                                            // next blob in collection will be sent over
                                            Res::Found { size, outboard } => {
                                                println!("Res::Found size {size}");
                                                let (event, task) = read_and_decode_blob_data(size, outboard, bao::Hash::from(blob.hash), Some(blob.name), &mut reader, &mut in_buffer).await?;

                                                yield event;
                                                task.await??;

                                            }
                                        }
                                     },
                                     None => {
                                        Err(anyhow!("server disconnected"))?;
                                     }
                                }
                            }
                        }
                        // server is sending over a single blob
                        Res::Found { size, outboard } => {
                            yield Event::Requested { size };
                            println!("got FOUND response with data size {size}");
                            if size > MAX_DATA_SIZE {
                                Err(anyhow!("size too large: {} > {}", size, MAX_DATA_SIZE))?;
                            }
                            data_len = size;

                            let (event, task) = read_and_decode_blob_data(size, outboard, hash, None, reader, &mut in_buffer).await?;
                            yield event;

                            task.await??;
                        }
                        // data associated with the hash is not found
                        Res::NotFound => {
                            Err(anyhow!("data not found"))?;
                        }
                    }

                    // Shut down the stream
                    println!("shutting down stream");
                    debug!("shutting down stream");
                    writer.close().await?;

                    let elapsed = now.elapsed();
                    let elapsed_s = elapsed.as_secs_f64();
                    let data_len_bit = data_len * 8;
                    let mbits = data_len_bit as f64 / (1000. * 1000.) / elapsed_s;

                    let stats = Stats {
                        data_len,
                        elapsed,
                        mbits,
                    };

                    yield Event::Done(stats);
                }
                None => {
                    Err(anyhow!("provider disconnected"))?;
                }
            }
        }
    }
}

/// reads data into a passed in buffer, returning a buffer of length `size`, and clearing
/// the original buffer of the already read data
async fn read_data<R: AsyncRead + Unpin>(
    size: usize,
    mut reader: R,
    mut buffer: &mut BytesMut,
) -> Result<BytesMut> {
    // TODO: avoid buffering
    while buffer.len() < size {
        reader.read_buf(&mut buffer).await?;
    }
    if size > buffer.len() {
        Err(anyhow!(
            "expected {} bytes, got {} bytes",
            size,
            buffer.len()
        ))?;
    }

    debug!("received data: {}bytes", size);
    Ok(buffer.split_to(size))
}

async fn read_and_decode_blob_data<R: AsyncRead + Unpin>(
    size: usize,
    outboard: &[u8],
    hash: bao::Hash,
    name: Option<String>,
    reader: R,
    buffer: &mut BytesMut,
) -> Result<(Event, tokio::task::JoinHandle<Result<()>>)> {
    let data = read_data(size, reader, buffer).await?;
    let (a, mut b) = tokio::io::duplex(1024);
    println!("data size: {:#?}", size);

    // TODO: avoid copy
    let outboard = outboard.to_vec();
    let t = tokio::task::spawn(async move {
        println!("in task...");
        let mut buf = [0u8; 1024];
        let mut decoder =
            bao::decode::Decoder::new_outboard(std::io::Cursor::new(&data[..]), &*outboard, &hash);

        loop {
            // TODO: avoid blocking
            let read_res = decoder.read(&mut buf);
            println!("read_res: {:#?}", read_res);
            let read = read_res?;
            if read == 0 {
                break;
            }
        }
        // tokio::task::spawn_blocking(move || {
        //     let mut decoder = bao::decode::Decoder::new_outboard(
        //         std::io::Cursor::new(&data[..]),
        //         &*outboard,
        //         &hash,
        //     );

        //     loop {
        //         loops += 1;
        //         // TODO: avoid blocking
        //         println!("decoder loop {loops}");
        //         let read = decoder.read(&mut buf)?;
        //         if read == 0 {
        //             println!("break!");
        //             break Ok::<(), anyhow::Error>(());
        //         }
        //         // b.write(&buf[..read]).await?;
        //     }
        // })
        // .await??;

        println!("about to write:");
        // let written = b.write(&d).await?;
        let mut written = 0;
        while written < size {
            let written_res = b.write(&data[written..]).await;
            println!("written_res: {:#?}", written_res);
            written += written_res?;
            tokio::task::yield_now().await;
            println!("wrote {written} so far");
        }
        println!("wrote {written}!");
        b.flush().await?;
        debug!("finished writing");
        println!("finished writing");
        Ok::<(), anyhow::Error>(())
    });

    Ok((
        Event::Receiving {
            hash,
            reader: Box::new(a),
            name,
        },
        t,
    ))
}

async fn read_and_decode_collection_data<R: AsyncRead + Unpin>(
    size: usize,
    outboard: &[u8],
    hash: bao::Hash,
    reader: R,
    buffer: &mut BytesMut,
) -> Result<Blobs> {
    let data = read_data(size, reader, buffer).await?;
    // TODO: avoid copy
    let outboard = outboard.to_vec();
    let mut decoder =
        bao::decode::Decoder::new_outboard(std::io::Cursor::new(&data[..]), &*outboard, &hash);

    let mut buf = [0u8; 1024];
    loop {
        // TODO: avoid blocking
        let read = decoder.read(&mut buf)?;
        if read == 0 {
            break;
        }
    }
    let blobs: Blobs = postcard::from_bytes(&data)?;
    Ok(blobs)
}
