#![cfg(any(target_os = "windows", target_os = "macos"))]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::Result;
use assert_cmd::prelude::*;
use tempfile::tempdir;

const KEY_PATH: &str = "key";
const TOKEN: &str = "uyfZLJHxXhyrL3T2FG7waiAh214H0fETxVqzAdYHGX0";
const PEER_ID: &str = "oK2O4t8twxqe3mUiv_aRds2ZDS-ln03b-oU2KvI8qpU";
const FOLDER_HASH: &str = "bafkr4idyzqc7g2wggwyo6dos7z3qwi2keus46kmk3ljh2hg5ezpnd7jnqy";
const FILE_HASH: &str = "bafkr4ict7dy3iohmc4xpupfxnoogwcfgily7vukhxuwooje6ph7h775wtq";

#[tokio::test]
async fn cli_transfer_one_file() -> Result<()> {
    let dir = tempdir()?;
    let out = dir.path().join("out");

    let res = CliTestRunner::new()
        .path(PathBuf::from("transfer").join("foo.bin"))
        .port(43333)
        .out(&out)
        .hash(FILE_HASH)
        .run()
        .await?;

    // run test w/ `UPDATE_EXPECT=1` to update snapshot files
    let expect = expect_test::expect_file!("./snapshots/cli__transfer_one_file__provide.snap");
    expect.assert_eq(&res.provider_stderr);

    let expect = expect_test::expect_file!("./snapshots/cli__transfer_one_file__get.snap");
    expect.assert_eq(&res.getter_stderr);
    compare_files(res.input_path.unwrap(), out)?;
    Ok(())
}

#[tokio::test]
async fn cli_transfer_folder() -> Result<()> {
    let dir = tempdir()?;
    let out = dir.path().join("out");

    let res = CliTestRunner::new()
        .port(43334)
        .path(PathBuf::from("transfer"))
        .out(&out)
        .hash(FOLDER_HASH)
        .run()
        .await?;

    // run test w/ `UPDATE_EXPECT=1` to update snapshot files
    let expect = expect_test::expect_file!("./snapshots/cli__transfer_folder__provide.snap");
    expect.assert_eq(&res.provider_stderr);

    let expect = expect_test::expect_file!("./snapshots/cli__transfer_folder__get.snap");
    expect.assert_eq(&res.getter_stderr);
    compare_files(res.input_path.unwrap(), out)
}

#[tokio::test]
async fn cli_transfer_from_stdin() -> Result<()> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let path = src.join("transfer").join("foo.bin");
    let f = std::fs::File::open(&path)?;
    let stdin = Stdio::from(f);
    let mut cmd = Command::cargo_bin("sendme")?;
    cmd.stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .stdin(stdin)
        .arg("provide")
        .arg("--key")
        .arg(src.join(KEY_PATH))
        .arg("--auth-token")
        .arg(TOKEN)
        .arg("--addr")
        .arg("127.0.0.1:43335");

    // b/c we are using stdin, the hash of the file will change every time, since we currently save
    // the content of stdin to a tempfile
    // since there is no way to neatly extract the collection hash, let's just test that we are
    // able to get the content from stdin without error

    // run test w/ `UPDATE_EXPECT=1` to update snapshot files

    let mut stderr = {
        let mut provide_process = ProvideProcess {
            child: cmd.spawn()?,
        };

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        provide_process.child.stderr.take().unwrap()
    };

    let mut got = String::new();
    stderr.read_to_string(&mut got)?;
    let got = redact_collection_and_ticket(&mut got)?;
    let expect = expect_test::expect_file!("./snapshots/cli__transfer_from_stdin__provide.snap");
    expect.assert_eq(&got);
    Ok(())
}

#[tokio::test]
async fn cli_transfer_to_stdout() -> Result<()> {
    let res = CliTestRunner::new()
        .port(43336)
        .path(PathBuf::from("transfer").join("foo.bin"))
        .hash(FILE_HASH)
        .run()
        .await?;

    // run test w/ `UPDATE_EXPECT=1` to update snapshot files
    let expect = expect_test::expect_file!("./snapshots/cli__transfer_to_stdout__provide.snap");
    expect.assert_eq(&res.provider_stderr);

    let expect = expect_test::expect_file!("./snapshots/cli__transfer_to_stdout__get.snap");
    expect.assert_eq(&res.getter_stderr);

    let expect_content = tokio::fs::read(res.input_path.unwrap()).await?;
    assert_eq!(expect_content, res.getter_stdout);
    Ok(())
}

struct ProvideProcess {
    child: Child,
}

impl Drop for ProvideProcess {
    fn drop(&mut self) {
        self.child.kill().unwrap();
    }
}

fn redact_provide_path(path: &Path, s: String) -> String {
    let path = path.to_string_lossy();
    s.replace(&*path, "[PATH]")
}

fn redact_get_time(s: &mut str) -> Result<String> {
    let re = regex::Regex::new(r"Done in \d\s\w*")?;
    let s = re.replace(s, "Done in [TIME]");
    Ok(s.to_string())
}

fn redact_collection_and_ticket(s: &mut str) -> Result<String> {
    let re = regex::Regex::new(r"Collection: \S*")?;
    let s = re.replace(s, "Collection: [HASH]").to_string();
    let re = regex::Regex::new(r"All-in-one ticket: \S*")?;
    Ok(re.replace(&s, "All-in-one ticket: [TICKET]").to_string())
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
        let expect = std::fs::read(expect_path)?;
        let got = std::fs::read(got_dir_path.join(file_name))?;
        assert_eq!(expect, got);
    }

    Ok(())
}

struct CliTestRunner {
    port: u16,
    path: PathBuf,
    out: Option<PathBuf>,
    hash: Option<String>,
}

struct CliTestResults {
    // expected terminal output from the provider
    provider_stderr: String,
    // potential terminal output from the provider
    provider_stdout: String,
    // expected terminal output from the getter
    getter_stderr: String,
    // only used when we don't specify an `--out` folder, the content of the transfered file gets
    // pushed to the getter's stdout
    getter_stdout: Vec<u8>,
    // the content path given to the provider
    input_path: Option<PathBuf>,
}

impl CliTestResults {
    fn empty() -> Self {
        Self {
            provider_stdout: "".to_string(),
            provider_stderr: "".to_string(),
            getter_stdout: vec![],
            getter_stderr: "".to_string(),
            input_path: None,
        }
    }
}

impl CliTestRunner {
    fn new() -> Self {
        Self {
            port: 40000_u16,
            path: "transfer".parse().unwrap(),
            out: None,
            hash: None,
        }
    }

    fn port(mut self, p: u16) -> Self {
        self.port = p;
        self
    }

    fn path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = path.as_ref().to_path_buf();
        self
    }

    fn out(mut self, out: impl AsRef<Path>) -> Self {
        self.out = Some(out.as_ref().to_path_buf());
        self
    }

    fn hash<I: Into<String>>(mut self, hash: I) -> Self {
        self.hash = Some(hash.into());
        self
    }

    async fn run(self) -> Result<CliTestResults> {
        let hash = self
            .hash
            .expect("Must provider a collection hash to the test runner");

        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");

        let path = src.join(&self.path);

        let addr = format!("127.0.0.1:{}", self.port);

        let mut cmd = Command::cargo_bin("sendme")?;
        cmd.stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .arg("provide")
            .arg(&path)
            .arg("--key")
            .arg(src.join(KEY_PATH))
            .arg("--auth-token")
            .arg(TOKEN)
            .arg("--addr")
            .arg(&addr);

        let (get_output, mut stderr, mut stdout) = {
            // to ensure we drop the child process, do provide work in its own
            // closure
            // if we don't drop the child process the provider's stderr & stdout readers will never
            // close, and will never EOF
            let mut provide_process = ProvideProcess {
                child: cmd.spawn()?,
            };

            let mut cmd = Command::cargo_bin("sendme")?;
            cmd.arg("get")
                .arg(hash)
                .arg("--peer")
                .arg(PEER_ID)
                .arg("--auth-token")
                .arg(TOKEN)
                .arg("--addr")
                .arg(addr);
            let cmd = if let Some(out) = self.out {
                cmd.arg("--out").arg(out)
            } else {
                &mut cmd
            };

            let get_output = cmd.output()?;

            let stderr = provide_process.child.stderr.take().unwrap();
            let stdout = provide_process.child.stdout.take().unwrap();
            (get_output, stderr, stdout)
        };

        let mut res = CliTestResults::empty();
        stderr.read_to_string(&mut res.provider_stderr)?;

        // this is useful if you have to change the underlying transfer files & need to know the
        // new hash you should be expecting
        // run the test with `cargo test TEST_NAME -- --nocapture` to see this
        println!("{}", res.provider_stderr);
        println!("{}", res.provider_stdout);

        stdout.read_to_string(&mut res.provider_stdout)?;

        res.getter_stderr = String::from_utf8_lossy(&get_output.stderr).to_string();
        res.getter_stdout = get_output.stdout;

        // redactions
        res.provider_stderr = redact_provide_path(&path, res.provider_stderr);
        res.getter_stderr = redact_get_time(&mut res.getter_stderr)?;

        res.input_path = Some(path);
        Ok(res)
    }
}
