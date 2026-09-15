# Demo: layered reference resolution (Hybrid, slice 1)

Two tiny projects that show the new resolver settling calls the way a compiler
would — by **scope**, **import binding**, and **declared type composition** —
instead of by matching names.

**Guided walkthrough** — narrates every resolved call edge and the layer that
produced it. Add the `before` flag to also build `main` and show the old
name-only result side by side (it stashes and restores your working tree):

```sh
bash docs/demo/walkthrough.sh            # or: --before
```
```powershell
powershell -ExecutionPolicy Bypass -File docs\demo\walkthrough.ps1    # or: -Before
```

Both need `cargo` and `python` (or `py`) on PATH.

## Run it

```sh
cargo build --release        # or: cargo build   (then use target/debug)

./target/release/code-rcl graph --project docs/demo/mini_rust --format json -o /tmp/mr.json
./target/release/code-rcl graph --project docs/demo/mini_ts   --format json -o /tmp/mt.json

# eyeball the call edges
jq -r '.nodes as $n | .edges[] | select(.kind=="calls")
       | ( $n[] | select(.id==.source) ) as $s | ""' /tmp/mr.json   # (or just open the JSON)
```

Simpler: `code-rcl serve --project docs/demo/mini_rust` and hover the edges.

## What you should see

### `mini_rust/` — `Job::start` makes four calls

| call site | resolves to | why | conf |
|---|---|---|---|
| `helper()` | `lib.rs::helper` | **L1** scope: bare name, same-file symbol | 0.95 |
| `pick()` | `lib.rs::pick` | **L1** scope | 0.95 |
| `crate::scheduler::run()` | `scheduler.rs::run` | **L2** import binding: `scheduler` is a module path, so this is the free function — **not** `Retry::run` | 0.90 |
| `self.retry.run()` | `worker.rs::Retry::run` | **L3** type composition: field `retry: Retry` → method `run` on `Retry` | 0.80 |

`scheduler` is also a **parameter** of `start`; a bare `scheduler` would resolve
to the parameter, never the module (shadowing).

### `mini_ts/` — `Service::lookup` calls `findById` twice

Both `UserStore` and `OrderStore` define `findById`. The declared field types

```ts
private users: UserStore;
private orders: OrderStore;
```

make `this.users.findById()` resolve to `UserStore.findById` and
`this.orders.findById()` to `OrderStore.findById` — two edges to two different
methods, not one guessed edge.

## Before / after on `main`

```sh
# after (this branch)
cargo build --release
./target/release/code-rcl graph --project docs/demo/mini_rust --format json -o after.json

# before
git stash && git switch main && cargo build --release
./target/release/code-rcl graph --project docs/demo/mini_rust --format json -o before.json
git switch - && git stash pop && cargo build --release

jq -S '[.edges[]|select(.kind=="calls")|{source,target}]' before.json
jq -S '[.edges[]|select(.kind=="calls")|{source,target}]' after.json
```

On `main` the two colliding `run` methods collapse onto **one wrong edge**
(`start → Retry::run` standing in for `scheduler::run()`), and `scheduler::run`
is missed entirely. After, both calls land on the right target.

The machine-checked version of this is
`tests/fixtures/resolve_app/` + `cargo test --test resolve_accuracy -- --nocapture`.
