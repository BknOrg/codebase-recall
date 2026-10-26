//! Operation telemetry is independent of the exclusively locked cache database.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::{File, OpenOptions},
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub type Sink = Arc<dyn Fn(Value) + Send + Sync>;
struct State {
    value: Value,
    started: Instant,
    phase_started: Instant,
    completed: Instant,
}
pub struct Observer {
    project: PathBuf,
    state: Mutex<State>,
    cancel: Option<Arc<AtomicBool>>,
}
thread_local! { static CURRENT: RefCell<Option<Arc<Observer>>> = const { RefCell::new(None) }; static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) }; }
pub fn sink(sink: Option<Sink>) {
    SINK.with(|s| *s.borrow_mut() = sink);
}
pub fn update(f: impl FnOnce(&mut Value)) {
    CURRENT.with(|s| {
        if let Some(o) = s.borrow().as_ref() {
            f(&mut o.state.lock().unwrap().value);
        }
    });
}
pub fn phase(name: &str) {
    CURRENT.with(|s| {
        if let Some(o) = s.borrow().as_ref() {
            let mut s = o.state.lock().unwrap();
            let old = s.value["phase"].as_str().unwrap().to_string();
            let elapsed = s.phase_started.elapsed().as_secs_f64() * 1000.;
            let previous = s.value["timings_ms"][&old].as_f64().unwrap_or(0.);
            s.value["timings_ms"][old] = json!(previous + elapsed);
            s.phase_started = Instant::now();
            s.value["phase"] = json!(name);
            if matches!(name, "loading_model" | "verifying_model") {
                s.value["file"] = Value::Null;
            }
        }
    });
}
pub fn completed() {
    CURRENT.with(|s| {
        if let Some(o) = s.borrow().as_ref() {
            o.state.lock().unwrap().completed = Instant::now();
        }
    });
}
pub fn index(value: Value) {
    update(|s| {
        s["invalidated_chunks"] = value["invalid_chunks"].clone();
        s["files_total"] = value["total_files"].clone();
        s["last_index"] = value;
    });
}
impl Observer {
    fn snapshot(&self) -> Value {
        let s = self.state.lock().unwrap();
        let mut v = s.value.clone();
        v["elapsed_ms"] = json!(s.started.elapsed().as_millis() as u64);
        v["since_completion_ms"] = json!(s.completed.elapsed().as_millis() as u64);
        v["updated_at"] = json!(now());
        v["cancel_requested"] = json!(
            self.cancel
                .as_ref()
                .is_some_and(|v| v.load(Ordering::Relaxed))
        );
        let phase = v["phase"].as_str().unwrap().to_string();
        let previous = v["timings_ms"][&phase].as_f64().unwrap_or(0.);
        v["timings_ms"][phase] = json!(previous + s.phase_started.elapsed().as_secs_f64() * 1000.);
        v
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn lock(project: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(project.join(".code-rcl/embedding-operation.lock"))?)
}
/// Internal staging work reuses the current observer's ownership; direct service
/// callers must acquire the same OS lock before recovering interrupted staging.
pub fn operation_lock(project: &Path) -> Result<Option<File>> {
    let canonical = project.canonicalize()?;
    if CURRENT.with(|s| s.borrow().as_ref().is_some_and(|o| o.project == canonical)) {
        return Ok(None);
    }
    let file = lock(project)?;
    file.try_lock()
        .context("another embedding operation owns staging")?;
    Ok(Some(file))
}
fn save(path: &Path, value: &Value) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer(&mut temp, value)?;
    temp.flush()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn read(project: &Path) -> Result<Option<Value>> {
    let path = project.join(".code-rcl/embedding-operation.json");
    let text = match std::fs::read(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut value: Value =
        serde_json::from_slice(&text).context("invalid embedding operation snapshot")?;
    let file = lock(project)?;
    let active = match file.try_lock() {
        Ok(()) => false,
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
    };
    value["active"] = json!(active);
    if !active && value["state"] == "running" {
        value["state"] = json!("interrupted");
    }
    Ok(Some(value))
}
pub struct Guard {
    observer: Arc<Observer>,
    path: PathBuf,
    _lock: File,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    finished: bool,
    sink: Option<Sink>,
}
impl Guard {
    pub fn start(project: &Path, mode: Option<&str>) -> Result<Self> {
        std::fs::create_dir_all(project.join(".code-rcl"))?;
        let previous = read(project)?;
        let file = lock(project)?;
        file.try_lock()
            .context("another embedding operation is active; run embeddings status")?;
        let started = Instant::now();
        let observer = Arc::new(Observer {
            project: project.canonicalize()?,
            cancel: crate::operation::cancellation_flag(),
            state: Mutex::new(State {
                started,
                phase_started: started,
                completed: started,
                value: json!({
                    "id":format!("{}-{}-{:?}",std::process::id(),now(),started), "pid":std::process::id(), "started_at":now(),
                    "state":"running", "phase":"opening_cache", "file":null,"files_total":null,"files_done":0,
                    "embedded_chunks":0,"reused_chunks":0,"committed_chunks":0,"batch":null,"timings_ms":{},"last_index":null,
                    "unique_inferences":0,"deduplicated_chunks":0,"active_files":0,"pending_inputs":0,"scheduler_peak_bytes":0,"actual_tokens":0,"padded_tokens":0,"staging_bytes_written":0,
                    "recovered_interrupted":previous.as_ref().is_some_and(|v| v["state"]=="interrupted")
                }),
            }),
        });
        let sink = mode
            .map(cli_sink)
            .or_else(|| SINK.with(|s| s.borrow().clone()));
        let path = project.join(".code-rcl/embedding-operation.json");
        let initial = observer.snapshot();
        save(&path, &initial)?;
        if let Some(s) = &sink {
            s(initial);
        }
        CURRENT.with(|s| *s.borrow_mut() = Some(observer.clone()));
        let (sender, receiver) = mpsc::channel();
        let (o, p, output) = (observer.clone(), path.clone(), sink.clone());
        let worker = std::thread::spawn(move || {
            while receiver.recv_timeout(Duration::from_secs(1)).is_err() {
                let value = o.snapshot();
                if let Err(error) = save(&p, &value) {
                    eprintln!("warning: cannot update embedding progress: {error}");
                }
                if let Some(s) = &output {
                    s(value);
                }
            }
        });
        let guard = Self {
            observer,
            path,
            _lock: file,
            stop: Some(sender),
            worker: Some(worker),
            finished: false,
            sink,
        };
        crate::embeddings::recover_staging(project)?;
        Ok(guard)
    }
    pub fn finish(&mut self, state: &str) -> Result<Value> {
        if let Some(sender) = self.stop.take() {
            let _ = sender.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        phase("finalizing");
        self.observer.state.lock().unwrap().value["state"] = json!(state);
        let value = self.observer.snapshot();
        save(&self.path, &value)?;
        if let Some(s) = &self.sink {
            s(value.clone());
        }
        self.finished = true;
        CURRENT.with(|s| *s.borrow_mut() = None);
        Ok(value)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish(if crate::operation::cancelled() {
                "cancelled"
            } else {
                "failed"
            });
        }
    }
}
pub(crate) fn cli_sink(mode: &str) -> Sink {
    let off = mode == "off";
    let live = mode == "auto" && std::io::stderr().is_terminal();
    let last = Mutex::new((String::new(), Instant::now()));
    Arc::new(move |v| {
        if off {
            return;
        }
        let phase = v["phase"].as_str().unwrap_or("");
        let state = v["state"].as_str().unwrap_or("");
        let mut previous = last.lock().unwrap();
        if !live
            && phase == previous.0
            && previous.1.elapsed() < Duration::from_secs(5)
            && state == "running"
        {
            return;
        }
        *previous = (phase.into(), Instant::now());
        let total = v["files_total"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into());
        let active_paths = if phase == "inference" {
            v["batch_files"].as_array().map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        } else {
            None
        };
        let text = format!(
            "embedding {state}: {phase} | files {}/{} | new {} reused {} committed {} | {}s elapsed, {}s since completion | {} | batch {} active files {} unique {} dedup {}",
            v["files_done"],
            total,
            v["embedded_chunks"],
            v["reused_chunks"],
            v["committed_chunks"],
            v["elapsed_ms"].as_u64().unwrap_or(0) / 1000,
            v["since_completion_ms"].as_u64().unwrap_or(0) / 1000,
            active_paths
                .as_deref()
                .unwrap_or_else(|| v["file"].as_str().unwrap_or("")),
            v["batch"],
            v["active_files"],
            v["unique_inferences"],
            v["deduplicated_chunks"]
        );
        let mut stderr = std::io::stderr().lock();
        if live && state == "running" {
            let _ = write!(stderr, "\r\x1b[2K{text}");
        } else {
            if live {
                let _ = write!(stderr, "\r\x1b[2K");
            }
            let _ = writeln!(stderr, "{text}");
        }
        let _ = stderr.flush();
    })
}
pub fn install_interrupt() -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    crate::operation::begin(cancel.clone());
    ctrlc::set_handler(move || {
        if cancel.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        eprintln!(
            "cancellation requested; waiting for the current native batch/load to finish. Press Ctrl+C again to force exit."
        );
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heartbeat_lives_without_work_and_lock_detects_interrupted_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let (send, receive) = mpsc::channel();
        sink(Some(Arc::new(move |value| {
            let _ = send.send(value);
        })));
        let mut guard = Guard::start(d.path(), None).unwrap();
        assert_eq!(receive.recv().unwrap()["phase"], "opening_cache");
        phase("inference");
        let beat = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(beat["phase"], "inference");
        assert_eq!(beat["embedded_chunks"], 0);
        assert!(beat["elapsed_ms"].as_u64().unwrap() >= 900);
        assert_eq!(read(d.path()).unwrap().unwrap()["active"], true);
        assert!(Guard::start(d.path(), Some("off")).is_err());
        let finished = guard.finish("cancelled").unwrap();
        assert_eq!(finished["state"], "cancelled");
        drop(guard);
        let mut stale = finished;
        stale["state"] = json!("running");
        save(&d.path().join(".code-rcl/embedding-operation.json"), &stale).unwrap();
        assert_eq!(read(d.path()).unwrap().unwrap()["state"], "interrupted");
        sink(None);
    }
}
