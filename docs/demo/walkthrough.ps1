<#
.SYNOPSIS
  Guided, re-runnable walkthrough of the layered reference resolver (PowerShell).
.DESCRIPTION
  Runs `code-rcl graph` on the two demo projects and tags every `calls` edge with
  the layer that produced it (L1 scope / L2 import binding / L3 receiver type /
  L4 disambiguation), read off the confidence plus the source ref.

  Touches only docs/demo/*/.code-rcl (created then removed). With -Before it also
  builds `main` for contrast, using a `git stash` that is always restored.
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File docs\demo\walkthrough.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File docs\demo\walkthrough.ps1 -Before
#>
[CmdletBinding()]
param([switch]$Before)

# Native tools (cargo / git / code-rcl) write progress to stderr; under 'Stop'
# PowerShell 5.1 turns that into a terminating error. Use 'Continue' and check
# $LASTEXITCODE explicitly instead.
$ErrorActionPreference = 'Continue'

$root = (& git rev-parse --show-toplevel).Trim()
Set-Location $root

$bin = Join-Path $root 'target\debug\code-rcl.exe'

# --- find a real Python (skip the Windows Store "python3" alias) --------------
$py = $null
foreach ($c in 'python', 'py', 'python3') {
    if (-not (Get-Command $c -ErrorAction SilentlyContinue)) { continue }
    try {
        & $c -c 'import sys' 2>$null
        if ($LASTEXITCODE -eq 0) { $py = $c; break }
    } catch {}
}
if (-not $py) { Write-Error 'python not found (need python / py / python3)'; exit 1 }

function Build([string]$what) {
    Write-Host "building $what ..."
    & cargo build -q
    if ($LASTEXITCODE -ne 0) { Write-Error 'build failed'; exit 1 }
}

function Say([string]$text) {
    Write-Host ''
    Write-Host $text -ForegroundColor White
}

# --- shared analyzer: parse the graph JSON + cache.db, print tagged edges ----
$analyzer = @'
import json, sqlite3, sys, os
gj, dbp, label = sys.argv[1], sys.argv[2], sys.argv[3]
raw = len(sys.argv) > 4 and sys.argv[4] == "raw"
g = json.load(open(gj))
nid = {n["id"]: n for n in g["nodes"]}

if raw:
    print()
    print(f"### {label}")
    bar = "-" * 78
    print(bar)
    calls = sorted((e for e in g["edges"] if e["kind"] == "calls"),
                   key=lambda e: (nid[e["source"]].get("path", ""), nid[e["source"]]["label"]))
    if not calls:
        print("  (no call edges)")
    for e in calls:
        s, t = nid[e["source"]], nid[e["target"]]
        tline = (t.get("lines") or [None])[0]
        tloc = f"@{tline}" if tline else ""
        print(f"  {s.get('path','?')}::{s['label']:<12} ->  {t.get('path','?')}::{t['label']}{tloc}"
              f"   (conf {e['confidence']:.2f})")
    print(bar)
    sys.exit(0)

sym_by_id, field_types, all_names = {}, {}, set()
refs = []
if os.path.exists(dbp):
    c = sqlite3.connect(dbp)
    for sid, path, name, kind, tn, sl in c.execute(
        "select s.id,f.path,s.name,s.kind,s.type_name,s.start_line "
        "from symbols s join files f on f.id=s.file_id"):
        sym_by_id[sid] = dict(path=path, name=name, kind=kind, type_name=tn, line=sl)
        all_names.add(name)
    for owner_name, fname, ftype in c.execute(
        "select os.name, b.name, b.type_expr "
        "from bindings b join scopes sc on sc.id=b.scope_id "
        "join symbols os on os.id=sc.owner_symbol_id "
        "where b.binding_kind='field' and b.type_expr is not null"):
        field_types[(owner_name, fname)] = ftype
    for path, esid, rn, recv, rk, loc, resolved, rkind in c.execute(
        "select f.path,r.from_symbol_id,r.name,r.receiver,r.receiver_kind,"
        "r.local_only,r.resolved_symbol_id,r.ref_kind "
        "from refs r join files f on f.id=r.file_id"):
        refs.append(dict(path=path, esid=esid, name=rn, recv=recv, rk=rk,
                         loc=loc, resolved=resolved, kind=rkind))

def chain_type(enclosing_type, recv):
    segs = (recv or "").split(".")
    cur = enclosing_type if segs[0] in ("self", "this") else segs[0]
    for s in segs[1:]:
        cur = field_types.get((cur, s))
        if cur is None:
            return None
    return cur

def label_for(conf, recv, tgt_type):
    if conf >= 0.94:
        return "L1", "scope: bare name resolves to a same-file symbol"
    if conf >= 0.88:
        return "L2", (f"import binding: `{recv}` is a module -> its exported symbol"
                      if recv else "import binding: a named import")
    if conf >= 0.83:
        return "L3", f"receiver type: `{recv}` is the type `{tgt_type}`"
    if conf >= 0.75:
        return "L3", f"type composition: `{recv}` -> field type `{tgt_type}` -> its method"
    return "L4", "scored disambiguation: a same-named definition won by a margin"

print()
print(f"### {label}")
bar = "-" * 78
print(bar)
calls = sorted((e for e in g["edges"] if e["kind"] == "calls"),
               key=lambda e: (nid[e["source"]].get("path", ""), nid[e["source"]]["label"]))
if not calls:
    print("  (no call edges)")

used = set()
for e in calls:
    s, t = nid[e["source"]], nid[e["target"]]
    conf = e["confidence"]
    tline = (t.get("lines") or [None])[0]
    tsid = next((sid for sid, v in sym_by_id.items()
                 if v["name"] == t["label"] and v["path"] == t.get("path")
                 and v["line"] == tline), None)
    tgt_type = sym_by_id.get(tsid, {}).get("type_name")

    cands = [r for r in refs if r["kind"] == "call" and r["name"] == t["label"]
             and sym_by_id.get(r["esid"], {}).get("name") == s["label"]
             and sym_by_id.get(r["esid"], {}).get("path") == s.get("path")]
    chosen = None
    if len(cands) == 1:
        chosen = cands[0]
    else:
        encl_type = sym_by_id.get(cands[0]["esid"], {}).get("type_name") if cands else None
        for r in cands:
            if id(r) in used:
                continue
            if tgt_type and chain_type(encl_type, r["recv"]) == tgt_type:
                chosen = r; break
            last = (r["recv"] or "").split("::")[-1].split(".")[-1]
            if tgt_type and last == tgt_type:
                chosen = r; break
        chosen = chosen or next((r for r in cands if id(r) not in used), cands[0] if cands else None)
    if chosen is not None:
        used.add(id(chosen))
    recv = chosen["recv"] if chosen else None

    L, why = label_for(conf, recv, tgt_type)
    tloc = f"@{tline}" if tline else ""
    print(f"  {s.get('path','?')}::{s['label']:<12} ->  {t.get('path','?')}::{t['label']}{tloc}")
    print(f"        [{L}] {why}   (conf {conf:.2f})")

def edge_exists(r):
    for e in calls:
        s, t = nid[e["source"]], nid[e["target"]]
        if t["label"] == r["name"] and sym_by_id.get(r["esid"], {}).get("name") == s["label"] \
           and sym_by_id.get(r["esid"], {}).get("path") == s.get("path"):
            return True
    return False

misses = []
for r in refs:
    if r["kind"] != "call" or edge_exists(r):
        continue
    encl = sym_by_id.get(r["esid"], {}).get("name", "<file>")
    if r["name"].endswith("!"):
        reason = "a macro / builtin, not a project symbol"
    elif r["loc"]:
        reason = f"`{r['name']}` binds to a local/param here -> no edge (correct)"
    elif r["name"] not in all_names:
        reason = (f"`{(r['recv'] + '.') if r['recv'] else ''}{r['name']}` is external "
                  "(no project symbol by that name)")
    else:
        reason = "ambiguous -> dropped by precision mode"
    misses.append(f"  {r['path']}::{encl} -> {r['name']}()   -- {reason}")
if misses:
    print("  no edge:")
    for m in sorted(set(misses)):
        print("  " + m)
print(bar)
'@

function Explain([string]$proj, [string]$label, [switch]$Raw) {
    $cache = Join-Path $proj '.code-rcl'
    Remove-Item -Recurse -Force $cache -ErrorAction SilentlyContinue
    $out = [System.IO.Path]::GetTempFileName()
    & $bin graph --project $proj --format json -o $out 2>&1 | Out-Null
    $db = Join-Path $cache 'cache.db'
    $pyArgs = @('-', $out, $db, $label)
    if ($Raw) { $pyArgs += 'raw' }
    $analyzer | & $py $pyArgs
    Remove-Item $out -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $cache -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
Say 'Layered reference resolution - live walkthrough'
Write-Host @'
The resolver tries layers from most-certain to most-guessy and stops at the
first that answers. The confidence on each edge tells you which layer won:

  0.95  L1  lexical scope        (bare name -> a symbol in the same file)
  0.90  L2  import binding       (named import, or ns.foo() through a module)
  0.85  L3  receiver type        (self, Foo::bar, an annotated parameter)
  0.80  L3  type composition     (self.field.method() walked via field types)
  <=.70 L4  scored disambiguation (a same-named definition wins by a margin)
'@

Build 'current branch (after)'

# ---------------------------------------------------------------------------
Say '1) docs/demo/mini_rust  -  Job::start makes four calls'
Write-Host @'
  scheduler.rs :  pub fn run()            <- a FREE function
  worker.rs    :  impl Retry { fn run() } <- a METHOD with the SAME name
  lib.rs       :  struct Job { retry: Retry }
                  fn start(&self, scheduler: u32) {
                      helper(); pick();          // bare calls
                      crate::scheduler::run();   // the free function
                      self.retry.run();          // Retry::run, via the field type
                  }
  `scheduler` is also a PARAMETER here - a bare `scheduler` would be the param,
  never the module.
'@
Explain 'docs/demo/mini_rust' 'mini_rust: resolved call edges'

# ---------------------------------------------------------------------------
Say '2) docs/demo/mini_ts  -  one method name, two classes'
Write-Host @'
  store.ts   :  class UserStore  { findById() }
                class OrderStore { findById() }
  service.ts :  class Service {
                    private users: UserStore;
                    private orders: OrderStore;
                    lookup(id) { this.users.findById(id); this.orders.findById(id); }
                }
  The declared field types make each findById call land on the right class.
'@
Explain 'docs/demo/mini_ts' 'mini_ts: resolved call edges'

# ---------------------------------------------------------------------------
if ($Before) {
    Say '3) BEFORE - the old name-only matcher (branch: main)'

    & git diff --quiet;        $dirty  = ($LASTEXITCODE -ne 0)
    & git diff --cached --quiet; if ($LASTEXITCODE -ne 0) { $dirty = $true }
    $curr = (& git rev-parse --abbrev-ref HEAD).Trim()
    $stashed = $false

    try {
        if ($dirty) {
            & git stash push -q -m 'walkthrough-before'
            if ($LASTEXITCODE -ne 0) { Write-Error 'git stash failed'; exit 1 }
            $stashed = $true
        }
        & git switch -q main
        if ($LASTEXITCODE -ne 0) { Write-Error 'cannot switch to main'; exit 1 }
        Build 'main (before)'
        Explain 'docs/demo/mini_rust' 'mini_rust @ main: name-only matching' -Raw
        Explain 'docs/demo/mini_ts'   'mini_ts @ main: name-only matching'   -Raw
    }
    finally {
        & git switch -q $curr 2>$null
        if ($stashed) { & git stash pop -q 2>$null }
        Build "restoring $curr"
    }

    Say 'Notice on main (name-only matching):'
    Write-Host '  mini_rust: start() shows just ONE run edge, to worker.rs::run (0.55) --'
    Write-Host '             it stands in for BOTH calls, and scheduler::run() is missed.'
    Write-Host '  mini_ts:   lookup() has NO edges at all -- two methods named findById'
    Write-Host '             exist, so the old heuristic refuses to pick and drops both.'
    Write-Host ''
    Write-Host '  After (this branch): 4 correct edges for start(), and lookup() resolves'
    Write-Host '  to UserStore.findById and OrderStore.findById separately.'
}

Say 'Machine-checked version:'
Write-Host '  cargo test --test resolve_accuracy -- --nocapture'
Write-Host ''
