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
        let res = single_file_in_collection(filename, "hello world!".as_bytes()).await;
        println!("res {:#?}", res);
        res
    }

    async fn single_file_in_collection(filename: &str, data: &[u8]) -> Result<()> {
        let dir: PathBuf = testdir!();
        let path = dir.join(filename);
        tokio::fs::write(&path, data).await?;

        // hash of the transfer file
        let data = tokio::fs::read(&path).await?;
        let (_, expect_hash) = bao::encode::outboard(&data);

        let (db, blobs_db) =
            provider::create_db(vec![provider::DataSource::File(path.clone())]).await?;
        let _collection_hash = *blobs_db.iter().next().unwrap().0;
        let mut provider = provider::Provider::builder()
            .database(db)
            .blobs_database(blobs_db)
            .build()?;
        let peer_id = provider.peer_id();
        let token = provider.auth_token();
        let addr = "127.0.0.1:4443".parse().unwrap();

        let provider_task = tokio::task::spawn(async move {
            provider.run(provider::Options { addr }).await.unwrap();
        });

        let opts = get::Options {
            addr,
            peer_id: Some(peer_id),
        };
        // let stream = client::run(collection_hash, token, opts);
        let stream = get::run(expect_hash, token, opts);
        tokio::pin!(stream);

        let mut receiving_event = None;

        // needs to iterate until `done`
        while let Some(event) = stream.next().await {
            println!("EVENT: {:#?}", event);
            let event = event?;
            if let Event::Receiving { .. } = event {
                receiving_event = Some(event);
            }
        }

        if let Some(Event::Receiving {
            hash: got_hash,
            mut reader,
            name,
        }) = receiving_event
        {
            assert_eq!(expect_hash, got_hash);
            let expect = tokio::fs::read(&path).await?;
            let mut got = Vec::new();
            reader.read_to_end(&mut got).await?;
            // println!("got len: {}", got.len())
            assert_eq!(expect.len(), got.len());
            assert_eq!(name, Some(filename.to_string()));
        }

        provider_task.abort();
        let _ = provider_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn sizes() -> Result<()> {
        let sizes = [
            // 10,
            // 100,
            // 1024,
            1024 * 100,
            1024 * 500,
            1024 * 1024,
            1024 * 1024 + 10,
        ];

        for size in sizes {
            println!("testing {size} bytes");
            let mut content = vec![0u8; size];
            rand::thread_rng().fill_bytes(&mut content);
            single_file_in_collection("hello_world", &content).await?;
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_clients() -> Result<()> {
        let dir: PathBuf = testdir!();
        let path = dir.join("hello_world");
        let content = b"hello world!";
        let addr = "127.0.0.1:4444".parse().unwrap();

        tokio::fs::write(&path, content).await?;
        let (db, blobs_db) = provider::create_db(vec![provider::DataSource::File(path)]).await?;
        let hash = *blobs_db.iter().next().unwrap().0;
        let mut provider = provider::Provider::builder()
            .database(db)
            .blobs_database(blobs_db)
            .build()?;
        let peer_id = provider.peer_id();
        let token = provider.auth_token();

        tokio::task::spawn(async move {
            provider.run(provider::Options { addr }).await.unwrap();
        });

        async fn run_client(
            hash: bao::Hash,
            token: AuthToken,
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
                    hash: new_hash,
                    mut reader,
                    name,
                } = event
                {
                    assert_eq!(hash, new_hash);
                    let mut got = Vec::new();
                    reader.read_to_end(&mut got).await?;
                    assert_eq!(content, got);
                    assert_eq!(name, Some("hello world".to_string()));
                }
            }
            Ok(())
        }

        let mut tasks = Vec::new();
        for _i in 0..3 {
            tasks.push(tokio::task::spawn(run_client(
                hash,
                token,
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
