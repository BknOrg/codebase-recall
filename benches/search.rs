//! Opt-in to keep correctness CI independent of machine performance.
#[allow(dead_code, unused_imports)]
#[path = "../src/analysis/mod.rs"]
mod analysis;
#[allow(dead_code, unused_imports)]
#[path = "../src/cache/mod.rs"]
mod cache;
#[allow(dead_code)]
#[path = "../src/operation.rs"]
mod operation;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    if std::env::var("CODE_RCL_RUN_BENCHMARKS").as_deref() != Ok("1") {
        println!(
            "search benchmark: set CODE_RCL_RUN_BENCHMARKS=1 and run cargo bench --bench search"
        );
        return Ok(());
    }
    for size in [1000, 10_000, 100_000] {
        let temp = tempfile::tempdir()?;
        let source = (0..size)
            .map(|n| format!("pub fn symbol_{n:06}() {{}}\n"))
            .collect::<String>();
        std::fs::write(temp.path().join("lib.rs"), &source)?;
        let mut db = cache::CacheDb::open(temp.path())?;
        let started = Instant::now();
        let hash = blake3::hash(source.as_bytes()).to_hex().to_string();
        let parsed = analysis::parse_file(analysis::Language::Rust, &source);
        assert_eq!(parsed.symbols.len(), size);
        db.replace_file_analysis(
            "lib.rs",
            "rust",
            &hash,
            None,
            Some(source.len() as i64),
            parsed.parse_ok,
            &parsed.symbols,
            &parsed.imports,
            &parsed.refs,
            &parsed.scopes,
            &parsed.bindings,
            &parsed.string_literals,
            Some(source.as_bytes()),
        )?;
        let sync_us = started.elapsed().as_micros();
        drop(db);
        let started = Instant::now();
        let db = cache::CacheDb::open(temp.path())?;
        assert_eq!(
            db.search_symbols("symbol_000999", "exact", None, false, 25)?
                .len(),
            1
        );
        let cold_us = started.elapsed().as_micros();
        let mut times = Vec::new();
        for (mode, query) in [
            ("exact", "symbol_000999"),
            ("prefix", "symbol_0009"),
            ("substring", "0009"),
        ] {
            let started = Instant::now();
            for _ in 0..10 {
                db.search_symbols(query, mode, None, false, 25)?;
            }
            times.push((mode, started.elapsed().as_micros() / 10));
        }
        let started = Instant::now();
        db.fuzzy_search_symbols("symbl_000999", 6)?;
        println!(
            "{}",
            serde_json::json!({"symbols":size,"hash_parse_store_us":sync_us,"reopen_exact_us":cold_us,"warm_query_us":times,"fuzzy_us":started.elapsed().as_micros()})
        );
    }
    Ok(())
}
