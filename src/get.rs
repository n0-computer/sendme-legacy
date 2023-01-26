use std::fmt::Debug;
use std::time::Duration;
use std::{net::SocketAddr, time::Instant};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::Stream;
use postcard::experimental::max_size::MaxSize;
use s2n_quic::stream::ReceiveStream;
use s2n_quic::Connection;
use s2n_quic::{client::Connect, Client};
use tracing::debug;

use crate::bao_slice_decoder::AsyncSliceDecoder;
use crate::protocol::{read_lp, write_lp, AuthToken, Handshake, Request, Res, Response};
use crate::tls::{self, Keypair, PeerId};

const MAX_DATA_SIZE: u64 = 1024 * 1024 * 1024;

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
    pub data_len: u64,
    pub elapsed: Duration,
    pub mbits: f64,
}

/// The events that are emitted while running a transfer.
#[allow(clippy::large_enum_variant)]
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
        reader: AsyncSliceDecoder<tokio::io::BufReader<ReceiveStream>>,
    },
    /// The transfer is done.
    Done(Stats),
}

impl Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => write!(f, "Connected"),
            Self::Requested { size } => f.debug_struct("Requested").field("size", size).finish(),
            Self::Receiving { hash, reader: _ } => f
                .debug_struct("Receiving")
                .field("hash", hash)
                .field(
                    "reader",
                    &"Box<dyn AsyncRead + Unpin + Sync + Send + 'static>",
                )
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
        let (reader, mut writer) = stream.split();
        let mut reader = tokio::io::BufReader::new(reader);

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
            let mut in_buffer = BytesMut::with_capacity(Response::POSTCARD_MAX_SIZE);

            // read next message
            match read_lp::<_, Response>(&mut reader, &mut in_buffer).await? {
                Some((response, _)) => {
                    match response.data {
                        Res::Found(size) => {
                            yield Event::Requested { size, };

                            // Need to read the message now
                            if size as u64 > MAX_DATA_SIZE {
                                Err(anyhow!("size too large: {} > {}", size, MAX_DATA_SIZE))?;
                            }

                            // TODO: this means that the stats below are not correct
                            let decoder = crate::bao_slice_decoder::AsyncSliceDecoder::new(reader, hash, 0, size as u64);
                            yield Event::Receiving { hash, reader: decoder };


                            // Shut down the stream
                            debug!("shutting down stream");
                            writer.close().await?;

                            let data_len = size as u64;
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
