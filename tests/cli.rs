use std::io::Read;
use std::process::{Command, Stdio};

use anyhow::Result;
use assert_cmd::prelude::*;
use predicates::prelude::*;
use testdir::testdir;

#[test]
fn file_doesnt_exist() -> Result<()> {
    let mut cmd = Command::cargo_bin("sendme")?;
    cmd.arg("provide").arg("/doesnt/exist");
    cmd.assert().failure().stderr(predicate::str::contains(
        "path must be either a Directory or a File",
    ));

    Ok(())
}

#[tokio::test]
async fn transfer_one_file() -> Result<()> {
    // set up input test dir w/ file
    let key = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCgrY7i3y3DGp7eZSK/9pF2zZkNL6WfTdv6hTYq8jyqlQAAAIi2+3n+tvt5
/gAAAAtzc2gtZWQyNTUxOQAAACCgrY7i3y3DGp7eZSK/9pF2zZkNL6WfTdv6hTYq8jyqlQ
AAAEAVjwFviyQDI6YO+L+8yn0WpBX1X5T8KzFs3XhbbqBYLqCtjuLfLcMant5lIr/2kXbN
mQ0vpZ9N2/qFNiryPKqVAAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----";

    let peer_id = "oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU";
    let auth_token = "uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0";
    let collection_hash = "bafkr4ihztdqpllivflhsxdg77kzsbw757znfee3ybdncqvxqwdi2jpbtsy";

    // use random
    let addr = "127.0.0.1:43333";

    let dir = testdir!();
    let key_path = dir.join("key");
    tokio::fs::write(&key_path, key).await?;

    let path = dir.join("hello_world");
    tokio::fs::write(&path, "hello world!").await?;
    let mut cmd = Command::cargo_bin("sendme")?;

    cmd.stderr(Stdio::piped())
        .arg("provide")
        .arg(path)
        .arg("--key")
        .arg(key_path)
        .arg("--auth-token")
        .arg(auth_token)
        .arg("--addr")
        .arg(addr);

    let mut provide_process = cmd.spawn()?;

    // TRY: using same cmd
    let out_dir = dir.join("out");
    let mut cmd = Command::cargo_bin("sendme")?;
    cmd.arg("get")
        .arg(collection_hash)
        .arg("--peer")
        .arg(peer_id)
        .arg("--auth-token")
        .arg(auth_token)
        .arg("--addr")
        .arg(addr)
        .arg("--out")
        .arg(out_dir);

    cmd.assert().success().stderr(predicate::str::contains(
        "Fetching: bafkr4ihztdqpllivflhsxdg77kzsbw757znfee3ybdncqvxqwdi2jpbtsy
[1/3] Connecting ...
[2/3] Requesting ...
[3/3] Downloading collection...
  1 file(s) with total transfer size 12B
Done in 0 seconds",
    ));

    let mut output_reader = provide_process.stderr.take().unwrap();
    provide_process.kill()?;
    let mut output = String::new();
    output_reader.read_to_string(&mut output)?;
    assert!(predicates::str::starts_with("Reading").eval(&output));
    assert!(predicates::str::ends_with("PeerID: oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU
Auth token: uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0
All-in-one ticket: IPmY4PWtFSrPK4zf-rMg2_3-WlITeAjaKFbwsNGkvDOWIKCtjuLfLcMant5lIr_2kXbNmQ0vpZ9N2_qFNiryPKqVAH8AAAHF0gK7J9kskfFeHKsvdPYUbvBqICHbXgfR8RPFWrMB1gcZfQ
").eval(&output));
    Ok(())
}

fn transfer_folder() -> Result<()> {
    todo!();
}

fn transfer_from_stdin() -> Result<()> {
    todo!();
}

fn transfer_to_stdout() -> Result<()> {
    todo!();
}
