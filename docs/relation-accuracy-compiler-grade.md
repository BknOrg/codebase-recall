# Opsi C — Akurasi Setara Compiler (SCIP / Stack-Graphs)

> Dokumen pembanding. Pendekatan **utama** yang dipilih tim adalah **Hybrid**
> ([`relation-accuracy-plan.md`](./relation-accuracy-plan.md)). Dokumen ini
> menjelaskan alternatif **paling ambisius**: menyelesaikan nama seakurat IDE /
> compiler sungguhan.
>
> Gaya penulisan sama: untuk orang yang baru kenal proyek. Istilah dasar dijelaskan
> di §1.2 dokumen Hybrid — buka itu dulu bila belum.

---

## 1. Ringkasan & kapan memilih ini

**Ide:** berhenti menebak. Bangun (atau pinjam) sebuah **name resolver sungguhan**
per bahasa yang menghasilkan **indeks simbol global** — tiap deklarasi punya ID
stabil, tiap penggunaan nama dipetakan **tepat** ke deklarasi yang dimaksud,
persis seperti "Go to Definition" di VS Code. Graph relasi lalu dibangun dari
indeks itu, bukan dari pencocokan string.

**Pilih Opsi C kalau:**

- Akurasi mendekati IDE adalah **syarat**, termasuk untuk `obj.method()`,
  generic, trait/impl, pewarisan class, dan simbol dari dependency eksternal.
- Kamu siap menerima **kompleksitas besar** dan/atau **dependency tambahan**.
- `code-rcl` boleh berubah dari "satu binary kecil" menjadi sesuatu yang lebih
  berat, atau boleh punya mode "akurasi tinggi" opsional yang butuh tool lain
  terpasang.

**Jangan pilih Opsi C kalau:** kamu butuh hasil dalam hitungan minggu, atau
distribusi "unduh satu file, jalan" adalah nilai jual utama produk.

---

## 2. Dua sub-jalur

Opsi C bisa ditempuh dengan dua cara yang sangat berbeda ongkosnya.

### C1 — Bangun resolver sendiri (di dalam `code-rcl`)

Tulis name resolution engine di Rust, memakai AST tree-sitter yang sudah ada.
Komponen:

1. **Symbol index global dengan *moniker* stabil.**
   *Moniker* = string identitas kanonik sebuah simbol yang sama di seluruh
   codebase, mis. `rust . crate mycrate . module cache . struct CacheDb . method open`.
   Ini konsep inti **SCIP** (SCIP Code Intelligence Protocol, dari Sourcegraph).
   Indeks: `moniker -> definisi (file, range, kind, signature)` +
   `moniker -> daftar occurrence (file, range, role: definition/reference/import)`.

2. ***Scope graph*** **gaya `stack-graphs` (GitHub).**
   Alih-alih tabel scope datar, name binding dimodelkan sebagai **graph**: node
   "definisi", node "referensi", node "push/pop symbol" untuk melewati batas
   modul/import. Resolusi sebuah nama = mencari **jalur** dari node referensi ke
   node definisi yang "seimbang" (setiap `push` nama dibalas `pop` nama yang
   sama). Kelebihannya: resolusi lintas file jadi satu algoritma graph yang sama
   untuk semua bahasa; hanya *aturan pembentuk graph* yang beda per bahasa.

3. **Resolver semantik per bahasa** (aturan pembentuk graph + tabel modul):
   - **Rust:** pohon modul (`mod`/`use`/`pub`/`pub(crate)`), visibilitas,
     `impl` (inherent & trait), resolusi trait method, prelude.
   - **JS/TS:** ES module (binding hidup, `export`/`export from`/`export *`),
     CommonJS `require`, resolusi `tsconfig` `paths`, deklarasi ambient `.d.ts`.
   - **Python:** paket & `__init__.py`, import relatif vs absolut, `__all__`,
     `sys.path`-style resolution (disederhanakan ke root proyek).

4. **Inferensi tipe ringan→sedang** (untuk `obj.method()` dan tipe field):
   propagasi tipe lokal, inferensi dari konstruktor (`new Foo()`, `Foo()`,
   `Foo::new()`), tipe parameter & return dari signature, akses field
   (`self.x.y()`), substitusi generic minimal (monomorfik), rantai pewarisan
   class, resolusi `impl Trait for T`.

**Ongkos C1:** ini pekerjaan **berbulan-bulan** dan permukaan perawatan yang
besar selamanya (tiap update grammar/bahasa berpotensi perlu penyesuaian aturan).

### C2 — Konsumsi SCIP dari indexer yang sudah matang

Jangan tulis resolver; **panggil** indexer bahasa yang sudah dipakai Sourcegraph
di produksi, lalu **serap** output SCIP-nya (format protobuf standar):

| Bahasa | Indexer | Kebutuhan |
|---|---|---|
| TypeScript / JavaScript | `scip-typescript` | Node.js + `npm`/`yarn` project (`tsconfig`/`package.json`) |
| Python | `scip-python` | Python + resolusi environment (venv ideal) |
| Rust | `rust-analyzer scip` | toolchain Rust + `Cargo.toml` yang bisa `cargo metadata` |

Alur:

```text
code-rcl sync --accuracy=scip
   ├─ untuk tiap bahasa yang terdeteksi & indexer-nya ADA di PATH:
   │     jalankan indexer -> hasil index.scip (protobuf)
   │     parse index.scip -> { symbols[], occurrences[] }
   │     map moniker SCIP -> node CodeGraph; occurrence reference -> edge
   ├─ untuk bahasa tanpa indexer terpasang:
   │     fallback ke resolver heuristik lama (opsi A/B)
   └─ gabungkan jadi satu CodeGraph
```

**Ongkos C2:** integrasi **beberapa minggu**, TAPI menambah **beban distribusi &
lingkungan**: user harus punya `node` / `python` / `rust-analyzer`, proyek harus
"resolvable" (dependency ter-install), indexer bisa lambat & rakus memori pada
repo besar. Perlu deteksi versi, penanganan error indexer, dan sandbox proses.

---

## 3. Arsitektur & dampak ke `code-rcl`

Sekarang:

```text
sync (tree-sitter walk) -> SQLite (symbols/imports/refs) -> resolve::build (heuristik) -> CodeGraph
```

Dengan **C1**:

```text
sync -> tree-sitter walk -> BUILD SCOPE GRAPH per file -> simpan node/edge scope-graph
                                                                │
resolve::build -> JALANKAN PATH-FINDING di scope graph (lintas file) -> occurrences terselesaikan
              -> + inferensi tipe -> CodeGraph
```

- Tabel DB baru yang jauh lebih kaya: `scope_nodes`, `scope_edges`, `monikers`,
  `occurrences`, `types`. Migrasi besar (bukan sekadar `ADD COLUMN`).
- `resolve/refs.rs` & `resolve/imports.rs` sekarang: **dihapus/diganti** oleh
  mesin path-finding.
- `sync` jadi jauh lebih berat; wajib **index inkremental** (hanya bangun ulang
  sub-graph file yang berubah + tetangga yang terpengaruh).

Dengan **C2**:

```text
sync --accuracy=scip -> spawn indexer eksternal -> parse .scip -> ingest ke SQLite
                     -> resolve::build jadi TIPIS (occurrences sudah terselesaikan)
```

- Tambah dependency runtime (tool eksternal), bukan compile-time.
- `resolve::build` menyusut drastis: tinggal memetakan simbol SCIP ke `Node`/`Edge`.
- Perlu strategi: apa yang terjadi kalau indexer gagal separuh jalan, versi
  bahasa tidak didukung, monorepo multi-`tsconfig`, dll.

**Yang tetap sama di kedua sub-jalur:** bentuk akhir `CodeGraph`
([`src/graph/mod.rs`](../src/graph/mod.rs)) dan semua renderer
([`src/graph/render/`](../src/graph/render)) tidak berubah — hanya cara `edges`
diisi yang berubah.

---

## 4. Milestone (besar)

| # | Isi | Selesai bila |
|---|---|---|
| **M1** | POC dua sub-jalur untuk **satu** bahasa (mis. TypeScript): C1 mini scope-graph + path-finding vs C2 jalankan `scip-typescript` & parse. Ukur akurasi & effort. **Putuskan C1 vs C2.** | Ada angka precision/recall + estimasi effort untuk masing-masing; keputusan tertulis. |
| **M2** | (jalur terpilih) Skema indeks: `monikers` + `occurrences` (+ `scope_nodes`/`scope_edges` bila C1). Ingest/build untuk 1 bahasa. | `sync` menghasilkan indeks; `CodeGraph` untuk fixture 1-bahasa 100% benar. |
| **M3** | Resolusi lintas file penuh untuk bahasa itu (import/export/re-export/namespace, module tree). | Fixture barrel berlapis & re-export siklik benar. |
| **M4** | Inferensi tipe untuk `obj.method()`: konstruktor, anotasi, field, return type; (Rust) trait/impl; (JS/TS/Py) hierarki class. | Fixture method-call lintas tipe benar; generic monomorfik benar. |
| **M5** | Bahasa sisanya (Rust, Python[, Vue/Svelte lewat virtual source]). Fallback heuristik untuk bahasa tanpa dukungan/indexer. | Semua fixture; mode campuran (satu repo banyak bahasa) benar. |
| **M6** | Harness akurasi (precision/recall/ambiguous-rate) + anggaran performa `sync` + index inkremental + dokumentasi kebutuhan lingkungan. | CI akurasi & CI performa hijau; `README` mendokumentasikan syarat. |

Perkiraan: **C1 berbulan-bulan**; **C2 ~4–8 minggu** integrasi + biaya dukungan
lingkungan berkelanjutan.

---

## 5. Kelebihan & kekurangan

**Kelebihan**

- Akurasi tertinggi dari ketiga opsi — mendekati IDE. `obj.method()`, generic,
  trait/impl, pewarisan, simbol dependency eksternal: tertangani.
- (C2) memanfaatkan mesin yang sudah teruji di skala besar (Sourcegraph).
- Membuka fitur turunan berkualitas: "find references", "call hierarchy",
  "dead code" yang benar.
- Output SCIP bisa diekspor/diinteroperasikan dengan tool lain.

**Kekurangan**

- **C1:** usaha & perawatan sangat besar; pada dasarnya membangun sebagian
  front-end compiler untuk 3+ bahasa.
- **C2:** `code-rcl` tidak lagi "unduh satu file, jalan" — butuh `node` /
  `python` / `rust-analyzer` dan proyek yang dependency-nya ter-install; indexer
  lambat/berat di repo besar; permukaan kegagalan lingkungan yang luas.
- `sync` jauh lebih lambat; wajib index inkremental yang benar (kompleks).
- Migrasi skema besar; `resolve/refs.rs` & `resolve/imports.rs` dibongkar.
- Overkill bila kebutuhan sebenarnya cuma "graph relasi yang cukup akurat untuk
  navigasi & review".

---

## 6. Perbandingan tiga opsi

| Kriteria | A: Pragmatis | **C: Setara compiler (dokumen ini)** | B: Hybrid (dipilih) |
|---|---|---|---|
| Perubahan skema | minimal (kolom `refs`) | index besar / eksternal | sedang (`scopes`+`bindings`) |
| Usaha awal | ~1–2 minggu | berbulan-bulan (C1) / minggu + beban toolchain (C2) | ~3–5 minggu |
| Akurasi `foo()` bebas | tinggi | sangat tinggi | tinggi |
| Akurasi `obj.method()` | sedang (self/Path saja) | sangat tinggi | sedang→tinggi |
| Dependency eksternal | tidak ada | `node`/`python`/`rust-analyzer` (C2) | tidak ada |
| Biaya `sync` | ~sama | jauh lebih berat | sedikit naik |
| Filosofi "1 binary kecil" | terjaga | pecah (C2) / berat (C1) | terjaga |
| Jalur upgrade ke compiler | rework model data | sudah di sana | tinggal isi lebih lengkap |
| Risiko | rendah | tinggi | sedang |

Detail Opsi A: [`relation-accuracy-pragmatic.md`](./relation-accuracy-pragmatic.md).
Detail Opsi B (dipilih): [`relation-accuracy-plan.md`](./relation-accuracy-plan.md).

> **Kenapa Hybrid dipilih:** ia memberi mayoritas keuntungan akurasi Opsi A
> dengan biaya sedang, sambil menyimpan data (`scopes`/`bindings`) dalam bentuk
> yang **sudah** menyerupai indeks SCIP — sehingga bila suatu saat butuh Opsi C,
> yang dikerjakan adalah *mengisi indeks lebih lengkap*, bukan membangun ulang
> pipeline dari nol.

---

## 7. Glosarium & bacaan

Istilah dasar: §1.2 [`relation-accuracy-plan.md`](./relation-accuracy-plan.md).

| Topik | Kenapa relevan | Rujukan |
|---|---|---|
| **SCIP** | Format & konsep *moniker* / occurrence yang jadi tulang punggung Opsi C | https://github.com/sourcegraph/scip |
| **stack-graphs** | Model "name binding = graph reachability" untuk C1 | https://github.com/github/stack-graphs |
| **tree-sitter-stack-graphs** | Kerangka menulis aturan scope-graph di atas grammar tree-sitter (TS/JS/Python tersedia) | https://docs.rs/tree-sitter-stack-graphs |
| **scip-typescript** | Indexer TS/JS untuk sub-jalur C2 | https://github.com/sourcegraph/scip-typescript |
| **scip-python** | Indexer Python untuk C2 | https://github.com/sourcegraph/scip-python |
| **rust-analyzer `scip`** | `rust-analyzer scip` menghasilkan index SCIP untuk crate Rust | https://rust-analyzer.github.io |
| **rust-analyzer architecture (`hir`, name resolution)** | Gambaran resolver Rust sungguhan bila menempuh C1 | https://rust-analyzer.github.io/book/contributing/architecture.html |
