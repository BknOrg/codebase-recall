//! A small synchronous LSP client speaking JSON-RPC over a child process'
//! stdio. Enough of the protocol to open documents and ask for definitions.
//!
//! Everything the rest of code-rcl does is synchronous, so rather than pull in
//! an async runtime this reads the server on a background thread and hands
//! messages to the caller through a channel — which is also what makes every
//! wait here bounded by a timeout instead of blocking forever on a server that
//! died or wedged.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::backend::Launcher;
use super::map::path_to_uri;

/// How many lines of the server's stderr to keep for error messages.
const STDERR_TAIL_LINES: usize = 40;

/// Why a readiness wait ended. Both outcomes are usable; the caller decides
/// whether to warn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The server reported that it finished indexing.
    Ready,
    /// It was still working when the budget ran out. Answers may be incomplete.
    TimedOut,
}

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    inbox: Receiver<Value>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    next_id: i64,
    /// `$/progress` tokens the server has begun but not ended.
    outstanding_progress: HashSet<String>,
    /// Set when the server reports it has gone quiet (rust-analyzer).
    quiescent: bool,
    name: String,
    root: String,
}

impl LspClient {
    /// Spawn the server and wire up its pipes.
    pub fn start(launcher: &Launcher, root: &Path) -> Result<Self> {
        let mut command = launcher.command();
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().with_context(|| {
            format!(
                "starting language server `{}`.\n  \
                 It was found at {}, but could not be executed. \
                 Check that it runs from a terminal and that any runtime it needs \
                 (Node.js for pyright, a JDK for jdtls/kotlin-language-server) is installed.",
                launcher.name,
                launcher.program.display()
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("language server `{}` gave no stdin", launcher.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("language server `{}` gave no stdout", launcher.name))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("language server `{}` gave no stderr", launcher.name))?;

        let (tx, inbox) = channel();
        thread::spawn(move || read_messages(stdout, tx));

        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        // Drained on its own thread: a server that fills its stderr pipe while
        // nobody reads it will block and look like a hang.
        thread::spawn({
            let tail = Arc::clone(&stderr_tail);
            move || drain_stderr(stderr, tail)
        });

        Ok(Self {
            child,
            stdin,
            inbox,
            stderr_tail,
            next_id: 1,
            outstanding_progress: HashSet::new(),
            quiescent: false,
            name: launcher.name.clone(),
            root: path_to_uri(root),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tail of the server's own log output, for error messages.
    pub fn stderr_tail(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|t| t.join("\n"))
            .unwrap_or_default()
    }

    /// `initialize` + `initialized`. Must be the first thing sent.
    pub fn initialize(&mut self, root: &Path, timeout: Duration) -> Result<()> {
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "code-rcl", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": self.root,
            "rootPath": root.to_string_lossy(),
            "workspaceFolders": [{
                "uri": self.root,
                "name": root.file_name().map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "workspace".to_string()),
            }],
            "capabilities": {
                "workspace": {
                    "workspaceFolders": true,
                    "configuration": true,
                    "didChangeConfiguration": { "dynamicRegistration": false },
                },
                "textDocument": {
                    "synchronization": { "dynamicRegistration": false, "didSave": false },
                    // linkSupport gets us `targetSelectionRange`: the definition's
                    // name range rather than its whole body.
                    "definition": { "dynamicRegistration": false, "linkSupport": true },
                },
                "window": { "workDoneProgress": true },
                "general": { "positionEncodings": ["utf-16"] },
                // rust-analyzer answers this with `quiescent: true` once it has
                // finished indexing — an exact "ready" signal.
                "experimental": { "serverStatusNotification": true },
            },
        });

        self.request("initialize", params, timeout)
            .with_context(|| format!("handshaking with language server `{}`", self.name))?;
        self.notify("initialized", json!({}))?;
        Ok(())
    }

    /// Wait for the server to finish its initial indexing.
    ///
    /// Querying before that returns empty answers, which would silently look
    /// like "nothing resolves" — so this is a correctness step, not a nicety.
    pub fn wait_until_ready(&mut self, max: Duration) -> Readiness {
        let start = Instant::now();
        // Servers that never report progress at all shouldn't be waited on for
        // the full budget, so give progress a short window to show up.
        let grace = Duration::from_secs(3);
        let settle = Duration::from_millis(400);
        let mut saw_progress = false;
        let mut idle_since: Option<Instant> = None;

        while start.elapsed() < max {
            if self.quiescent {
                return Readiness::Ready;
            }
            if !self.outstanding_progress.is_empty() {
                saw_progress = true;
                idle_since = None;
            } else if saw_progress {
                match idle_since {
                    Some(t) if t.elapsed() >= settle => return Readiness::Ready,
                    Some(_) => {}
                    None => idle_since = Some(Instant::now()),
                }
            } else if start.elapsed() >= grace {
                return Readiness::Ready;
            }
            // Drives the message loop; anything but a response is handled inline.
            let _ = self.pump(None, Instant::now() + Duration::from_millis(200));
        }
        Readiness::TimedOut
    }

    pub fn did_open(&mut self, uri: &str, language_id: &str, text: &str) -> Result<()> {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
    }

    pub fn did_close(&mut self, uri: &str) -> Result<()> {
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    /// `textDocument/definition` at a zero-based UTF-16 position.
    pub fn definition(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
        timeout: Duration,
    ) -> Result<Value> {
        self.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
            timeout,
        )
    }

    /// Ask the server to stop, then make sure it actually did.
    pub fn shutdown(mut self) {
        let _ = self.request("shutdown", Value::Null, Duration::from_secs(5));
        let _ = self.notify("exit", Value::Null);

        // Give it a moment to leave on its own before forcing the issue, so a
        // server flushing caches to disk isn't cut off mid-write.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    // ----- protocol plumbing ------------------------------------------------

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let deadline = Instant::now() + timeout;
        match self.pump(Some(id), deadline)? {
            Some(result) => Ok(result),
            None => Err(anyhow!(
                "language server `{}` did not answer `{method}` within {}s.\n  \
                 The project may be larger than the timeout allows — raise it with \
                 `--precise-timeout <seconds>`.{}",
                self.name,
                timeout.as_secs(),
                self.stderr_hint(),
            )),
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, message: Value) -> Result<()> {
        let body = serde_json::to_vec(&message)?;
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .and_then(|_| self.stdin.write_all(&body))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| {
                anyhow!(
                    "lost the connection to language server `{}` while sending a request ({e}).\n  \
                     It most likely exited early.{}",
                    self.name,
                    self.stderr_hint(),
                )
            })
    }

    /// Read messages until the response to `want` arrives or `deadline` passes.
    ///
    /// Server-to-client requests are answered as they come in: a server that is
    /// left waiting on one of those (jdtls asks for configuration during
    /// startup) simply stops making progress.
    fn pump(&mut self, want: Option<i64>, deadline: Instant) -> Result<Option<Value>> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let message = match self.inbox.recv_timeout(remaining) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    bail!(
                        "language server `{}` exited while code-rcl was talking to it.{}",
                        self.name,
                        self.stderr_hint(),
                    );
                }
            };

            let method = message.get("method").and_then(Value::as_str);
            let id = message.get("id");

            match (method, id) {
                // A request from the server.
                (Some(method), Some(id)) => {
                    let reply = self.server_request_result(method, &message);
                    self.send(json!({ "jsonrpc": "2.0", "id": id, "result": reply }))?;
                }
                // A notification.
                (Some(method), None) => {
                    self.handle_notification(method, message.get("params"));
                }
                // A response to one of ours.
                (None, Some(id)) => {
                    if id.as_i64() != want {
                        continue; // a stale answer we no longer care about
                    }
                    if let Some(error) = message.get("error") {
                        let text = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error");
                        bail!("language server `{}` returned an error: {text}", self.name);
                    }
                    return Ok(Some(message.get("result").cloned().unwrap_or(Value::Null)));
                }
                (None, None) => {}
            }
        }
    }

    /// What to answer a server-initiated request with.
    fn server_request_result(&self, method: &str, message: &Value) -> Value {
        match method {
            // One settings object per requested item, or the server may wait
            // forever for a well-formed answer. Empty means "use your defaults".
            "workspace/configuration" => {
                let n = message
                    .get("params")
                    .and_then(|p| p.get("items"))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Value::Array(vec![json!({}); n])
            }
            "workspace/workspaceFolders" => json!([{ "uri": self.root, "name": "workspace" }]),
            _ => Value::Null,
        }
    }

    fn handle_notification(&mut self, method: &str, params: Option<&Value>) {
        match method {
            "$/progress" => {
                let Some(params) = params else { return };
                let Some(token) = params.get("token") else {
                    return;
                };
                let token = token.to_string();
                match params
                    .get("value")
                    .and_then(|v| v.get("kind"))
                    .and_then(Value::as_str)
                {
                    Some("begin") => {
                        self.outstanding_progress.insert(token);
                    }
                    Some("end") => {
                        self.outstanding_progress.remove(&token);
                    }
                    _ => {}
                }
            }
            "experimental/serverStatus" => {
                if let Some(true) = params
                    .and_then(|p| p.get("quiescent"))
                    .and_then(Value::as_bool)
                {
                    self.quiescent = true;
                }
            }
            _ => {}
        }
    }

    /// The server's own last words, appended to an error when it has any.
    fn stderr_hint(&self) -> String {
        let tail = self.stderr_tail();
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!("\n  Last output from the server:\n{}", indent(&tail))
        }
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read `Content-Length`-framed JSON messages until the pipe closes.
fn read_messages(stdout: ChildStdout, tx: Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return, // server closed its stdout
                Ok(_) => {}
            }
            let line = line.trim_end();
            if line.is_empty() {
                break; // blank line ends the header block
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse().ok();
            }
        }

        // Without a length there is no way to find the next message boundary.
        let Some(length) = length else { return };
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        match serde_json::from_slice(&body) {
            // A message we cannot parse is skipped rather than fatal: the frame
            // boundary is still known, so the stream stays usable.
            Err(_) => continue,
            Ok(value) => {
                if tx.send(value).is_err() {
                    return; // client gone
                }
            }
        }
    }
}

fn drain_stderr(stderr: ChildStderr, tail: Arc<Mutex<Vec<String>>>) {
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(std::result::Result::ok) {
        if let Ok(mut tail) = tail.lock() {
            if tail.len() == STDERR_TAIL_LINES {
                tail.remove(0);
            }
            tail.push(line);
        }
    }
}
