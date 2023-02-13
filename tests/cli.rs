use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::Result;
use assert_cmd::prelude::*;
use predicates::prelude::*;
use testdir::testdir;

const KEY_PATH: &str = "key";
const TOKEN: &str = "uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0";
const PEER_ID: &str = "oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU";

#[tokio::test]
async fn transfer_one_file() -> Result<()> {
    let dir = testdir!();
    let out = dir.join("out");

    let opts = TransferOptions {
    addr: "127.0.0.1:43333",
    path: PathBuf::from("transfer/hello_world"),
    key: KEY_PATH,
    token: TOKEN,
    peer_id: PEER_ID,
    hash: "bafkr4ic7nvgyutah2cpnavkwittawseizlln4r7xjciturflycwl3hmzx4",
    out: &out,
    expected_get_stderr: "bafkr4ic7nvgyutah2cpnavkwittawseizlln4r7xjciturflycwl3hmzx4
[1/3] Connecting ...
[2/3] Requesting ...
[3/3] Downloading collection...
  1 file(s) with total transfer size 13B
Done in 0 seconds",
expected_provide_stderr: "Reading [PATH]
Collection: bafkr4ic7nvgyutah2cpnavkwittawseizlln4r7xjciturflycwl3hmzx4

PeerID: oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU
Auth token: uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0
All-in-one ticket: IF9tTYpMB9Ce0FVWROYLSIjK1t5H90iROkSrwKy9nZm_IKCtjuLfLcMant5lIr_2kXbNmQ0vpZ9N2_qFNiryPKqVAH8AAAHF0gK7J9kskfFeHKsvdPYUbvBqICHbXgfR8RPFWrMB1gcZfQ
",
    };

    transfer_cmd(opts).await
    // TODO: test output file is == to input file
}

#[tokio::test]
async fn transfer_folder() -> Result<()> {
    let dir = testdir!();
    let out = dir.join("out");

    let opts = TransferOptions {
    addr: "127.0.0.1:43334",
    path: "transfer".parse()?, 
    key: KEY_PATH,
    token: TOKEN,
    peer_id: PEER_ID,
    hash: "bafkr4iahpa5b75ondci6tkri7ny4pxrfdmqaeycg5uu5kelizoekjn3or4",
    out: &out,
    expected_get_stderr: "bafkr4iahpa5b75ondci6tkri7ny4pxrfdmqaeycg5uu5kelizoekjn3or4
[1/3] Connecting ...
[2/3] Requesting ...
[3/3] Downloading collection...
  2 file(s) with total transfer size 25B
Done in 0 seconds",
expected_provide_stderr: "Reading [PATH]
Collection: bafkr4iahpa5b75ondci6tkri7ny4pxrfdmqaeycg5uu5kelizoekjn3or4

PeerID: oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU
Auth token: uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0
All-in-one ticket: IAd4Oh_1zRiR6aoo-3HH3iUbIAJgRu0p1RFoy4ikt26PIKCtjuLfLcMant5lIr_2kXbNmQ0vpZ9N2_qFNiryPKqVAH8AAAHG0gK7J9kskfFeHKsvdPYUbvBqICHbXgfR8RPFWrMB1gcZfQ
",
    };

    transfer_cmd(opts).await
    // TODO: test output files have correct content
}

#[test]
#[ignore]
fn transfer_from_stdin() -> Result<()> {
    todo!();
}

#[test]
#[ignore]
fn transfer_to_stdout() -> Result<()> {
    todo!();
}

struct TransferOptions<'a> {
    // TODO: figure out a method for knowing the randomly assigned port
    // Maybe output the addr to the provider's stderr & parsing the output to get the address?
    addr: &'a str,
    // `path` is appended to `sendme/tests/fixtures`
    path: PathBuf,
    key: &'a str,
    token: &'a str,
    peer_id: &'a str,
    hash: &'a str,
    out: &'a Path,
    expected_get_stderr: &'a str,
    expected_provide_stderr: &'a str,
}

struct ProvideProcess {
    child: Child,
}

impl Drop for ProvideProcess {
    fn drop(&mut self) {
        self.child.kill().unwrap();
    }
}

async fn transfer_cmd(opts: TransferOptions<'_>) -> Result<()> {
    let mut cmd = Command::cargo_bin("sendme")?;

    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let path = src.join(opts.path);

    cmd.stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .arg("provide")
        .arg(&path)
        .arg("--key")
        .arg(src.join(opts.key))
        .arg("--auth-token")
        .arg(opts.token)
        .arg("--addr")
        .arg(opts.addr);

    let (get_assert, mut output_reader) = {
        let mut provide_process = ProvideProcess {
            child: cmd.spawn()?,
        };

        let mut cmd = Command::cargo_bin("sendme")?;
        cmd.arg("get")
            .arg(opts.hash)
            .arg("--peer")
            .arg(opts.peer_id)
            .arg("--auth-token")
            .arg(opts.token)
            .arg("--addr")
            .arg(opts.addr)
            .arg("--out")
            .arg(opts.out);

        let get_assert = cmd.assert();

        // the the output before we drop the `provide_process`
        let output_reader = provide_process.child.stderr.take().unwrap();
        (get_assert, output_reader)
    };

    let mut output = String::new();
    output_reader.read_to_string(&mut output)?;

    // this is convoluted, but I can't use the same `assert` pattern that I did in the `get`
    // command, since we need the `provider` to be a longer running process
    // I can use a predicate to see if the output "ends_with" the `opts.expected_provide_stderr`
    // but then we don't get nice output to display what is different between the two strs
    let redact_path = tokio::fs::canonicalize(&path).await?;
    let output = output.replace(redact_path.to_str().unwrap(), "[PATH]");
    assert_eq!(output, opts.expected_provide_stderr);
    get_assert
        .success()
        .stderr(predicate::str::contains(opts.expected_get_stderr));
    compare_files(path, opts.out)?;
    Ok(())
}

fn compare_files(expect_path: impl AsRef<Path>, got_dir_path: impl AsRef<Path>) -> Result<()> {
    // if dir, get filename,  come up with paths for expect and got
    // call compare_files() on each
    // if file, open files & assert_eq
    let expect_path = expect_path.as_ref();
    let got_dir_path = got_dir_path.as_ref();
    if expect_path.is_dir() {
        let paths = std::fs::read_dir(expect_path)?;
        for entry in paths {
            let entry = entry?;
            compare_files(entry.path(), got_dir_path)?;
        }
    } else {
        let file_name = expect_path.file_name().unwrap();
        let expect = std::fs::read_to_string(expect_path)?;
        let got = std::fs::read_to_string(got_dir_path.join(file_name))?;
        assert_eq!(expect, got);
    }

    Ok(())
}
