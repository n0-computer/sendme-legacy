use anyhow::Result;
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
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
    let collection_hash = "bafkr4ie23o4uzjpjs7siacb2cjiqnnugrpdqa3dnawbesmqlddfc4buw4i";

    // use random
    let addr = "127.0.0.1:43333";

    let dir = testdir!();
    let key_path = dir.join("key");
    tokio::fs::write(&key_path, key).await?;

    let path = dir.join("hello_world");
    tokio::fs::write(&path, "hello world!").await?;
    let mut prov_cmd = Command::cargo_bin("sendme")?;

    prov_cmd
        .arg("provide")
        .arg(path)
        .arg("--key")
        .arg(key_path)
        .arg("--token")
        .arg(auth_token)
        .arg("--addr")
        .arg(addr);

    let mut provide_process = prov_cmd.spawn()?;

    // TRY: using same cmd
    let out_dir = dir.join("out");
    let mut get_cmd = Command::cargo_bin("sendme")?;
    get_cmd
        .arg("get")
        .arg(collection_hash)
        .arg("--peer")
        .arg(peer_id)
        .arg("--token")
        .arg(auth_token)
        .arg("--addr")
        .arg(addr)
        .arg("--out")
        .arg(out_dir);

    get_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("woooo"));

    // test output for get side
    // test output for provide side
    // test files are the same
    // test file is saved in the correct spot
    provide_process.kill()?;
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
