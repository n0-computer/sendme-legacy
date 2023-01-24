mod blobs;
pub mod get;
pub mod protocol;
pub mod provider;

mod tls;

pub use tls::{Keypair, PeerId, PeerIdError, PublicKey, SecretKey, Signature};

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use crate::get::Event;
    use crate::protocol::AuthToken;
    use crate::tls::PeerId;

    use super::*;
    use anyhow::Result;
    use futures::StreamExt;
    use rand::RngCore;
    use testdir::testdir;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn basics() -> Result<()> {
        let filename = "hello_world";
        let port: u16 = 4443;
        single_file(filename, "hello world!".as_bytes(), false, port).await?;
        single_file(filename, "hello world!".as_bytes(), true, port).await
    }

    // Run the test for a single file, with the option to transfer it as part of a collection or on
    // its own
    // TODO: random ports? rather than assigning?
    async fn single_file(filename: &str, data: &[u8], wrapped: bool, port: u16) -> Result<()> {
        let dir: PathBuf = testdir!();
        let path = dir.join(filename);
        let (_, expect_hash) = bao::encode::outboard(data);
        tokio::fs::write(&path, data).await?;

        // hash of the transfer file
        let expect_name = {
            if wrapped {
                Some(filename.to_string())
            } else {
                None
            }
        };

        let (db, collection_db) =
            provider::create_db(vec![provider::DataSource::File(path.clone())]).await?;
        // hash of the whole collection
        let collection_hash = *collection_db.iter().next().unwrap().0;
        let addr = format!("127.0.0.1:{port}").parse().unwrap();
        let mut provider = provider::Provider::builder()
            .database(db)
            .collection_database(collection_db)
            .build()?;
        let peer_id = provider.peer_id();
        let token = provider.auth_token();

        let provider_task = tokio::task::spawn(async move {
            provider.run(provider::Options { addr }).await.unwrap();
        });

        let opts = get::Options {
            addr,
            peer_id: Some(peer_id),
        };
        // let stream = client::run(collection_hash, opts);
        let stream = {
            if wrapped {
                println!("fetching collection hash {}", collection_hash);
                get::run(collection_hash, token, opts)
            } else {
                println!("fetching file hash {}", expect_hash);
                get::run(expect_hash, token, opts)
            }
        };
        tokio::pin!(stream);

        // needs to iterate until `done`
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            let event = event?;
            if let Event::Receiving {
                hash: got_hash,
                mut reader,
                name: got_name,
            } = event
            {
                assert_eq!(expect_hash, got_hash);
                let expect = tokio::fs::read(&path).await?;
                let mut got = Vec::new();
                reader.read_to_end(&mut got).await?;
                assert_eq!(expect, got);
                assert_eq!(expect_name, got_name);
            }
        }

        provider_task.abort();
        let _ = provider_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn multi_file() -> Result<()> {
        let dir: PathBuf = testdir!();
        let file_opts = vec![
            ("1", 10),
            ("2", 1024),
            ("3", 1024 * 1024),
            // overkill, but it works! Just annoying to wait for
            // ("4", 1024 * 1024 * 90),
        ];

        // create and save files
        let mut files = Vec::new();
        let mut expects = Vec::new();
        for opt in file_opts {
            let (name, size) = opt;
            let path = dir.join(name);
            let mut content = vec![0u8; size];
            rand::thread_rng().fill_bytes(&mut content);

            // get expected hash of file
            let (_, hash) = bao::encode::outboard(&content);

            tokio::fs::write(&path, content).await?;
            files.push(provider::DataSource::File(path.clone()));

            // keep track of expected values
            expects.push((Some(name.to_string()), path, hash));
        }

        let (db, collection_db) = provider::create_db(files).await?;
        // hash of the whole collection
        let collection_hash = *collection_db.iter().next().unwrap().0;
        let addr = "127.0.0.1:4446".parse().unwrap();
        let mut provider = provider::Provider::builder()
            .database(db)
            .collection_database(collection_db)
            .build()?;
        let peer_id = provider.peer_id();
        let token = provider.auth_token();

        let provider_task = tokio::task::spawn(async move {
            provider.run(provider::Options { addr }).await.unwrap();
        });

        let opts = get::Options {
            addr,
            peer_id: Some(peer_id),
        };
        let stream = get::run(collection_hash, token, opts);

        tokio::pin!(stream);

        // needs to iterate until `done`
        tokio::pin!(stream);
        let mut i = 0;
        while let Some(event) = stream.next().await {
            let event = event?;
            if let Event::Receiving {
                hash: got_hash,
                mut reader,
                name: got_name,
            } = event
            {
                let (expect_name, path, expect_hash) = expects.get(i).unwrap();
                assert_eq!(*expect_hash, got_hash);
                let expect = tokio::fs::read(&path).await?;
                let mut got = Vec::new();
                reader.read_to_end(&mut got).await?;
                assert_eq!(expect, got);
                assert_eq!(*expect_name, got_name);
                i += 1;
            }
        }

        provider_task.abort();
        let _ = provider_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn sizes() -> Result<()> {
        let sizes = [
            10,
            100,
            1024,
            1024 * 100,
            1024 * 500,
            1024 * 1024,
            1024 * 1024 + 10,
        ];
        let port: u16 = 4445;

        for size in sizes {
            let mut content = vec![0u8; size];
            rand::thread_rng().fill_bytes(&mut content);
            single_file("hello_world", &content, false, port).await?;
            single_file("hello_world", &content, true, port).await?;
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_clients() -> Result<()> {
        let dir: PathBuf = testdir!();
        let filename = "hello_world";
        let path = dir.join(filename);
        let content = b"hello world!";
        let addr = "127.0.0.1:4444".parse().unwrap();

        tokio::fs::write(&path, content).await?;
        // hash of the transfer file
        let data = tokio::fs::read(&path).await?;
        let (_, expect_hash) = bao::encode::outboard(&data);
        let expect_name = Some(filename.to_string());

        let (db, collection_db) =
            provider::create_db(vec![provider::DataSource::File(path)]).await?;
        let hash = *collection_db.iter().next().unwrap().0;
        let mut provider = provider::Provider::builder()
            .database(db)
            .collection_database(collection_db)
            .build()?;
        let peer_id = provider.peer_id();
        let token = provider.auth_token();

        tokio::task::spawn(async move {
            provider.run(provider::Options { addr }).await.unwrap();
        });

        async fn run_client(
            hash: bao::Hash,
            token: AuthToken,
            file_hash: bao::Hash,
            name: Option<String>,
            addr: SocketAddr,
            peer_id: PeerId,
            content: Vec<u8>,
        ) -> Result<()> {
            let opts = get::Options {
                addr,
                peer_id: Some(peer_id),
            };
            let stream = get::run(hash, token, opts);
            tokio::pin!(stream);
            while let Some(event) = stream.next().await {
                let event = event?;
                if let Event::Receiving {
                    hash: got_hash,
                    mut reader,
                    name: got_name,
                } = event
                {
                    assert_eq!(file_hash, got_hash);
                    let mut got = Vec::new();
                    reader.read_to_end(&mut got).await?;
                    assert_eq!(content, got);
                    assert_eq!(name, got_name);
                }
            }
            Ok(())
        }

        let mut tasks = Vec::new();
        for _i in 0..3 {
            tasks.push(tokio::task::spawn(run_client(
                hash,
                token,
                expect_hash,
                expect_name.clone(),
                addr,
                peer_id,
                content.to_vec(),
            )));
        }

        for task in tasks {
            task.await??;
        }

        Ok(())
    }
}
