use std::fmt::Debug;
use std::time::Duration;
use std::{io::Read, net::SocketAddr, time::Instant};

use anyhow::{anyhow, Result};
use bytes::{BufMut, BytesMut};
use futures::Stream;
use postcard::experimental::max_size::MaxSize;
use s2n_quic::Connection;
use s2n_quic::{client::Connect, Client};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::blobs::{Blob, Blobs};
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
#[derive(Debug)]
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
            Self::Connected => write!(f, "Connected"),
            Self::Requested { size } => f.debug_struct("Requested").field("size", size).finish(),
            Self::Receiving {
                hash,
                reader: _,
                name,
            } => f
                .debug_struct("Receiving")
                .field("hash", hash)
                .field(
                    "reader",
                    &"Box<dyn AsyncRead + Unpin + Sync + Send + 'static>",
                )
                .field("name", name)
                .finish(),
            Self::Done(arg0) => f.debug_tuple("Done").field(arg0).finish(),
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

            // read next message
            match read_lp_data(&mut reader, &mut in_buffer).await? {
                Some(response_buffer) => {
                    let response: Response = postcard::from_bytes(&response_buffer)?;
                    match response.data {
                        Res::FoundCollection { size, outboard, raw_transfer_size } => {
                            if raw_transfer_size > MAX_DATA_SIZE {

                                Err(anyhow!("size too large: {} > {}", size, MAX_DATA_SIZE))?;
                            }

                            yield Event::Requested { size: raw_transfer_size };

                            let collection = read_collection(size, outboard, hash, &mut reader, &mut in_buffer).await?;
                            for blob in collection.blobs {
                                let (event, t) = read_blob(blob, &mut reader, &mut in_buffer).await?;
                                yield event;
                                t.await??;
                            }

                            todo!();
                        }
                        Res::Found { size, outboard, .. } => {
                            yield Event::Requested { size };
                            if size > MAX_DATA_SIZE {
                                Err(anyhow!("size too large: {} > {}", size, MAX_DATA_SIZE))?;
                            }

                            let (event, task) = read_blob_data(size, outboard, hash, reader).await?;
                            yield event;

                            task.await??;

                            // Shut down the stream
                            debug!("shutting down stream");
                            writer.close().await?;

                            let data_len = size;
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
                        Res::NotFound => {
                            Err(anyhow!("data not found"))?;
                        }
                    }
                }
                None => {
                    Err(anyhow!("provider disconnected"))?;
                }
            }
        }
    }
}

async fn read_blob_data<R: AsyncRead + Unpin>(
    size: usize,
    outboard: &[u8],
    hash: bao::Hash,
    mut reader: R,
) -> Result<(Event, tokio::task::JoinHandle<Result<()>>)> {
    // TODO: avoid buffering
    // TODO: buffer is moved into the task. would like help figuring out a way to not re-allocate
    // for each time this fn is called
    let mut buffer = BytesMut::with_capacity(1024);
    while buffer.len() < size {
        reader.read_buf(&mut buffer).await?;
    }

    debug!("received data: {}bytes", buffer.len());
    if size != buffer.len() {
        Err(anyhow!(
            "expected {} bytes, got {} bytes",
            size,
            buffer.len()
        ))?;
    }
    let (a, mut b) = tokio::io::duplex(1024);

    // TODO: avoid copy
    let outboard = outboard.to_vec();
    let t = tokio::task::spawn(async move {
        let mut decoder = bao::decode::Decoder::new_outboard(
            std::io::Cursor::new(&buffer[..]),
            &*outboard,
            &hash,
        );

        let mut buf = [0u8; 1024];
        loop {
            // TODO: avoid blocking
            let read = decoder.read(&mut buf)?;
            if read == 0 {
                break;
            }
            b.write_all(&buf[..read]).await?;
        }
        b.flush().await?;
        debug!("finished writing");
        Ok::<(), anyhow::Error>(())
    });

    Ok((
        Event::Receiving {
            hash,
            reader: Box::new(a),
            name: None,
        },
        t,
    ))
}

async fn read_collection<R: AsyncRead + Unpin>(
    size: usize,
    outboard: &[u8],
    hash: bao::Hash,
    mut reader: R,
    buffer: &mut BytesMut,
) -> Result<Blobs> {
    // TODO: avoid buffering
    while buffer.len() < size {
        reader.read_buf(buffer).await?;
    }

    debug!("received data: {}bytes", buffer.len());
    if size != buffer.len() {
        Err(anyhow!(
            "expected {} bytes, got {} bytes",
            size,
            buffer.len()
        ))?;
    }

    let mut data = BytesMut::with_capacity(size);

    // TODO: avoid copy
    let outboard = outboard.to_vec();
    let mut decoder =
        bao::decode::Decoder::new_outboard(std::io::Cursor::new(&buffer[..]), &*outboard, &hash);

    let mut buf = [0u8; 1024];
    loop {
        // TODO: avoid blocking
        let read = decoder.read(&mut buf)?;
        if read == 0 {
            break;
        }
        data.put(&buf[..read]);
    }
    let blobs: Blobs = postcard::from_bytes(&data)?;
    Ok(blobs)
}

async fn read_blob<R: AsyncRead + Unpin>(
    blob: Blob,
    mut reader: R,
    buffer: &mut BytesMut,
) -> Result<(Event, tokio::task::JoinHandle<Result<()>>)> {
    todo!();
}
