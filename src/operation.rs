//! Per-request diagnostics and cooperative cancellation, shared by CLI and MCP.
use std::cell::RefCell;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct State {
    memo: std::collections::HashMap<String, String>,
    cancel: Option<Arc<AtomicBool>>,
    warnings: Vec<String>,
    partial: bool,
    embedding_partial: bool,
    analysis_partial: bool,
    omitted: usize,
    reasons: Vec<serde_json::Value>,
    retrieval: serde_json::Value,
    timings: std::collections::BTreeMap<String, f64>,
    container: [usize; 4],
}
pub fn container_tokens(original: usize, actual: usize, fallback: bool) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.container[0] += 1;
        s.container[1] += original;
        s.container[2] += actual;
        s.container[3] += usize::from(fallback);
    });
}
pub fn container_stats() -> serde_json::Value {
    STATE.with(|s| {let v=s.borrow().container;serde_json::json!({"containers":v[0],"original_tokens":v[1],"representation_tokens":v[2],"fallbacks":v[3],"tokens_removed":v[1].saturating_sub(v[2])})})
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }
pub fn begin(cancel: Arc<AtomicBool>) {
    STATE.with(|s| {
        *s.borrow_mut() = State {
            cancel: Some(cancel),
            ..Default::default()
        }
    });
}
pub fn check() -> anyhow::Result<()> {
    anyhow::ensure!(!cancelled(), "operation cancelled");
    Ok(())
}
pub fn cancel_token() -> Option<Arc<AtomicBool>> {
    STATE.with(|s| s.borrow().cancel.clone())
}
pub fn cancelled() -> bool {
    STATE.with(|s| {
        s.borrow()
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    })
}
pub fn warning(message: impl Into<String>) {
    STATE.with(|s| s.borrow_mut().warnings.push(message.into()));
}
pub fn partial() {
    STATE.with(|s| s.borrow_mut().partial = true);
}
pub fn omitted(count: usize) {
    omitted_for(count, "result_limit");
}
pub fn omitted_for(count: usize, kind: &str) {
    if count > 0 {
        reason(kind, serde_json::json!({"omitted":count}));
    }
    STATE.with(|s| s.borrow_mut().omitted += count);
}

pub fn metadata() -> (Vec<String>, bool, usize) {
    STATE.with(|s| {
        let s = s.borrow();
        (
            s.warnings.clone(),
            s.partial || s.embedding_partial || s.analysis_partial,
            s.omitted,
        )
    })
}

pub fn embedding_partial() {
    reason(
        "embedding_coverage",
        serde_json::json!({"recovery":"inspect embeddings status --files"}),
    );
    STATE.with(|s| s.borrow_mut().embedding_partial = true);
}

#[cfg(test)]
pub fn graph_partial() -> bool {
    STATE.with(|s| s.borrow().partial)
}

pub fn reason(kind: &str, details: serde_json::Value) {
    STATE.with(|s| {
        let value = serde_json::json!({"kind":kind,"details":details});
        let mut state = s.borrow_mut();
        if !state.reasons.contains(&value) {
            state.reasons.push(value);
        }
    });
}
pub fn reasons() -> Vec<serde_json::Value> {
    STATE.with(|s| s.borrow().reasons.clone())
}

pub fn retrieval(value: serde_json::Value) {
    STATE.with(|s| s.borrow_mut().retrieval = value);
}
pub fn retrieval_metadata() -> serde_json::Value {
    STATE.with(|s| s.borrow().retrieval.clone())
}

pub struct Timer {
    name: &'static str,
    start: std::time::Instant,
}
pub fn time(name: &'static str) -> Timer {
    Timer {
        name,
        start: std::time::Instant::now(),
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        STATE.with(|s| {
            *s.borrow_mut().timings.entry(self.name.into()).or_default() +=
                self.start.elapsed().as_secs_f64() * 1000.
        });
    }
}
pub fn timings() -> serde_json::Value {
    STATE.with(|s| serde_json::json!(s.borrow().timings))
}

pub fn cancellation_flag() -> Option<Arc<AtomicBool>> {
    STATE.with(|s| s.borrow().cancel.clone())
}

pub fn analysis_partial() {
    STATE.with(|s| s.borrow_mut().analysis_partial = true);
}

/// Operation-local expensive file identity cache; begin() clears it between requests.
pub fn memo(key: &str) -> Option<String> {
    STATE.with(|s| s.borrow().memo.get(key).cloned())
}
pub fn memo_set(key: String, value: String) {
    STATE.with(|s| {
        s.borrow_mut().memo.insert(key, value);
    });
}
