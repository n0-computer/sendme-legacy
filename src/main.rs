use std::{net::SocketAddr, path::PathBuf};

use anyhow::{ensure, Result};
use clap::{Parser, Subcommand};
use console::style;
use futures::StreamExt;
use indicatif::{HumanDuration, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use sendme::{client, server};

#[derive(Parser, Debug, Clone)]
#[clap(version, about, long_about = None)]
#[clap(about = "Send data.")]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Serve the data from the given path. If none is specified reads from STDIN.
    #[clap(about = "Serve the data from the given path")]
    Server {
        path: Option<PathBuf>,
        #[clap(long, short)]
        /// Optional port, defaults to 127.0.01:4433.
        addr: Option<SocketAddr>,
    },
    /// Fetch some data
    #[clap(about = "Fetch the data from the hash")]
    Client {
        hash: bao::Hash,
        #[clap(long, short)]
        /// Optional address of the server, defaults to 127.0.0.1:4433.
        addr: Option<SocketAddr>,
        /// Optional path to save the file. If none is specified writes the data to STDOUT.
        out: Option<PathBuf>,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Client { hash, addr, out } => {
            println!("Fetching: {}", hash.to_hex());
            let mut opts = client::Options::default();
            if let Some(addr) = addr {
                opts.addr = addr;
            }

            println!("{} Connecting ...", style("[1/3]").bold().dim());
            let pb = ProgressBar::hidden();
            let stream = client::run(hash, opts);
            tokio::pin!(stream);
            while let Some(event) = stream.next().await {
                match event? {
                    client::Event::Connected => {
                        println!("{} Requesting ...", style("[2/3]").bold().dim());
                    }
                    client::Event::Requested { size } => {
                        println!("{} Downloading ...", style("[3/3]").bold().dim());
                        pb.set_style(
                            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                                .unwrap()
                                .with_key("eta", |state: &ProgressState, w: &mut dyn std::fmt::Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
                                .progress_chars("#>-")
                        );
                        pb.set_length(size as u64);
                        pb.set_draw_target(ProgressDrawTarget::stderr());
                    }
                    client::Event::Received {
                        hash: new_hash,
                        mut data,
                    } => {
                        ensure!(hash == new_hash, "invalid hash received");
                        if let Some(ref out) = out {
                            let file = tokio::fs::File::create(out).await?;
                            let file = tokio::io::BufWriter::new(file);
                            // wrap for progress bar
                            let mut file = pb.wrap_async_write(file);
                            tokio::io::copy(&mut data, &mut file).await?;
                        } else {
                            // Write to STDOUT
                            let mut stdout = tokio::io::stdout();
                            tokio::io::copy(&mut data, &mut stdout).await?;
                        }
                    }
                    client::Event::Done(stats) => {
                        pb.finish_and_clear();

                        println!("Done in {}", HumanDuration(stats.elapsed));
                    }
                }
            }
        }
        Commands::Server { path, addr } => {
            let sources = if let Some(path) = path {
                vec![server::DataSource::File(path)]
            } else {
                vec![server::DataSource::Stdin(tokio::io::stdin())]
            };
            let db = server::create_db(sources).await?;
            let mut opts = server::Options::default();
            if let Some(addr) = addr {
                opts.addr = addr;
            }
            server::run(db, opts).await?
        }
    }

    Ok(())
}
