//! A throwaway local HTTP server for `code-rcl serve`.
//!
//! It serves the graph page plus its static assets on `127.0.0.1`, then shuts
//! itself down as soon as the browser tab goes away — so it never lingers in the
//! background. "Tab is open" is tracked by a held `EventSource` (`/live`); the
//! server also stops on `Ctrl-C` or a `/quit` request.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::assets::{self, Delivery};
use crate::graph::CodeGraph;

pub struct ServeOptions {
    /// 0 = let the OS pick a free port.
    pub port: u16,
    /// Open the URL in the default browser once the server is up.
    pub open: bool,
    /// Project root — needed by the `/dump` endpoint (viewer "dump -r" button).
    pub project: PathBuf,
}

/// Worker threads pulling from the shared accept queue. A held `/live` stream
/// occupies one for the life of the tab, so keep a little headroom for reloads.
const WORKERS: usize = 4;
/// After the last `/live` client disconnects, wait this long before exiting so a
/// page reload (which briefly drops to zero connections) doesn't kill us.
const ZERO_GRACE: Duration = Duration::from_millis(2000);
/// If no browser ever connects (e.g. `--no-open` and nobody opens the URL), give
/// up after this long instead of running forever.
const STARTUP_BACKSTOP: Duration = Duration::from_secs(90);
/// Keep-alive comment cadence on the `/live` stream. Also bounds how fast we
/// notice a dropped connection (the write to a closed socket is what fails).
const KEEPALIVE: Duration = Duration::from_millis(2000);

pub fn serve(graph: &CodeGraph, opts: ServeOptions) -> Result<()> {
    let data = serde_json::to_string(graph)?;
    let stat = format!(
        "{} nodes \u{00b7} {} edges",
        graph.nodes.len(),
        graph.edges.len()
    );
    let page = Arc::new(assets::graph_page(&data, &stat, Delivery::Server));

    let server = Server::http(("127.0.0.1", opts.port))
        .map_err(|e| anyhow!("cannot bind 127.0.0.1:{}: {e}", opts.port))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(opts.port);
    let url = format!("http://127.0.0.1:{port}/");
    let server = Arc::new(server);

    let shutdown = Arc::new(AtomicBool::new(false));
    let live = Arc::new(AtomicUsize::new(0));
    let project = Arc::new(opts.project);

    // Ctrl-C -> ask everything to wind down. Workers poll `shutdown` between
    // short accept timeouts, so flipping the flag is all it takes.
    {
        let shutdown = shutdown.clone();
        let _ = ctrlc::set_handler(move || shutdown.store(true, Ordering::SeqCst));
    }

    spawn_reaper(shutdown.clone(), live.clone());

    println!("code-rcl graph  ->  {url}");
    println!(
        "  serving {} nodes / {} edges; the server exits when you close the tab (or press Ctrl-C)",
        graph.nodes.len(),
        graph.edges.len()
    );
    if opts.open && webbrowser::open(&url).is_err() {
        eprintln!("  (couldn't open a browser automatically — open the URL above)");
    }

    let mut workers = Vec::with_capacity(WORKERS);
    for _ in 0..WORKERS {
        let server = server.clone();
        let shutdown = shutdown.clone();
        let live = live.clone();
        let page = page.clone();
        let project = project.clone();
        workers.push(thread::spawn(move || {
            worker_loop(&server, &shutdown, &live, &page, &project);
        }));
    }
    for w in workers {
        let _ = w.join();
    }

    println!("code-rcl serve: stopped.");
    Ok(())
}

fn spawn_reaper(shutdown: Arc<AtomicBool>, live: Arc<AtomicUsize>) {
    thread::spawn(move || {
        let started = Instant::now();
        let mut ever_connected = false;
        let mut zero_since: Option<Instant> = None;
        loop {
            thread::sleep(Duration::from_millis(300));
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            let n = live.load(Ordering::SeqCst);
            if n > 0 {
                ever_connected = true;
                zero_since = None;
                continue;
            }
            let expired = if ever_connected {
                zero_since.get_or_insert_with(Instant::now).elapsed() > ZERO_GRACE
            } else {
                started.elapsed() > STARTUP_BACKSTOP
            };
            if expired {
                shutdown.store(true, Ordering::SeqCst);
                return;
            }
        }
    });
}

fn worker_loop(
    server: &Arc<Server>,
    shutdown: &Arc<AtomicBool>,
    live: &Arc<AtomicUsize>,
    page: &str,
    project: &Path,
) {
    while !shutdown.load(Ordering::SeqCst) {
        let req = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(r)) => r,
            Ok(None) => continue, // accept timed out — re-check `shutdown`
            Err(_) => break,      // fatal accept error
        };

        let is_get = *req.method() == Method::Get;
        let url = req.url().to_owned();
        let path = url.split('?').next().unwrap_or("/").to_owned();

        match (is_get, path.as_str()) {
            (_, "/dump") => {
                let body = run_dump(project, &url);
                let _ = req.respond(with_type(
                    text(
                        if body.contains("\"ok\":true") {
                            200
                        } else {
                            500
                        },
                        &body,
                    ),
                    "application/json; charset=utf-8",
                ));
            }
            (_, "/quit") => {
                let _ = req.respond(text(204, ""));
                shutdown.store(true, Ordering::SeqCst);
                break;
            }
            (true, "/live") => {
                // A held stream keeps a worker busy for the life of the tab, so
                // hand it to its own thread and get straight back to accepting.
                let live = live.clone();
                let shutdown = shutdown.clone();
                thread::spawn(move || live_stream(req, live, shutdown));
            }
            (true, "/") | (true, "/index.html") => {
                let _ = req.respond(with_type(text(200, page), "text/html; charset=utf-8"));
            }
            (true, "/assets/d3.min.js") => {
                let _ = req.respond(js(assets::D3_JS));
            }
            (true, "/assets/graph-view.js") => {
                let _ = req.respond(js(assets::GRAPH_VIEW_JS.as_str()));
            }
            (true, "/assets/live.js") => {
                let _ = req.respond(js(assets::LIVE_JS));
            }
            (true, "/assets/graph.css") => {
                let _ = req.respond(with_type(
                    text(200, assets::GRAPH_CSS),
                    "text/css; charset=utf-8",
                ));
            }
            _ => {
                let _ = req.respond(text(404, "not found"));
            }
        }
    }
}

/// Answer `/live` with an endless `text/event-stream`, written straight to the
/// socket so every keep-alive is flushed immediately (tiny_http's buffered
/// `Response` streaming would hold the bytes back). Runs until the client
/// disconnects — tab closed or navigated away — or the server shuts down.
///
/// The count of open `/live` streams is how the reaper knows a tab is still
/// there; an open stream is also immune to background-tab timer throttling.
fn live_stream(req: Request, live: Arc<AtomicUsize>, shutdown: Arc<AtomicBool>) {
    live.fetch_add(1, Ordering::SeqCst);
    let _guard = Decrement(&live);

    let mut sock = req.into_writer();
    let head: &[u8] = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-store\r\n\
Connection: close\r\n\
\r\n\
: connected\n\n";
    if sock.write_all(head).and_then(|_| sock.flush()).is_err() {
        return;
    }
    while !shutdown.load(Ordering::SeqCst) {
        let mut waited = Duration::ZERO;
        while waited < KEEPALIVE {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
            waited += Duration::from_millis(100);
        }
        if sock
            .write_all(b": keep-alive\n\n")
            .and_then(|_| sock.flush())
            .is_err()
        {
            return; // the browser is gone
        }
    }
}

struct Decrement<'a>(&'a Arc<AtomicUsize>);
impl Drop for Decrement<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

// --- /dump: relation-aware context bundle for the viewer's "dump -r" button ---

/// Runs `dump -r` for `?target=…&depth=…` and returns a small JSON reply.
/// NOTE: unlike the rest of `serve`, this writes `codebase-context.md` to disk.
fn run_dump(project: &Path, url: &str) -> String {
    let Some(target) = query_param(url, "target").filter(|t| !t.is_empty()) else {
        return r#"{"ok":false,"error":"missing target"}"#.to_string();
    };
    let depth = query_param(url, "depth")
        .and_then(|d| d.parse::<u32>().ok())
        .unwrap_or(2)
        .clamp(1, 5);
    // Output name: basename only, no traversal / drive-relative paths.
    let name = query_param(url, "name")
        .map(|s| s.trim().to_string())
        .and_then(|s| {
            let base = s.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            (!base.is_empty() && base != "." && base != ".." && !base.contains(':')).then_some(base)
        })
        .unwrap_or_else(|| "codebase-context.md".to_string());
    let output = project.join(&name);

    match crate::commands::dump::relation_bundle(project, &target, depth, 50, false, &output) {
        Ok((path, n)) => format!(
            r#"{{"ok":true,"path":"{}","files":{}}}"#,
            json_escape(&path.display().to_string()),
            n
        ),
        Err(e) => format!(
            r#"{{"ok":false,"error":"{}"}}"#,
            json_escape(&e.to_string())
        ),
    }
}

/// First `key=value` from a URL query string, percent-decoded (`+` -> space).
fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

// --- small response helpers ------------------------------------------------

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

fn text(status: u16, body: &str) -> Response<io::Cursor<Vec<u8>>> {
    Response::from_string(body).with_status_code(status)
}

fn with_type(mut r: Response<io::Cursor<Vec<u8>>>, ct: &str) -> Response<io::Cursor<Vec<u8>>> {
    r.add_header(header("Content-Type", ct));
    r
}

fn js(src: &str) -> Response<io::Cursor<Vec<u8>>> {
    with_type(text(200, src), "application/javascript; charset=utf-8")
}
