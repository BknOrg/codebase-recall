mod worker;

struct Dispatcher(fn(u32) -> u32);

async fn cmd_run() {
    // The only call to `run_inner` lives inside a macro's token tree.
    futures::try_join!(
        async { worker::run_inner(1).await },
        async { 0u32 },
    );
    let _ = Dispatcher(worker::dispatch_handler);
}

fn tally(v: u32) -> u32 {
    // A local passed as an argument must NOT become an edge.
    let finish = 3u32;
    helper(finish);
    assert!(worker::finish(v) > 0);
    v
}

fn helper(x: u32) -> u32 {
    x
}
