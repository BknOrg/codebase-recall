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
    let mut token_bytes = [0u8; 32];
    getrandom::fill(&mut token_bytes).map_err(|e| anyhow!("creating session token: {e}"))?;
    let token = token_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let page = Arc::new(assets::graph_page(&data, &stat, Delivery::Server).replace(
        "</head>",
        &format!("<meta name=\"code-rcl-token\" content=\"{token}\"></head>"),
    ));
    let token = Arc::new(token);

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
        let token = token.clone();
        workers.push(thread::spawn(move || {
            worker_loop(&server, &shutdown, &live, &page, &project, &token);
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
    token: &str,
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

        let authority = server
            .server_addr()
            .to_ip()
            .expect("TCP address")
            .to_string();
        let origin = format!("http://{authority}");
        let field = |name: &'static str| {
            req.headers()
                .iter()
                .find(|h| h.field.equiv(name))
                .map(|h| h.value.as_str())
        };
        if field("Host") != Some(authority.as_str()) || field("Origin").is_some_and(|o| o != origin)
        {
            let _ = req.respond(text(403, "access denied"));
            continue;
        }
        if matches!(path.as_str(), "/dump" | "/impact" | "/quit") && *req.method() != Method::Post {
            let _ = req.respond(text(405, "POST required"));
            continue;
        }
        let api = matches!(
            path.as_str(),
            "/dump" | "/impact" | "/quit" | "/source" | "/live"
        );
        if api
            && field("X-Code-Rcl-Token") != Some(token)
            && !(path == "/live" && query_param(&url, "token").as_deref() == Some(token))
        {
            let _ = req.respond(text(403, "invalid session token"));
            continue;
        }
        match (is_get, path.as_str()) {
            (_, "/dump") => {
                let body = run_dump(project, &url);
                let _ = req.respond(with_type(
                    text(
                        if body.contains("\"ok\":true") {
                            200
                        } else {
                            if body.contains("[409]") { 409 } else { 500 }
                        },
                        &body,
                    ),
                    "application/json; charset=utf-8",
                ));
            }
            (_, "/impact") => {
                let body = run_impact(project, &url);
                let _ = req.respond(with_type(
                    text(
                        if body.contains("\"ok\":true") {
                            200
                        } else {
                            if body.contains("[409]") { 409 } else { 500 }
                        },
                        &body,
                    ),
                    "application/json; charset=utf-8",
                ));
            }
            (true, "/source") => {
                let (status, body) = run_source(project, &url);
                let _ = req.respond(with_type(
                    text(status, &body),
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
            (true, "/favicon.ico") | (true, "/favicon.svg") => {
                let _ = req.respond(with_type(
                    text(200, assets::APP_LOGO),
                    "image/svg+xml; charset=utf-8",
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
// --- /impact: caller/importer report for the viewer's right-click menu -----

/// Sanitize a query-string filename to a bare basename — no directory
/// components, no `.`/`..`, no drive letters. Shared by `/dump` and `/impact`.
fn sanitize_name(raw: Option<String>, default: &str) -> String {
    raw.map(|s| s.trim().to_string())
        .and_then(|s| {
            let base = s.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            (!base.is_empty() && base != "." && base != ".." && !base.contains(':')).then_some(base)
        })
        .unwrap_or_else(|| default.to_string())
}

/// Resolve an optional `dir` query param (a project-relative directory, as
/// sent by the viewer's right-click menu for "write next to this file") to
/// an absolute path guaranteed to stay inside `project`. Falls back to
/// `project` itself when `dir` is absent/empty — this keeps the side panel's
/// existing "dump -r" button (which never sends `dir`) writing to the
/// project root exactly as before. Same containment check as `run_source`.
fn resolve_output_dir(project: &Path, dir_param: Option<String>) -> Result<PathBuf, String> {
    let Some(dir) = dir_param.filter(|d| !d.trim().is_empty()) else {
        return Ok(project.to_path_buf());
    };
    let dir_clean = dir.replace('\\', "/");
    let dir_path = Path::new(&dir_clean);
    if dir_path.is_absolute()
        || dir_clean.starts_with('/')
        || dir_clean.split('/').any(|segment| segment == "..")
        || dir_clean.contains(':')
    {
        return Err("access denied".to_string());
    }
    Ok(project.join(dir_path))
}

/// Runs `dump -r` for `?target=…&depth=…&dir=…` and returns a small JSON
/// reply. NOTE: unlike the rest of `serve`, this writes a file to disk.
fn run_dump(project: &Path, url: &str) -> String {
    let Some(target) = query_param(url, "target").filter(|t| !t.is_empty()) else {
        return serde_json::json!({"ok": false, "error": "missing target"}).to_string();
    };
    let depth = query_param(url, "depth")
        .and_then(|d| d.parse::<u32>().ok())
        .unwrap_or(2)
        .clamp(1, 5);
    let name = sanitize_name(query_param(url, "name"), &unique_name("context", "md"));
    let dir = match resolve_output_dir(project, query_param(url, "dir")) {
        Ok(d) => d,
        Err(e) => return serde_json::json!({"ok": false, "error": e}).to_string(),
    };
    let output = dir.join(&name);

    match crate::commands::dump::generate_relation_bundle(project, &target, depth, 50, false)
        .and_then(|(body, n)| {
            write_project_file(project, &output, &body)?;
            Ok((output.clone(), n))
        }) {
        Ok((path, n)) => serde_json::json!({
            "ok": true,
            "path": path.display().to_string(),
            "files": n,
        })
        .to_string(),
        Err(e) => serde_json::json!({
            "ok": false,
            "error": e.to_string(),
        })
        .to_string(),
    }
}

/// Runs `impact` for `?target=…&depth=…&kinds=…&format=ascii|json&dir=…` and
/// writes the rendered report to disk, returning a small JSON reply.
fn run_impact(project: &Path, url: &str) -> String {
    let Some(target) = query_param(url, "target").filter(|t| !t.is_empty()) else {
        return serde_json::json!({"ok": false, "error": "missing target"}).to_string();
    };
    let depth = query_param(url, "depth")
        .and_then(|d| d.parse::<u32>().ok())
        .unwrap_or(3)
        .clamp(1, 8);
    let kinds: Vec<String> = query_param(url, "kinds")
        .filter(|k| !k.trim().is_empty())
        .map(|k| k.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_else(|| vec!["calls".to_string(), "imports".to_string()]);
    let as_json = query_param(url, "format").as_deref() == Some("json");
    let default_name = unique_name("impact", if as_json { "json" } else { "md" });
    let name = sanitize_name(query_param(url, "name"), &default_name);
    let dir = match resolve_output_dir(project, query_param(url, "dir")) {
        Ok(d) => d,
        Err(e) => return serde_json::json!({"ok": false, "error": e}).to_string(),
    };
    let output = dir.join(&name);

    let direction = query_param(url, "direction").unwrap_or_else(|| "both".to_string());

    let args = crate::cli::ImpactArgs {
        symbol: Some(target),
        diff: false,
        project: project.to_path_buf(),
        depth,
        direction,
        kinds,
        json: as_json,
        no_sync: false,
        precise: Default::default(),
    };

    let result = crate::commands::impact::generate_reports(&args).and_then(|reports| {
        let body = if as_json {
            crate::commands::impact::render_json(&reports)?
        } else {
            crate::commands::impact::render_ascii(&reports)
        };
        write_project_file(project, &output, &body)?;
        Ok(reports.len())
    });

    match result {
        Ok(n) => serde_json::json!({
            "ok": true,
            "path": output.display().to_string(),
            "targets": n,
        })
        .to_string(),
        Err(e) => serde_json::json!({
            "ok": false,
            "error": e.to_string(),
        })
        .to_string(),
    }
}

fn run_source(project: &Path, url: &str) -> (u16, String) {
    let Some(rel) = query_param(url, "path") else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing path param"}).to_string(),
        );
    };
    let rel_clean = rel.replace('\\', "/");
    let rel_path = Path::new(&rel_clean);
    if rel_path.is_absolute()
        || rel_clean.starts_with('/')
        || rel_clean.split('/').any(|segment| segment == "..")
        || rel_clean.contains(':')
    {
        return (
            403,
            serde_json::json!({"ok": false, "error": "access denied"}).to_string(),
        );
    }
    let content = cap_std::fs::Dir::open_ambient_dir(project, cap_std::ambient_authority())
        .and_then(|root| root.read_to_string(rel_path));
    match content {
        Ok(content) => (
            200,
            serde_json::json!({
                "ok": true,
                "path": rel_clean,
                "content": content,
            })
            .to_string(),
        ),
        Err(e) => (
            if e.kind() == io::ErrorKind::NotFound {
                404
            } else {
                403
            },
            serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            })
            .to_string(),
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

fn unique_name(stem: &str, extension: &str) -> String {
    let mut bytes = [0; 12];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    format!(
        "{stem}-{}.{extension}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

fn write_project_file(project: &Path, path: &Path, body: &str) -> Result<()> {
    let relative = path.strip_prefix(project)?;
    anyhow::ensure!(
        relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "access denied"
    );
    let root = cap_std::fs::Dir::open_ambient_dir(project, cap_std::ambient_authority())?;
    if let Some(parent) = relative.parent() {
        root.create_dir_all(parent)?;
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = root.open_with(relative, &options).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            anyhow!("[409] output already exists; choose another name")
        } else {
            anyhow!(e)
        }
    })?;
    file.write_all(body.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn exclusive_outputs_never_overwrite_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/result.md");
        write_project_file(root.path(), &path, "original").unwrap();
        assert!(
            write_project_file(root.path(), &path, "replacement")
                .unwrap_err()
                .to_string()
                .contains("[409]")
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
        assert!(resolve_output_dir(root.path(), Some("../outside".into())).is_err());
    }
    #[test]
    fn capability_operations_reject_directory_link_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        assert!(
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(write_project_file(root.path(), &link.join("new/output.md"), "escape").is_err());
        assert!(!outside.path().join("new").exists());
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        assert_ne!(
            run_source(root.path(), "/source?path=escape/secret.txt").0,
            200
        );
    }
}
