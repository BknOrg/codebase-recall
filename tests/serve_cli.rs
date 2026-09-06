//! End-to-end test for `code-rcl serve`: spin up the server against a fixture,
//! talk to it over raw TCP, and confirm `/quit` makes the process exit.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn workdir(name: &str, tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&fixture(name), &dir);
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), dst).unwrap();
        }
    }
}

/// One HTTP/1.0 request over a fresh connection; returns `(status_line, body)`.
fn http_get(port: u16, path: &str) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.lines().next().unwrap_or_default().to_string();
    (status, body.to_string())
}

/// Kills the child on drop so a failed assertion never leaves a server running.
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn serve_starts_answers_and_quits() {
    let work = workdir("rust_app", "serve");
    let mut child = Command::new(BIN)
        .args(["serve", "--no-open", "--port", "0", "--project"])
        .arg(&work)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn code-rcl serve");

    // First stdout line is: `code-rcl graph  ->  http://127.0.0.1:<port>/`
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let port: u16 = {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut line = String::new();
        loop {
            assert!(Instant::now() < deadline, "server never printed its URL");
            line.clear();
            if out.read_line(&mut line).unwrap() == 0 {
                panic!("server exited before serving");
            }
            if let Some(rest) = line.split("http://127.0.0.1:").nth(1) {
                break rest.trim().trim_end_matches('/').parse().unwrap();
            }
        }
    };
    let mut reap = Reap(child);

    let (status, body) = http_get(port, "/");
    assert!(status.contains("200"), "GET / status: {status}");
    assert!(body.contains("code-rcl graph"), "GET / body missing header");
    assert!(
        body.contains(r#"id="graph-data""#),
        "GET / body missing data blob"
    );

    let (status, js) = http_get(port, "/assets/d3.min.js");
    assert!(
        status.contains("200"),
        "GET /assets/d3.min.js status: {status}"
    );
    assert!(js.contains("d3js.org"), "d3 asset not served");

    let (status, _) = http_get(port, "/assets/graph-view.js");
    assert!(status.contains("200"), "graph-view.js status: {status}");

    // /quit asks the server to stop; it should exit on its own shortly after.
    let _ = http_get(port, "/quit");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if reap.0.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "server did not exit after /quit");
        std::thread::sleep(Duration::from_millis(100));
    }
}
