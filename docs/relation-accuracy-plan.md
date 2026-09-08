# Rencana Peningkatan Akurasi Relasi Kode (Relation Accuracy Plan)

> Dokumen ini ditujukan untuk orang yang **baru pertama kali** menyentuh proyek
> `code-rcl`. Semua istilah dijelaskan saat pertama muncul, ada analogi, contoh
> kode kecil, dan diagram. Kalau ada bagian yang terasa "kok tiba-tiba", laporkan —
> berarti dokumennya yang kurang, bukan kamu yang bodoh.

---

## 0. Ringkasan satu paragraf

`code-rcl` menggambar **graph relasi kode**: titik (node) = file / fungsi / tipe,
garis (edge) = "file A meng-import file B", "fungsi X memanggil fungsi Y".
Masalahnya: saat menentukan "fungsi Y yang mana", program **hanya mencocokkan nama
string**. Padahal nama seperti `new`, `run`, `build`, `get`, `parse`, `handle`
muncul puluhan kali di proyek yang sama. Akibatnya garis panggilan sering salah
sasaran atau malah dibuang. Rencana ini menaikkan akurasi dengan meniru cara
**compiler** mengenali simbol: pakai **scope** (lingkup), **import binding**
(pengikatan nama impor), dan **tipe receiver** (tipe objek pemanggil method) —
tanpa membangun type-checker penuh.

---

## 1. Kenapa "cocok nama saja" tidak cukup

### 1.1 Analogi kantor "Budi"

Bayangkan kantor dengan 5 karyawan bernama **Budi**. Kamu menemukan memo:

> "Tolong serahkan laporan ini ke **Budi**."

Budi yang mana? Manusia (dan compiler) menyelesaikannya dengan 3 petunjuk:

1. **Scope / lingkup** — "Budi yang satu ruangan denganmu." Kalau di ruanganmu ada
   Budi, hampir pasti itu yang dimaksud. Kalau tidak ada, baru cari ke ruangan
   sebelah, lalu ke lantai lain.
2. **Import / daftar tamu** — "Budi yang tadi pagi kamu daftarkan namanya di buku
   tamu meja resepsionis." Kamu sendiri yang menulis `import { Budi } from "lantai-3"`,
   jadi jelas Budi lantai 3.
3. **Tipe / jabatan** — "Serahkan ke Budi, **Manajer Keuangan**." Kata "laporan
   keuangan" + jabatan mempersempit ke satu orang.

Program kita saat ini seperti orang yang **meneriakkan "BUDI!" ke seluruh gedung**
dan menyerahkan memo ke Budi pertama yang menoleh. Kadang benar, sering salah.

### 1.2 Istilah yang dipakai sepanjang dokumen

| Istilah | Arti singkat |
|---|---|
| **symbol** | Sesuatu yang **dideklarasikan**: fungsi, method, struct/class, enum, tipe, variabel top-level, module. Di DB: tabel `symbols`. |
| **reference / ref** | Tempat sebuah **nama dipakai**: pemanggilan `foo()`, penggunaan tipe `Foo`, baca/tulis variabel. Di DB: tabel `refs`. |
| **binding** | Ikatan "nama ini → deklarasi itu" **di dalam satu lingkup**. Contoh: di file ini, `db` mengikat ke parameter fungsi, bukan ke variabel global `db`. |
| **scope / lingkup** | Wilayah kode tempat sebuah binding berlaku: isi sebuah fungsi, isi sebuah blok `{ }`, isi sebuah class, isi satu file (module scope). Scope bersarang seperti matryoshka. |
| **shadowing** | Nama di scope dalam **menutupi** nama sama di scope luar. `let handler = ...` di dalam fungsi menutupi `import handler`. |
| **receiver** | Objek di sebelah kiri titik pada `objek.method()`. Di `user.save()`, receiver-nya `user`. |
| **resolution** | Proses memutuskan sebuah ref menunjuk ke symbol yang mana. |
| **confidence** | Angka 0..1 seberapa yakin kita dengan hasil resolution. Edge dengan confidence di bawah `--min-confidence` (default 0.4) dibuang. |

### 1.3 Tiga kegagalan nyata di kode sekarang

Lihat fungsi inti [`resolve_ref()`](../src/graph/resolve/refs.rs). Alurnya sekarang:

```text
1. Ada symbol di FILE YANG SAMA dengan nama sama?  -> pakai, confidence 1.0
   (ambil yang PERTAMA ketemu; tidak cek scope, tidak cek shadowing)
2. Ada binding dari import dengan nama itu?         -> pakai, confidence 1.0
3. Selain itu: kumpulkan SEMUA symbol se-proyek yang namanya sama
   (defs_by_name), lalu tebak pakai heuristik receiver + keunikan export
   -> confidence 0.45 .. 0.7
```

**Kegagalan A — method nama umum di file yang sama menang keliru**

```rust
// file: worker.rs
impl Retry {
    fn run(&self) { /* ... */ }        // <-- symbol "run" #1 di file ini
}

impl Job {
    fn start(&self) {
        scheduler::run();              // maksudnya scheduler::run, di file LAIN
    }
}
```

Langkah 1 melihat "ada `run` di file yang sama" → langsung tarik garis
`start -> Retry::run` dengan confidence 1.0. Salah total, dan karena confidence-nya
1.0 tidak akan pernah kalah oleh kandidat yang benar.

**Kegagalan B — variabel lokal men-shadow import, tapi edge tetap ke import**

```ts
import { handler } from "./default-handler";

function register(handler: Handler) {   // parameter juga bernama "handler"
  handler.attach();                     // ini parameter, BUKAN import
}
```

Langkah 1 tidak menemukan symbol `handler` di file ini (parameter tidak disimpan
sebagai symbol), lalu langkah 2 menemukan binding import `handler` → tarik garis ke
`./default-handler`. Padahal `handler.attach()` memakai **parameter**. Seharusnya:
tidak ada edge antar-symbol sama sekali (atau edge ke tipe `Handler`).

**Kegagalan C — `x.parse()` nyambung ke semua `parse` di proyek**

```python
result = body.parse()
```

Langkah 1 & 2 gagal. Langkah 3 mengumpulkan **setiap** fungsi/method bernama `parse`
di seluruh repo. Heuristik `Receiver::Value` cuma lolos kalau **persis satu** yang
bernama `parse` berjenis `method`. Di repo nyata ada banyak → hasilnya: tidak ada
edge (recall hilang), atau kalau kebetulan satu, bisa salah (precision hilang).

Kesimpulan: kita butuh **scope**, **import binding yang benar**, dan **sedikit
pengetahuan tipe**. Itu isi rencana ini.

---

## 2. Peta wilayah: bagaimana `code-rcl` bekerja sekarang

```text
        code-rcl sync
        ┌───────────────────────────────────────────────────────────────┐
        │  walker.rs  : jalan-jalan direktori, kumpulkan file sumber     │
        │  analysis/  : tree-sitter parse tiap file -> AST               │
        │      rust.rs / javascript.rs / python.rs / sfc.rs              │
        │      hasilkan ParsedFile {                                     │
        │        symbols: Vec<NewSymbol>   (nama, kind, parent_index,    │
        │                                   byte range, is_exported)     │
        │        imports: Vec<NewImport>   (raw_specifier, imported_name,│
        │                                   alias, is_relative)          │
        │        refs:    Vec<NewRef>      (nama, ref_kind, receiver,    │
        │                                   start_byte)                  │
        │      }                                                         │
        └───────────────────────────────┬───────────────────────────────┘
                                        │
                                        ▼
        cache/mod.rs :: replace_file_analysis()
        ┌───────────────────────────────────────────────────────────────┐
        │  Simpan ke SQLite (.code-rcl/cache.db):                        │
        │    tabel files / symbols / imports / refs                      │
        │  - parent_symbol_id di-wire dari parent_index                  │
        │  - refs.from_symbol_id = innermost_symbol(byte): symbol        │
        │    terkecil yang byte range-nya membungkus ref  <-- caller     │
        └───────────────────────────────┬───────────────────────────────┘
                                        │
                                        ▼
        graph/resolve/mod.rs :: build(db, opts)
        ┌───────────────────────────────────────────────────────────────┐
        │  1. bikin Node untuk tiap file & symbol                        │
        │  2. resolve_import()  : "./util" -> "src/util.ts" (per bahasa) │
        │       + isi `binding: (file_id, local_name) -> symbol_id`      │
        │  3. untuk tiap ref: resolve_ref() -> (target_symbol, conf)     │
        │       drop kalau conf < min_confidence                         │
        │  4. postprocess: collapse ke file / focus / rollup direktori / │
        │       batasi max_nodes                                         │
        └───────────────────────────────┬───────────────────────────────┘
                                        │
                                        ▼
             CodeGraph { nodes, edges }  ->  render html / json / dot
```

File kunci yang akan sering kamu buka:

- [`src/analysis/mod.rs`](../src/analysis/mod.rs) — definisi `ParsedFile`, titik pasang
  helper generik.
- [`src/analysis/rust.rs`](../src/analysis/rust.rs),
  [`src/analysis/javascript.rs`](../src/analysis/javascript.rs),
  [`src/analysis/python.rs`](../src/analysis/python.rs),
  [`src/analysis/sfc.rs`](../src/analysis/sfc.rs) — tiap `Walker` sudah punya field
  `stack: Vec<usize>` (rantai symbol induk). Kita perluas jadi rantai **scope**.
- [`src/cache/schema.rs`](../src/cache/schema.rs) — daftar `MIGRATIONS`. Kita tambah v2.
- [`src/cache/models.rs`](../src/cache/models.rs) & [`src/cache/mod.rs`](../src/cache/mod.rs)
  — struct baris DB + `replace_file_analysis()` + `innermost_symbol()`.
- [`src/graph/resolve/refs.rs`](../src/graph/resolve/refs.rs) — `resolve_ref()`, jantung
  yang akan ditulis ulang jadi pipeline berlapis.
- [`src/graph/resolve/imports.rs`](../src/graph/resolve/imports.rs) — resolusi spesifier
  import per bahasa.
- [`src/graph/resolve/mod.rs`](../src/graph/resolve/mod.rs) — `build()`, tempat `binding`
  dan `defs_by_name` dibangun. Perhatikan baris `let _ = sym_by_id; // reserved for
  future receiver-type resolution` — itu titik yang rencana ini "mengaktifkan".

---

## 3. Metode yang diusulkan: **Resolver Berlapis**

### 3.1 Ide besar

Selesaikan tiap ref lewat **lapisan dari paling pasti ke paling menebak**. Begitu
sebuah lapisan menjawab **dengan yakin**, berhenti. Confidence = fungsi dari lapisan
mana yang menjawab + seberapa bersih jawabannya.

```text
ref "foo" / "obj.foo" di dalam fungsi bar() di file F
        │
        ▼
[L1] Scope lookup di file F  ──► ketemu binding lokal?
        ├─ local / param        → BUKAN edge antar-symbol (stop)
        ├─ symbol di file F      → edge ke symbol itu           conf 0.95
        └─ import                → lanjut ke L2
        │  (tidak ada binding)   → lanjut ke L3
        ▼
[L2] Import binding presisi  ──► nama ini di-import dari mana?
        ├─ import { foo }        → symbol `foo` di module target conf 0.9
        └─ import * as ns; ns.foo→ symbol `foo` di module ns     conf 0.9
        ▼
[L3] Inferensi tipe receiver ──► tahu tipe `obj`?
        ├─ self / this / Self    → method di class/impl pembungkus conf 0.9
        ├─ Foo::foo (Foo tipe)   → assoc fn / method di tipe Foo    conf 0.85
        ├─ obj: T (anotasi)      → method `foo` di tipe T           conf 0.85
        └─ obj = new Foo()       → method `foo` di tipe Foo         conf 0.8
        ▼
[L4] Skoring disambiguasi     ──► masih banyak kandidat nama sama
        rangking pakai sinyal (import-reachable, exported, arity, jarak)
        ├─ ada pemenang jelas (menang margin) → edge                conf 0.5–0.7
        └─ seri                                → TIDAK ada edge (mode presisi)
```

Lapisan lama (langkah 3 `defs_by_name` sekarang) tetap ada **sebagai L4**, tapi:
(a) hanya dipakai kalau L1–L3 gagal, (b) confidence-nya diturunkan, (c) diberi
sinyal tambahan supaya tebakannya lebih terarah.

### 3.2 Lapisan 0 — Model data lebih kaya (fondasi)

Ini bagian "Hybrid": kita **belum** membangun compiler, tapi bentuk datanya dibuat
mirip yang dipakai tool kelas berat (SCIP dari Sourcegraph, stack-graphs dari GitHub)
sehingga upgrade nanti = **mengisi lebih lengkap**, bukan merombak skema.

Tambahkan **migrasi v2** di [`src/cache/schema.rs`](../src/cache/schema.rs). Migrasi
bersifat *forward-only*: `MIGRATIONS[1]` menaikkan DB dari v1 ke v2, `V1` tidak
disentuh.

```sql
-- V2 (baru)
CREATE TABLE scopes (
    id              INTEGER PRIMARY KEY,
    file_id         INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    parent_scope_id INTEGER REFERENCES scopes(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,        -- module | function | method | class | block
    start_byte      INTEGER NOT NULL,
    end_byte        INTEGER NOT NULL
);

CREATE TABLE bindings (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    scope_id     INTEGER NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    binding_kind TEXT NOT NULL,           -- local | param | symbol | import | namespace
    symbol_id    INTEGER REFERENCES symbols(id) ON DELETE SET NULL,   -- kalau kind=symbol
    import_id    INTEGER REFERENCES imports(id) ON DELETE SET NULL,   -- kalau kind=import/namespace
    type_expr    TEXT                     -- anotasi tipe mentah, mis. "CacheDb" / "Option<T>"
);

CREATE INDEX idx_scopes_file   ON scopes(file_id);
CREATE INDEX idx_bindings_file ON bindings(file_id);
CREATE INDEX idx_bindings_name ON bindings(name);

-- kolom tambahan di tabel lama (SQLite: ALTER TABLE ... ADD COLUMN, aman & cepat)
ALTER TABLE symbols ADD COLUMN params_json  TEXT;      -- [{"name":"db","type":"CacheDb"}, ...]
ALTER TABLE symbols ADD COLUMN return_type  TEXT;
ALTER TABLE symbols ADD COLUMN receiver_type TEXT;     -- utk method Rust: nama tipe dari blok impl

ALTER TABLE refs ADD COLUMN arg_count          INTEGER;   -- jumlah argumen di call site
ALTER TABLE refs ADD COLUMN receiver_kind       TEXT;      -- none | path | value | self
ALTER TABLE refs ADD COLUMN receiver_type       TEXT;      -- kalau bisa disimpulkan saat walk
ALTER TABLE refs ADD COLUMN resolved_symbol_id  INTEGER REFERENCES symbols(id) ON DELETE SET NULL;
ALTER TABLE refs ADD COLUMN resolved_confidence REAL;
```

Perubahan struct pendamping di [`src/cache/models.rs`](../src/cache/models.rs):
`NewSymbol` dapat `params: Vec<Param>`, `return_type`, `receiver_type`;
`NewRef` dapat `arg_count`, `receiver_kind`; struct baru `NewScope`, `NewBinding`;
`ParsedFile` di [`src/analysis/mod.rs`](../src/analysis/mod.rs) dapat
`scopes: Vec<NewScope>` dan `bindings: Vec<NewBinding>`.

> **Kenapa hasil resolusi di-cache di `refs.resolved_symbol_id`?**
> Sekarang `resolve_ref` dihitung ulang **setiap kali** render graph. Kalau kita
> pindahkan komputasi berat (scope + tipe) ke tahap `sync`, render jadi ringan dan
> kita hanya menghitung ulang file yang hash isinya berubah (logika ini sudah ada
> di [`sync_cache`](../src/commands/sync.rs)). Cross-file resolution (L2–L4) tetap
> di tahap resolve karena butuh tahu semua file; L1 (murni dalam-file) bisa
> di-`sync`.

### 3.3 Lapisan 1 — Scope tree per file

**Tujuan:** ganti aturan "ambil symbol pertama dengan nama sama di file" menjadi
"cari binding sebenarnya dengan menaiki rantai scope dari dalam ke luar".

**Cara buat scope tree** (di tiap `analysis/*.rs`, saat `Walker` berjalan):

`Walker` sekarang punya `stack: Vec<usize>` berisi indeks symbol induk. Kita
tambah `scope_stack: Vec<usize>` berisi indeks ke `out.scopes`. Aturannya:

- Masuk file → buat 1 scope `module` (root).
- Masuk `function` / `method` / arrow function / `class` / blok `{ }` yang penting
  → `push` scope anak dengan `kind` sesuai dan `start_byte..end_byte` node itu.
- Keluar node → `pop`.
- Saat menemukan **deklarasi nama**, catat `NewBinding` di scope teratas saat itu:

  | Sumber di kode | `binding_kind` | catatan |
  |---|---|---|
  | parameter fungsi/method | `param` | ambil `type_expr` dari anotasi bila ada (TS, Python hint, Rust) |
  | `let` / `const` / `var` (JS), `x = ...` (Py) di dalam fungsi | `local` | `type_expr` dari anotasi atau dari RHS `new Foo()` (lihat L3) |
  | deklarasi fungsi/struct/class/enum/type di file ini | `symbol` | `symbol_id` diisi |
  | `import { x }` / `use a::b` / `from m import x` | `import` | `import_id` diisi |
  | `import * as ns` / `use m::*` / `from m import *` | `namespace` | `import_id` diisi |

**Cara pakai saat resolusi** (fungsi baru, mis. `resolve_in_file()` di
[`refs.rs`](../src/graph/resolve/refs.rs)):

```text
fn resolve_in_file(ref, scopes, bindings) -> InFileResult:
    scope = innermost scope whose [start_byte, end_byte) contains ref.start_byte
    while scope is not None:
        if bindings has (scope.id, ref.name):
            b = that binding
            match b.binding_kind:
                "local" | "param" -> return LocalOrParam(type_expr = b.type_expr)
                "symbol"          -> return SameFileSymbol(b.symbol_id)
                "import"          -> return ImportBinding(b.import_id)
                "namespace"       -> return Namespace(b.import_id)
        scope = scope.parent
    return NotFound
```

Efek langsung:

- **Kegagalan A** hilang: `scheduler::run()` di dalam `Job::start` — scope lookup
  untuk `run` menaiki: blok → `start` → `impl Job` → module. Tidak ada binding
  `run` di jalur itu (method `Retry::run` ada di sub-tree scope **lain**), jadi
  L1 = `NotFound` → lanjut ke L3/L4 yang lihat `receiver = "scheduler"`.
- **Kegagalan B** hilang: `handler` ketemu sebagai `param` di scope `register` →
  hasil `LocalOrParam` → **tidak dibuat edge ke import**. (Kalau `type_expr =
  "Handler"`, L3 bisa bikin edge `register -> Handler` sebagai relasi tipe.)

### 3.4 Lapisan 2 — Import binding presisi + tabel simbol modul

Sekarang [`resolve_import`](../src/graph/resolve/imports.rs) sudah memetakan
spesifier → path file, dan `build()` sudah mengisi `binding: (file_id, local) ->
symbol_id` tapi hanya untuk `import { namaTunggal }`. Yang kurang: **namespace
import**, **re-export / barrel file**, dan penelusuran transitif.

**Langkah 1 — bangun `module_exports`:**

```text
module_exports: file_id -> { export_name -> ResolvedTarget }
ResolvedTarget = Symbol(symbol_id) | ReExport(from_file_id, orig_name)
```

Isi awal dari tabel `symbols` (semua symbol `is_exported = true` di file itu).
Untuk re-export, analyzer harus menandai baris `export { x } from "./y"` /
`pub use a::b` / `from .a import b` (untuk Python, `__init__.py` sering jadi
barrel). Di JS sudah sebagian: `sfc.rs`/`javascript.rs` memperlakukan
`export { a } from "..."` sebagai import — cukup tandai juga sebagai **export**
bernama `a` yang menunjuk ke `./...`.

**Langkah 2 — selesaikan re-export secara transitif (fixpoint):**

```text
ulangi sampai tidak ada perubahan:
  untuk tiap module_exports[f][name] == ReExport(g, orig):
    kalau module_exports[g][orig] sudah berupa Symbol(id):
        ganti jadi Symbol(id)
```

Fixpoint karena barrel bisa berlapis (`index.ts` re-export dari `sub/index.ts`
re-export dari `sub/impl.ts`). Batasi iterasi (mis. 10) untuk jaga-jaga siklus.

**Langkah 3 — bangun binding presisi:**

```text
binding: (file_id, local_name) -> Target
Target = Symbol(symbol_id) | Module(file_id)

untuk tiap ImportRow im di file F:
  target_file = resolve_import(im)              // sudah ada
  match bentuk import:
    `import { name as alias }` :
        sym = module_exports[target_file][name]  // sudah resolved ke Symbol
        binding[(F, alias ?? name)] = Symbol(sym)
    `import * as ns` / `use m::*` :
        binding[(F, ns)] = Module(target_file)
    `import def from "..."` :
        binding[(F, def)] = Symbol(module_exports[target_file]["default"])
```

**Cara pakai:** dari hasil L1 —

- `ImportBinding(import_id)` → cari `binding[(F, ref.name)]`:
  - `Symbol(id)` → edge ke `id`, confidence **0.9**.
  - `Module(fid)` sebaiknya tidak terjadi di sini (itu jalur namespace).
- `Namespace(import_id)` dan ref berbentuk `ns.foo()` (receiver == `ns`) →
  `binding[(F, ns)] = Module(fid)` → cari `module_exports[fid]["foo"]` →
  edge, confidence **0.9**.

Efek: import path yang benar tidak lagi "kebetulan cocok nama" tapi betul-betul
ditelusuri sampai deklarasi aslinya, termasuk lewat barrel file.

### 3.5 Lapisan 3 — Inferensi tipe receiver ringan

**Tujuan:** menyelesaikan `obj.method()` — kasus tersulit — untuk pola-pola yang
bisa ditebak **tanpa** type inference penuh.

**Langkah 1 — bangun `type_methods`:**

```text
type_methods: type_symbol_id -> { method_name -> method_symbol_id }
```

Sumber:

- **JS / TS / Python:** method = symbol dengan `parent_symbol_id` menunjuk ke
  symbol `class` / `interface`. Langsung.
- **Rust:** method punya `parent_symbol_id` ke symbol `impl` (namanya `"Widget"`
  atau `"Display for Widget"`, lihat [`rust.rs push_symbol`](../src/analysis/rust.rs)).
  Tambah pass: dari nama `impl`, ekstrak **nama tipe** (`"Widget"`), lalu
  gabungkan semua method dari **semua** blok `impl Widget` + `impl Trait for
  Widget` ke bawah `type_symbol_id` si `struct Widget`. Simpan juga di kolom
  `symbols.receiver_type` tiap method (= `"Widget"`) supaya cepat dicari.
- **Pewarisan (opsional, JS):** kalau `class B extends A`, saat lookup method di
  `B` gagal, coba `A`. Butuh analyzer menyimpan nama superclass (kolom/би­nding
  `type_expr` pada symbol class).

**Langkah 2 — simpulkan tipe receiver:**

| Pola call site | Cara simpulkan tipe | Contoh |
|---|---|---|
| `self` / `this` / `Self` | tipe = symbol class/impl yang **membungkus** ref (naik dari `from_symbol_id`) | `self.step()` di `impl Builder` → `Builder` |
| `Foo::bar` / `Foo.bar`, `Foo` PascalCase | resolve `Foo` lewat L1/L2 → kalau kena symbol tipe, tipe = `Foo` | `CacheDb.open()` → tipe `CacheDb` |
| `obj.bar()`, `obj` = param/local **beranotasi** | tipe = `type_expr` dari binding-nya (L1), resolve nama tipe itu lewat L1/L2 | `db: CacheDb` → `db.query()` → `CacheDb` |
| `obj.bar()`, `obj` = local di-assign `new Foo()` / `Foo()` / `Foo::new()` | analyzer catat `type_expr = "Foo"` di binding `local` saat lihat RHS | `let b = new Builder(); b.build()` → `Builder` |

Assignment tracking dibatasi ketat: **hanya** pola satu baris `x = <ctor>` di scope
yang sama, tanpa reassignment. Kalau `x` di-assign ulang, buang `type_expr`-nya
(set ke ambiguous). Ini menjaga implementasi tetap kecil dan tidak "berhalusinasi".

**Langkah 3 — method lookup:**

```text
kalau tipe T diketahui dan type_methods[T][ref.name] ada:
    edge ke method itu, confidence:
        self/this/Self       -> 0.9
        Foo::bar (path)      -> 0.85
        anotasi eksplisit    -> 0.85
        inferensi dari ctor  -> 0.8
kalau tidak: lanjut ke L4
```

**Batasan yang harus ditulis jujur di kode & changelog:**

- JS murni & Python tanpa type hint sering tidak punya anotasi → jalur "anotasi"
  mati, tinggal `self`/ctor/`Foo.bar`.
- Rust generik (`impl<T> Foo<T>`), tipe dari crate eksternal, trait objects →
  di luar cakupan tahap pertama; jatuh ke L4.

### 3.6 Lapisan 4 — Skoring disambiguasi

Dipakai **hanya** kalau L1–L3 tidak menghasilkan target. Ini penerus langkah 3
`defs_by_name` yang sekarang, tapi lebih pintar.

```text
kandidat = semua symbol se-proyek dengan nama == ref.name
           DAN bahasa kompatibel (languages_compatible, sudah ada)

buang kandidat yang jelas salah:
  - kalau ref.receiver_kind == "value"  -> hanya kandidat kind "method"
  - kalau ref.receiver_kind == "none"   -> hanya kandidat kind "function"/"method" bebas

skor tiap kandidat c (jumlahkan bobot):
  + 3  file ref meng-import file c            (module reachable)  <-- sinyal terkuat
  + 2  c.is_exported
  + 2  arg_count ref == jumlah param c        (arity match)
  + 1  |arg_count - params| == 1              (arity dekat, mis. beda `self`)
  + 1  c di direktori yang sama / crate sama
  + 1  c di file yang sama dgn ref            (tapi L1 sudah gagal, jadi jarang)

pemenang = skor tertinggi
kalau (skor pemenang - skor kedua) >= MARGIN (mis. 2):
    edge ke pemenang, confidence = clamp(0.4 + 0.05*skor, 0.4, 0.7)
selain itu:
    mode "precision" (default): TIDAK ada edge
    mode "recall": edge ke pemenang, confidence 0.35, tandai edge.ambiguous = true
```

`arg_count` dan `receiver_kind` berasal dari kolom `refs` baru (L0). Mode
precision/recall diatur di `config.toml`:

```toml
[graph]
min_confidence = 0.4
# "precision" = buang edge yang masih ambigu; "recall" = tetap tarik dengan confidence rendah
ambiguity_mode = "precision"
disambiguation_margin = 2
```

### 3.7 Confidence: ringkasan tabel

| Sumber jawaban | Confidence |
|---|---|
| L1 symbol di file sama (scope match) | 0.95 |
| L2 import langsung `{ name }` | 0.90 |
| L2 namespace `ns.name` | 0.90 |
| L3 `self`/`this`/`Self` method | 0.90 |
| L3 `Foo::bar` path ke tipe | 0.85 |
| L3 anotasi tipe eksplisit | 0.85 |
| L3 inferensi dari `new Foo()` | 0.80 |
| L4 menang dengan margin | 0.40–0.70 |
| L4 seri, mode recall | 0.35 (di bawah default `min_confidence` → efektif dibuang kecuali user menurunkan ambang) |

---

## 4. Urutan kerja (milestone kecil yang bisa di-review satu per satu)

> Prinsip: tiap milestone harus **kompilasi, lulus test lama, dan bisa di-merge
> sendiri**. Jangan bikin satu PR raksasa.

### M1 — Skema & model data (Lapisan 0)
- Tambah `V2` di [`schema.rs`](../src/cache/schema.rs), naikkan `SCHEMA_VERSION` ke 2,
  `MIGRATIONS` jadi `&[V1, V2]`.
- Tambah struct `NewScope`, `NewBinding`, perluas `NewSymbol`/`NewRef`/`ParsedFile`
  di [`models.rs`](../src/cache/models.rs) & [`analysis/mod.rs`](../src/analysis/mod.rs).
- Perluas [`replace_file_analysis`](../src/cache/mod.rs) untuk menulis `scopes` &
  `bindings` & kolom baru; tambah reader `all_scopes()` / `all_bindings()` di
  [`cache/mod.rs`](../src/cache/mod.rs).
- Analyzer **belum** mengisi apa-apa yang baru (semua `Vec` kosong) — resolver
  belum berubah.
- **Selesai bila:** `cargo test` hijau; hapus `cache.db` lama lalu `sync` →
  `PRAGMA user_version` = 2, tabel baru ada & kosong.

### M2 — Scope tree + bindings untuk semua bahasa (Lapisan 1)
- Helper generik di [`analysis/mod.rs`](../src/analysis/mod.rs): `ScopeBuilder`
  (push/pop scope, `bind(name, kind, ...)`), dipakai keempat walker.
- Isi di [`rust.rs`](../src/analysis/rust.rs),
  [`javascript.rs`](../src/analysis/javascript.rs),
  [`python.rs`](../src/analysis/python.rs); `sfc.rs` ikut lewat `javascript::parse`
  pada virtual source (byte offset sudah dijaga — lihat komentar di
  [`sfc.rs`](../src/analysis/sfc.rs)).
- Tambah `resolve_in_file()` di [`refs.rs`](../src/graph/resolve/refs.rs) dan
  sisipkan sebagai **L1** di depan `resolve_ref`. Sisanya (langkah 2–3 lama) tetap
  jalan sebagai fallback.
- **Selesai bila:** test baru untuk Kegagalan A & B (fixture) berubah dari salah
  jadi benar; metrik (M5) tidak turun untuk kasus lain.

### M3 — Import binding presisi (Lapisan 2)
- Analyzer menandai re-export sebagai entri export (JS/TS/Python barrel; Rust `pub use`).
- Di [`resolve/mod.rs`](../src/graph/resolve/mod.rs): bangun `module_exports`,
  jalankan fixpoint re-export, bangun `binding` versi `Target` enum.
- Dukung namespace import di ketiga bahasa.
- **Selesai bila:** fixture barrel (`pkg_app`, `ts_app` dengan `index.ts` re-export)
  menghasilkan edge ke deklarasi asli, bukan ke barrel.

### M4 — Resolver berlapis penuh (Lapisan 3 & 4)
- Bangun `type_methods` + pass Rust `impl` → tipe di [`resolve/mod.rs`](../src/graph/resolve/mod.rs)
  (aktifkan `sym_by_id` yang sekarang `let _ = ...`).
- Analyzer: isi `params_json` / `return_type` / `receiver_type` untuk symbol;
  `arg_count` / `receiver_kind` untuk ref; `type_expr` untuk binding `local`/`param`.
- Tulis ulang `resolve_ref` jadi urutan L1→L2→L3→L4 yang jelas, satu fungsi per
  lapisan, mudah dites terpisah.
- Cache hasil ke `refs.resolved_symbol_id` / `resolved_confidence` saat resolve.
- **Selesai bila:** Kegagalan C (fixture `x.parse()`) sekarang benar bila tipe bisa
  disimpulkan, dan **tidak** membuat edge asal-asalan bila tidak.

### M5 — Harness akurasi (Lapisan 5) + tuning
- Di tiap `tests/fixtures/<app>/` tambah `expected-edges.json`:

  ```json
  {
    "must_have":  [["src/job.rs::start", "src/scheduler.rs::run", "calls"]],
    "must_not_have": [["src/job.rs::start", "src/worker.rs::Retry::run", "calls"]]
  }
  ```
- `tests/resolve_accuracy.rs`: untuk tiap fixture, `sync` + `build`, hitung
  **precision** (edge benar / total edge), **recall** (edge benar / total
  `must_have`), **ambiguous-rate**. Cetak tabel. Gagal bila turun dari baseline
  yang disimpan di `tests/fixtures/baseline.json`.
- Catat angka **sebelum** (kondisi `main` sekarang) di bagian §6 dokumen ini.
- Tuning `disambiguation_margin`, bobot skor L4, ambang confidence berdasarkan
  metrik.

### M6 — Dokumentasi
- Update bagian resolve di `wiki.md` & `README.md`.
- Tambah entri `config.toml` (`ambiguity_mode`, `disambiguation_margin`) di
  [`init.rs DEFAULT_CONFIG`](../src/commands/init.rs).
- Tulis "known limitations" (§3.5) di README.

---

## 5. Risiko & keputusan yang masih terbuka

| Risiko | Dampak | Mitigasi |
|---|---|---|
| Banyak kode tanpa anotasi tipe (JS, Py) | L3 jalur "anotasi" jarang kena | Andalkan `self` + ctor-inference + `Foo.bar`; sisanya L4. Ukur berapa % ref yang tertolong. |
| Assignment tracking meledak jadi mini-interpreter | scope creep, bug halus | Batasi keras: hanya `x = <ctor>` satu baris, satu scope, tanpa reassign. Kalau ragu → ambiguous. |
| Biaya `sync` naik (scope tree tiap file) | sync lambat di repo besar | Scope tree hanya untuk file yang hash berubah (sudah ada di [`sync_cache`](../src/commands/sync.rs)). Ukur dengan `--stats`. Simpan L1 hasil di DB. |
| Migrasi v2 di DB user lama | `ALTER TABLE` gagal / data lama | `ALTER TABLE ADD COLUMN` di SQLite non-destruktif & instan; kolom baru `NULL`. Uji dengan `cache.db` v1 nyata. |
| Rust: `impl` untuk tipe generik / crate lain | method lookup meleset | Cakupan tahap 1 hanya tipe lokal non-generik; sisanya L4. Dokumentasikan. |
| Re-export siklik (barrel A ↔ B) | fixpoint tak berhenti | Batasi iterasi fixpoint (mis. 10), sisa `ReExport` diperlakukan `unknown`. |
| Confidence baru menggeser jumlah edge drastis | graph user berubah "tiba-tiba" | Rilis di balik catatan changelog; sediakan `ambiguity_mode = "recall"` untuk yang mau perilaku lama-ish. |

Keputusan terbuka (diskusikan sebelum M4):

1. Apakah edge **relasi tipe** (`fungsi -> tipe` dari anotasi param) diaktifkan
   default, atau di belakang `--kinds references`? (Bisa menambah kebisingan.)
2. Untuk `local`/`param` yang tidak jadi edge — apakah tetap dicatat sebagai
   metadata (untuk fitur "go to definition" nanti) atau dibuang?
3. Batas kedalaman scope block: catat setiap `{ }` atau hanya fungsi/method/class?
   (Setiap block lebih akurat untuk shadowing tapi lebih banyak baris DB.)

---

## 6. Baseline pengukuran (diisi saat M5)

| Fixture | Precision (before) | Recall (before) | Precision (after) | Recall (after) |
|---|---|---|---|---|
| rust_app | _TBD_ | _TBD_ | | |
| ts_app | _TBD_ | _TBD_ | | |
| py_app | _TBD_ | _TBD_ | | |
| pkg_app | _TBD_ | _TBD_ | | |
| vue_app | _TBD_ | _TBD_ | | |

---

## 7. Glosarium & bacaan lanjut

| Topik | Kenapa relevan | Rujukan |
|---|---|---|
| **Name resolution** | Nama proses inti yang kita tiru: nama → deklarasi | Cari "name resolution compiler" / bab awal buku compiler (Crafting Interpreters, bab "Resolving and Binding") |
| **Lexical scoping** | Aturan "scope dalam menutupi scope luar" = dasar L1 | Crafting Interpreters, "Scope" |
| **SCIP (SCIP Code Intelligence Protocol)** | Format indeks simbol dari Sourcegraph; bentuk `bindings`/`scopes` kita adalah versi mininya | https://github.com/sourcegraph/scip |
| **stack-graphs** | Pendekatan GitHub untuk name resolution lintas file tanpa compiler penuh — arah upgrade "Hybrid" kita | https://github.com/github/stack-graphs |
| **tree-sitter queries** | Cara lebih rapi mengekstrak pola dari AST dibanding `match node.kind()` manual | https://tree-sitter.github.io/tree-sitter/using-parsers#query-syntax |
| **rust-analyzer "hir"** | Contoh resolver Rust sungguhan (jauh lebih besar), berguna untuk intuisi kasus sulit | https://rust-analyzer.github.io/book/contributing/architecture.html |

---

### Lampiran: peta lapisan → file

```text
L0 model data     : src/cache/schema.rs · src/cache/models.rs · src/cache/mod.rs
                    src/analysis/mod.rs (ParsedFile)
L1 scope tree     : src/analysis/{rust,javascript,python}.rs · src/analysis/sfc.rs
                    src/graph/resolve/refs.rs (resolve_in_file)
L2 import binding : src/graph/resolve/imports.rs · src/graph/resolve/mod.rs (module_exports, binding)
L3 tipe receiver  : src/graph/resolve/mod.rs (type_methods) · src/graph/resolve/refs.rs
                    src/analysis/*.rs (params_json, receiver_type, type_expr)
L4 disambiguasi   : src/graph/resolve/refs.rs · src/commands/init.rs (config default)
L5 pengukuran     : tests/resolve_accuracy.rs · tests/fixtures/*/expected-edges.json
```
