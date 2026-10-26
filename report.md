# Laporan perbaikan dan peningkatan code-rcl

> Pembaruan implementasi: 1 November 2026. Bagian **Graph, integrasi agent, dan thread otomatis** mencatat perubahan terbaru. Angka benchmark dan keterangan default dalam bagian historis tetap mengikuti konfigurasi pada saat pengujian tersebut.

> Pembaruan checkout historis: 28 Oktober 2026. Bagian **Embedding dari snapshot source** dan **Perbaikan ranking hybrid** mencatat implementasi pada tanggal tersebut; bagian validasi berikutnya merekam baseline sebelum perubahan ranking. Bagian setelah **Arsip hasil penggunaan tool** mempertahankan laporan sebelumnya sebagai bukti historis, bukan daftar masalah yang semuanya masih terjadi. Tiga dokumen pengujian historis yang dahulu dirujuk tidak tersedia dalam checkout ini; isinya tidak direkonstruksi.

Tanggal pengujian awal: 27 Oktober 2026  
Pembaruan laporan: 28 Oktober 2026  
Versi yang diuji: `codebase-recall 0.10.0`  
Repo pengujian: BknDb, HEAD `509f9414adc0e312354965b584cd3e165d02a08e`

## Graph, integrasi agent, dan thread otomatis — 1 November 2026

Build checkout saat ini melaporkan paket `0.9.0`; angka versi pada pengujian
historis di bawah tidak diubah. Implementasi ini tidak memasang binary global
atau menghapus integrasi pada profil agent pengguna.

| Permintaan | Implementasi dan bukti |
| --- | --- |
| Komposisi highlight graph | Renderer bersama memakai hover → selection terlihat → tanpa fokus. Node terkait mempertahankan warna kategori/subsystem, node lain abu-abu redup. Fokus biru, outgoing merah, incoming biru, panah source → target. Relasi dua arah memakai jalur paralel agar tidak saling menutupi. Label tetangga dan panah aktif bertahan pada zoom kecil. |
| Setup seragam | `setup --workspace` dan `setup --global --target codex` memakai resolver yang sama dengan target Claude/Antigravity; Gemini menjadi alias. Native TOML/JSON, argumen root workspace absolut, dan preflight sebelum perubahan diterapkan. Status kepercayaan Codex tidak diubah. |
| Uninstall | Command baru dengan selector scope/target/komponen yang sama. Manifest v1 mencatat pemilik resource. Resource bersama dipertahankan sampai pemilik terakhir; legacy target tunggal konservatif. Hanya bundle dikenal, entry MCP, serta instruction/hook bertanda yang dibersihkan. File tambahan pengguna dipertahankan. `setup --remove` tetap tersedia untuk instruction/hook lama. |
| CPU otomatis | Default dan konfigurasi proyek ini menjadi `threads = 0`; runtime memakai `available_parallelism()` dengan fallback satu. Override 1–64 tetap valid, hasil auto tidak dipotong empat/64. Session cache memakai jumlah efektif; status/progress melaporkan konfigurasi dan jumlah efektif. Model, chunking, batching, ranking, namespace vector, dan schema tetap. |

Suite default: **261 lulus, 12 ignored**; tanpa default features:
**255 lulus, 6 ignored**. Kedua Clippy `--all-targets -- -D warnings`, format,
dan delapan kelompok pemeriksaan renderer Node lulus. Dua belas tes integrasi
setup/uninstall mencakup target/scope, alias, idempotensi, konfigurasi lain dan
komentar TOML, inline table, parsing rusak sebelum penulisan, konflik flag,
resource bersama, legacy, custom skill directory, serta file tambahan pengguna.
Seluruh instalasi/penghapusan pengujian memakai home/workspace sementara.

Pemeriksaan browser pada fixture 13 node/18 edge mencakup tema gelap/terang,
HTML offline dan `serve`, hover, selection setelah pointer keluar, reset,
relasi dua arah, pencarian, filter, expand/collapse, subsystem, legend, fit,
zoom, drag, panel source, context menu, serta focus depth 2 → 1 → 2 dan clear.
Sesi yang diperiksa tidak menghasilkan warning/error browser. Screenshot lokal:
`target/graph-validation-20261101/graph-dark.jpg`, `graph-light.jpg`, dan
`graph-offline.jpg`. Ini bukan pengujian end-to-end menyeluruh setiap aksi graph.

Validasi numerik memakai Gemma 1 300M Q4 lokal, 96 input tetap, batch empat,
dan tiga pengulangan hangat per konfigurasi. Semua input/identitas sama;
vector finite/ternormalisasi dan cosine minimum 768/256 dimensi **1,0**
(ambang ≥0,9999). Median inferensi 4 thread **26,163 s**, auto **23,941 s**,
atau waktu sekitar **8,49% lebih rendah**. Run auto adalah 22,650 / 23,941 /
37,794 s; variasi besar dan urutan 4 lalu auto membatasi interpretasi.
Pemuatan model/tokenisasi dicatat terpisah. Angka ini bukan cold indexing penuh
dan tidak menjanjikan penggunaan CPU 100% atau percepatan pada proyek lain.

Auto terdeteksi sebagai **12 thread** pada CPU Windows pengujian. Warm sync
4 → auto mempertahankan **4.030 chunk terindeks**, 172 file unchanged,
**0 inference** dan tidak memuat model untuk inference. Source corpus diperiksa
byte demi byte sebelum/sesudah; cache asal, konfigurasi/cache proyek utama,
dan snapshot operasi yang dilindungi tidak berubah selama validator.

Pada 32 query lama × 768/256 dimensi (**64 perbandingan**), serta 112 query
corpus × semantic/hybrid (**224 perbandingan**), Hit@1, Hit@5, MRR dan nDCG@5
tidak turun pada satu pun query dengan toleransi `1e-6`. Seluruh urutan hasil
corpus identik. Corpus ini memakai vector yang sudah terindeks: hasil tersebut
memverifikasi inference query dan reuse, bukan re-embedding penuh corpus.
Candidate cap produksi tetap berlaku; tidak ada pemotongan transport karena
budget respons. Baseline CRLF lama tidak dipakai sebagai pembanding baru.

Tiga tes model nyata tambahan lulus: session auto dan override jumlah efektif
memakai runtime yang sama; CLI setup/index/search/update dan warm sync saat
berpindah auto ↔ empat thread; serta kontrak indexing/search semantic/hybrid
MCP pada batching length. Tes mock terpisah membandingkan identitas chunk dan
byte vector pada batching file maupun length saat thread berubah. Kegagalan
awal pada penyisipan field TOML dalam kode tes CLI diperbaiki dan tes tersebut
dijalankan ulang sampai lulus; tidak memerlukan perubahan runtime produksi.

Rincian, fingerprint binary/model/input, delta setiap query, dan batas pengujian
disimpan dalam [bukti validasi](docs/IMPLEMENTATION-VALIDATION-2026-11-01.json).
Artefak mentah ada di `target/auto-threads-validation-20261101/`, termasuk
`corpus-rankings.json.gz`; runner dapat diulang melalui
`scripts/validate-auto-threads.py`. Pengujian ini hanya membuktikan perilaku
pada Windows/CPU yang diuji; Linux/macOS tidak dijalankan di sesi ini.

## Uji cold indexing Ollama otomatis — 1 November 2026

Uji terisolasi pada snapshot proyek code-rcl (209 file, 2.654 chunk) selesai
untuk satu run Ollama dan satu baseline, sesuai pilihan pengguna. Ollama tidak
diberi `num_thread` atau `num_gpu`; thread/perangkat dipilih server otomatis.
Runtime melaporkan `llamacpp`, konteks 2.048 dan VRAM 0. Batch tetap empat.

| Pipeline | Cold indexing total | Inferensi |
| --- | ---: | ---: |
| FastEmbed, Gemma 1 300M Q4, 4 thread | 18 menit 32,66 detik | 1.088,551 s |
| Ollama, Gemma 2 270M Q8_0, otomatis | 28 menit 14,35 detik | 1.680,230 s |

**Ollama membutuhkan waktu 52,28% lebih lama pada uji ini**, meskipun batas empat
thread telah dihapus. Ini bukan perbandingan engine saja: generasi model,
kuantisasi, bobot dan tokenizer inference berbeda. Jumlah thread aktual maupun
pemakaian maksimum hardware tidak diukur. Satu pasangan bukan median berulang;
urutan, cache OS, suhu dan beban sistem tidak dikontrol untuk klaim umum.

Kedua run menghasilkan input, identitas chunk, source range dan canonical hash
identik; 742 batch, reuse 0, blocked 0. Verifikasi source, readback BknDb, serta
vector finite/normalisasi lulus. Pada fixture terpisah 80 dokumen/32 query,
top-1 768 dimensi berubah 31/32 → 30/32 dan top-1 256 dimensi 30/32 → 27/32.
Top-5 tetap 32/32. Metrik per query turun pada dua query 768 dimensi dan lima
query 256 dimensi; ada satu/dua query yang membaik. Fixture ini bukan corpus
BknDb 172 file/112 query dan tidak membuktikan kualitas umum model.

Default, konfigurasi/cache proyek utama dan binary global tetap. Ollama masih
harness eksperimen, belum provider CLI/MCP produksi. Benchmark grid CPU yang
sebelumnya dihentikan tidak dilanjutkan atau dipromosikan. Format, Clippy,
build release, satu regresi konfigurasi dan empat tes gate benchmark lulus.
Rincian fase, RAM, batas pengukuran dan reproduksi tersedia pada
[laporan eksperimen Ollama](docs/OLLAMA-INDEXING-EXPERIMENT.md) serta
[bukti JSON](docs/OLLAMA-INDEXING-RESULTS.json). Run empat thread yang terputus
dan percobaan gagal sebelumnya dipertahankan terpisah sebagai jejak, tanpa
memasukkannya ke perbandingan cold indexing penuh.

## Uji singkat FastEmbed 4 versus 12 thread — 1 November 2026

Dengan Gemma 1 300M Q4 dan batch 4 yang sama, median inferensi 96 input tetap
(tiga pengulangan setelah warmup) turun dari **24,94 detik pada 4 thread** ke
**20,75 detik pada 12 thread**, atau pengurangan waktu **16,82%**. CPU proses
rata-rata seluruh run naik dari 27,51% ke 68,87% kapasitas 12 thread logis;
angka ini mencakup load/warmup/ekspor dan tidak membuktikan penggunaan 100%.

Input dan numerik vector 768/256 dimensi lulus verifikasi (cosine ≥0,9999).
Ini uji singkat terpisah, bukan kelanjutan grid CPU lama atau pengulangan
indexing penuh 209 file. Tidak mengukur ranking atau menetapkan waktu indexing
penuh dengan 12 thread. Default, cache/config utama dan binary global tetap.
[Rincian metode](docs/FIRST-INDEX-CPU.md) dan
[bukti JSON](docs/FASTEMBED-THREADS-QUICK-RESULTS.json) menyimpan batas dan hasil.

## Eksperimen Jina Embeddings v2 Base Code di CPU

Pengujian terisolasi pada 28 Oktober 2026 memakai release test checkout
`codebase-recall 0.9.0`, FastEmbed Rust 7.5.0 dan ONNX Runtime CPU pada Intel
Core i3-1220P. Forward pass berjalan native; Python hanya mengunduh artefak,
mengorkestrasi proses dan menghitung metrik. Model Jina FP32 dan INT8 berhasil
dijalankan dengan mean pooling, normalisasi L2, empat thread dan batas 512 token.

Corpus tetap berisi 172 file dan 112 query, dengan 80 tuning dan 32 evaluation.
Seluruh 224 urutan semantic/hybrid baseline Gemma cocok dengan hasil produksi.
Ketiga varian menyelesaikan seluruh query, dengan pemeriksaan byte source
sebelum/sesudah lulus. FP32 dan INT8 membentuk 3.957 chunk dengan teks identik;
Gemma memakai kembali 4.030 vector baseline yang sudah valid.

| Model | Top-1 hybrid evaluation | Top-5 hybrid evaluation | Median inferensi query warm | Peak working set proses |
| --- | ---: | ---: | ---: | ---: |
| Gemma 1 Q4 | 27/32 | 30/32 | 26,81 ms | 874,43 MiB |
| Jina FP32 | 17/32 | 25/32 | 26,66 ms | 1.487,24 MiB |
| Jina INT8 | 17/32 | 25/32 | 11,14 ms | 555,20 MiB |

INT8 sekitar 2,4× lebih cepat pada median inferensi query mesin ini, tetapi
kualitas retrieval turun. Top-1 hybrid pada 10 query evaluation Indonesia
berubah dari 7/10 pada Gemma menjadi 0/10 pada kedua Jina; pada 22 query Inggris,
dari 20/22 menjadi 17/22. Kedua Jina kehilangan 10 top-1 evaluation tanpa
peningkatan top-1 evaluation. Ini hasil pipeline dan seed dataset ini, bukan
penilaian akurasi umum seluruh penggunaan model.

Pada split tuning, query `roll back writes when a transaction closure fails`
memperbaiki posisi method Rust `write_tx` dari 73 pada Gemma menjadi 7 pada
FP32 dan 8 pada INT8; keduanya masih di luar lima hasil pertama. Query campuran
dengan identifier `write_tx` tetap pada posisi 1. Query/corpus tersebut berbeda
dari contoh historis rank 44. Beberapa kandidat binding/tes di atas method tetap
relevan terhadap query yang tidak menyebut Rust secara eksplisit.

**Default Gemma dipertahankan.** Indeks kerja dan binary global tidak diganti.
Vector antarmodel tidak kompatibel walaupun dimensinya sama. Timing di atas
tidak mencakup sync, pencarian vector atau transport MCP. Detail konfigurasi,
biaya indexing, batas eksperimen dan reproduksi ada dalam [evaluasi Jina](docs/JINA-EMBEDDING-EVALUATION.md),
[bukti JSON](docs/JINA-EMBEDDING-RESULTS.json), serta [672 ranking top-100](docs/JINA-EMBEDDING-RANKINGS.json.gz).

## Embedding dari snapshot source — schema 11

Source sync kini menyimpan byte source lengkap beserta hash dalam tabel `source_snapshots` pada `cache.bkndb`, terikat ke ID file yang stabil. Snapshot, metadata file, analisis dan invalidasi embedding dicommit bersama per file. Batching file dan length membentuk chunk dari snapshot yang diverifikasi identitas/hash/UTF-8-nya; pembacaan repo tetap digunakan untuk pemeriksaan freshness. Source sync tetap dapat berjalan tanpa model atau tokenizer.

Migrasi **schema 10 → 11** menambahkan tabel secara atomik dan mempertahankan graph serta vector. Warm sync mengisi atau memperbaiki snapshot tanpa reparse file unchanged. Audit corpus membuktikan seluruh baris graph dan byte dari **4.030 vector pada 172 file** tetap identik. Perbandingan terhadap binary yang dibekukan tepat sebelum refactor menghasilkan **336 pencarian pada 112 query**, dengan urutan hasil dan metrik relevansi yang seluruhnya identik. Model, pipeline, tokenisasi, default batching dan perbaikan hybrid sebelumnya dipertahankan.

Verifikasi akhir lulus: **250 tes default**, **245 tes tanpa default features**, format, Clippy, build, dua tes eksplisit model nyata CLI/MCP, serta status-lock/Ctrl+C/force-interrupt/resume nyata pada kedua batching. Suite biasa tetap mencatat 10/6 ignored; pengujian yang dijalankan eksplisit dipisahkan dalam bukti.

Snapshot menambah payload source logis **1.367.840 byte (sekitar 1,30 MiB)** pada corpus ini. File database teralokasi tetap 129,01 MiB pada sampel tersebut. Source sync dengan backfill pertama terukur 3,976 detik versus baseline 3,147 detik; no-op embedding sync warm kedua 0,422 versus 0,413 detik. Sampel lokal ini bukan bukti speedup atau regresi umum. Detail cold indexing fixture, peak working set, kondisi dan fingerprint ada di [validasi snapshot source](docs/SOURCE-SNAPSHOTS.md) dan [bukti JSON](docs/SOURCE-SNAPSHOT-RESULTS.json).

Build release metode snapshot tersedia di checkout; binary global tidak diganti pada implementasi ini. Pemasangan global menjadi langkah terpisah. Binary lama dapat membangun ulang cache schema 11, sehingga jangan memakainya pada cache baru jika ingin mempertahankan indeks.

## Perbaikan ranking hybrid

Perubahan produksi kini menangani query campuran identifier/perilaku. Sebelumnya hybrid mencari seluruh kalimat sebagai nama deklarasi, sehingga `write_tx roll back writes when a transaction closure fails` menghasilkan nol kandidat lexical. Bila pencarian seluruh query kosong, hybrid sekarang mencoba nama exact dari identifier kode di awal query atau identifier sederhana dalam backtick. Kata/nama anggota yang sekadar muncul di tengah prosa tidak diekspansi. Filter tetap berlaku sebelum candidate limit; `match_mode=exact/prefix` tetap mencari query utuh.

Pada binary hasil build checkout, method `Db.write_tx` untuk query campuran tersebut naik dari **rank 36 ke rank 1**, dengan empat kandidat lexical dan tiga kandidat yang terdapat pada kedua cabang. Tidak ada perubahan model, pipeline, rumus RRF, scope implisit, atau kebutuhan reindex. Pencarian lexical biasa dan semantic-only tidak berubah. Query perilaku tanpa identifier masih memiliki keterbatasan baseline.

Evaluasi produksi pada **112 query** (80 query awal ditambah 32 query container) selesai dengan postcheck byte source lulus: tiga query membaik dan tidak ada penurunan hit@1, hit@5, MRR atau nDCG per query. Top-1/top-5 tuning berubah dari 47/80 dan 74/80 menjadi 49/80 dan 75/80; held-out dari 26/32 dan 30/32 menjadi 27/32 dan 30/32. Seluruh query container mempertahankan metriknya. Ini hasil seed benchmark pada corpus yang sama, bukan klaim akurasi umum.

Suite setelah perbaikan lulus **240 tes default**, **235 tes tanpa default features**, format check, Clippy, serta uji MCP model nyata yang kini memeriksa kontribusi identifier pada query campuran. Binary release baru juga mengembalikan target rollback pada rank 1. [Bukti perbaikan ranking](docs/HYBRID-IDENTIFIERS-RESULTS.json) dipisahkan dari bukti baseline validasi sebelumnya.

Konfigurasi PATH dan MCP lokal memilih `C:\Users\SHI-CHAN\.cargo\bin\code-rcl.exe`. Mengubah source atau membangun `target/` tidak otomatis memperbarui executable tersebut, dan proses MCP yang sudah berjalan perlu dimulai ulang untuk memuat executable baru. Detail perilaku dan prosedur evaluasi ada pada [catatan perbaikan hybrid](docs/HYBRID-IDENTIFIERS.md).

Setelah persetujuan pengguna, binary release baru sudah dipasang pada path tersebut. Hash binary terpasang cocok dengan release yang diuji; backup binary lama diverifikasi dan disimpan di `C:\Users\SHI-CHAN\.cargo\bin\code-rcl.exe.running-before-hybrid-20261028-b9b7c95e.bak`. Pengujian langsung melalui perintah PATH `code-rcl` mengembalikan target query campuran pada **rank 1**. Proses MCP yang sudah berjalan masih perlu dimulai ulang; penggantian file tidak mengganti kode dalam memori proses lama. Receipt instalasi ada pada bagian `installation` di bukti JSON.

## Validasi source sebelum perbaikan ranking

Proyek ini adalah source `codebase-recall 0.10.0` sendiri. Pemeriksaan kali ini mencakup implementasi dan pengujian checkout lokal, berbeda dari sesi historis yang hanya memakai CLI/MCP. Ada banyak perubahan lokal sebelum validasi ini; keberadaan perbaikan tidak berarti semuanya dibuat dalam sesi ini.

Sejumlah rekomendasi lama sudah tersedia: filter eksplisit, diagnostik parser pada cache hangat, snapshot operasi di luar lock database, progress/cancellation, pemangkasan output yang mempertahankan identitas, evaluasi retrieval, dan profiling. Hasil uji serta batasnya dicatat dalam [catatan validasi](docs/CODE_RCL_VALIDATION_2026-10-28.md). Status “lulus” hanya berlaku pada skenario yang dicatat, bukan jaminan seluruh workload.

Pada tahap validasi awal, **238 tes default** dan **233 tes tanpa default features** lulus, bersama format check, Clippy, build tanpa default features, lima helper Python, dua tes eksplisit model nyata CLI/MCP, serta status-lock/cancellation/resume nyata pada mode file dan length. Suite biasa tetap melaporkan 9/5 ignored; detail pengujian yang belum dijalankan ada pada catatan validasi. Tahap awal hanya menambah regresi dan bukti; perbaikan ranking produksi ditambahkan kemudian sebagaimana dijelaskan di atas.

### Matriks temuan dan tindak lanjut

| Temuan / bagian lama | Implementasi atau bukti sekarang | Verifikasi dan pekerjaan tersisa |
| --- | --- | --- |
| Evaluasi hanya enam pertanyaan (§3.1) | Corpus beku 172 file; 80 query berlabel, split 56 tuning/24 evaluasi; tambahan 32 query container | Integritas/hash dan pemisahan task diuji; ini seed benchmark, bukan akurasi umum |
| Scope bahasa/path/jenis kode (§3.2, §6.3) | Selection eksplisit dibagi oleh CLI/MCP search dan report | Uji parity dan filter sebelum candidate limit; tidak menyimpulkan scope otomatis |
| Duplikasi container/anggota (§3.3) | Semantic memilih chunk terbaik per simbol; overlap antar-simbol baru eksperimen offline | Overlap memberi manfaat kecil pada tuning; belum dipromosikan untuk query container |
| Konteks chunk (§3.4) | Input memuat identitas/owner/signature/doc dan source range; compact diuji terpisah | Compact menurunkan kualitas held-out container; tetap harness-only |
| Hybrid kosong pada kalimat perilaku (§3.5) | Exact/fuzzy lexical dan RRF tersedia; statistik tiap cabang tersedia | Ekspansi umum ditolak; perbaikan exact identifier eksplisit kini tersedia untuk query campuran |
| Rollback / reranking (§3.6) | Kandidat top-100 baseline tersimpan; lihat diagnosis di bawah | Query campuran diperbaiki; query perilaku tanpa identifier dan model/reranker alternatif tetap pekerjaan lanjutan |
| Parser Kotlin (§4.1) | Normalisasi komentar untuk ambiguitas grammar, dengan lokasi source asli | Regresi UniFFI dan penolakan syntax rusak; bukan fallback graph presisi |
| File terabaikan (§4.2) | Inspeksi path menunjukkan aturan Git ignore tanpa memindai pohon ignored | Pengujian aturan ignore dan penolakan parent traversal |
| Freshness (§4.3) | Invalidasi source/config, transaksi per file, reuse dan pemulihan | Regresi edit/delete/restart/config/race/cancel; ditambah rename file dan simbol setelah restart |
| Warning antar-run (§4.4) | Diagnostik disimpan menurut source hash dan revision analyzer | Pengujian cache hangat, reparse, perbaikan syntax, dan penghapusan diagnostik |
| Lock, progress, cancellation (§5.1) | Snapshot dan OS lock operasi terpisah; status busy bertanda freshness unverified | Uji heartbeat, lock, interrupted snapshot, CLI/MCP progress, dan pengujian model nyata |
| Profiling dan incremental (§5.2–5.3) | Timing fase, counter status atomik, no-op tanpa model load; batching length opsional | Bukti performa lokal tersedia; length gagal gate numerik dan tidak menjadi default |
| Explain budget kecil (§6.1) | Relasi dipotong sebelum identitas; jumlah relasi asli dipertahankan; minimum yang tak muat menjadi error eksplisit | Regresi unit dan MCP end-to-end 4 KB dengan 80 caller |
| Partial versus truncation (§6.2) | `metadata.reasons` membedakan source, coverage, candidate/result limit dan response budget | Kontrak budget, parser health, dan hasil parsial diuji |

### Diagnosis rollback sebelum perbaikan hybrid

Target adalah method closure `Db.write_tx` di `crates/bkndb-core/src/db/write_tx.rs`. Deklarasi bernama `write_tx` di `db/mod.rs` adalah **modul**, bukan method target. Identitas evaluasi harus memakai path dan nama, bukan nama saja.

Evaluasi ulang checkout (80 query / 240 panggilan mode, postcheck byte source lulus), artefak baseline 80 query, dan run corpus full pada benchmark indexing memberikan posisi berikut untuk semantic dan hybrid tanpa scope:

| Query | Rank method target dalam top-100 semantic / hybrid |
| --- | --- |
| `write_tx` | 9 / 2 |
| `roll back writes when a transaction closure fails` | 73 / 73 |
| `batalkan perubahan jika closure transaksi gagal` | Tidak masuk / tidak masuk |
| `write_tx roll back writes when a transaction closure fails` | 36 / 36 |

Untuk kalimat Inggris, lima teratas adalah `rollback` dan container `BknDbTransaction` pada FFI, wrapper Python, serta `Rollback`/`txRollback` pada Go. Fungsi target tersedia dalam kandidat, sehingga ranking layak diteliti. Untuk kalimat Indonesia, target tidak tersedia dalam top-100: reranking kumpulan itu saja tidak cukup. Ini diagnosis dua query, bukan bukti penyebab internal model.

Uji scope semantic pada build checkout ini menghasilkan rank 42 dengan filter Rust dan rank 3 dengan path `crates/bkndb-core/src/db/**` untuk kalimat Inggris. Kalimat Indonesia menghasilkan rank 66 dengan filter Rust dan rank 2 dengan path yang sama. Scope mengubah himpunan kandidat; hasil ini tidak dihitung sebagai peningkatan ranking default.

Angka baseline lama tetap memiliki caveat: final postcheck berbasis text gagal akibat normalisasi CRLF. Angka tersebut tidak diubah menjadi “lulus”. Bukti corpus benchmark yang terpisah memiliki `source_bytes_verified:true`; run validasi baru juga lulus dengan runner byte-preserving dan workspace terisolasi. Ringkasan dan fingerprint tersedia dalam [bukti validasi JSON](docs/CODE_RCL_VALIDATION_2026-10-28.json).

### Evaluasi dan performa yang sudah tersedia

- [Evaluasi retrieval](docs/RETRIEVAL-EVALUATION.md) mencatat seed held-out 24 query: semantic top-1/top-5 18/24 dan 22/24; hybrid 19/24 dan 22/24. Evaluasi ulang checkout mempertahankan angka ini, dengan final postcheck byte source lulus. Corpus/query berbeda dari enam pertanyaan historis, sehingga angka itu bukan kenaikan langsung dari 3/6 atau 5/6.
- Eksperimen token expansion menurunkan held-out hybrid top-1 dari 19/24 menjadi 11/24 dan top-5 dari 22/24 menjadi 18/24. Overlap hanya menaikkan tuning top-1 dari 32/56 menjadi 33/56; held-out top-1/top-5 tetap. Keduanya tidak diaktifkan dalam retrieval produksi.
- [Benchmark indexing](docs/INDEXING-BENCHMARK.md) dan ringkasan artefaknya mencatat file/full 1544,71 detik dan length/full 1137,40 detik pada corpus 172 file/4030 chunk: perbaikan lokal 26,4%. Minimum cosine 0,999439 di bawah ambang 0,9999, sehingga default tetap **file/full**.
- Compact mengurangi actual inference tokens 12,2% dan waktu 7,4% terhadap full/length, tetapi held-out class/type top-1 turun dari 7/8 ke 6/8. Compact tetap eksperimen, bukan perbaikan produksi yang lulus.

Tidak ada perubahan model, pipeline produksi, schema cache, API publik, atau default batching pada validasi ini. Tidak ada bukti yang membenarkan pergantian model segera. Prioritas berikutnya tetap kualitas ranking dengan label yang sesuai intent, pengambilan kandidat untuk kasus Indonesia, dan eksperimen yang lolos evaluasi terpisah.

## Arsip hasil penggunaan tool — 27–28 Oktober 2026

Seluruh hasil dan kalimat “belum diuji” berikut adalah status pada pengujian historis. Untuk implementasi dan verifikasi checkout sekarang, gunakan bagian validasi di atas. Kesimpulan historis bahwa source internal belum diperiksa berlaku pada sesi lama saja.

## Ringkasan historis

code-rcl berhasil membantu pencarian deklarasi, pemetaan relasi, dan pencarian kode berdasarkan makna. Fitur embedding berfungsi, termasuk satu contoh pertanyaan bahasa Indonesia. Pengujian perubahan source juga menunjukkan bahwa chunk lama tidak disajikan sebagai hasil valid sebelum embedding diperbarui.

Pengujian ulang menunjukkan cakupan indeks sudah lengkap dan metadata/progres/scope meningkat. Prioritas yang masih relevan adalah evaluasi relevansi, ranking retrieval, pengelompokan chunk serupa, kontribusi lexical pada hybrid, dan profiling latensi. Masalah parser Kotlin yang terlihat pada baseline tidak lagi tampak pada indeks saat ini. Perilaku status selama sync panjang dan output explain berbudget kecil belum diuji ulang.

Model embedding mungkin ikut membatasi kualitas pencarian, tetapi belum terbukti sebagai penyebab utama. Bukti saat ini belum mengharuskan penggantian model.

**Belum ditemukan bug fungsional baru pada inference, retrieval, atau invalidasi embedding dalam kasus yang diuji.** Ada keterbatasan yang teramati dan beberapa dugaan yang perlu diverifikasi. Laporan ini tidak mengaudit source internal code-rcl; penyebab internal dan desain implementasi yang disarankan di bawah adalah usulan, bukan hasil pemeriksaan kode tool tersebut.

Temuan bug vector BknDb dari analisis sebelumnya tidak dipakai sebagai bukti bug code-rcl.

## Hasil pengujian tool historis: apa yang meningkat dan apa yang belum

Perbandingan menggunakan enam pertanyaan, fungsi target, limit=5, dan budget output=32 KB yang sama. HEAD repo tetap sama. Corpus embedding bertambah, sehingga ini perbandingan hasil penggunaan tool, bukan eksperimen yang mengisolasi satu perubahan algoritma.

| Aspek | 27 Oktober | 28 Oktober | Penilaian |
| --- | --- | --- | --- |
| Chunk terindeks | 2.449 | 3.621 | Coverage meningkat |
| Status coverage | partial; 1 pending | complete; 0 pending/blocked/invalid | Keterbatasan parser baseline tidak lagi terlihat |
| Sync tanpa source berubah | 2.385 ms | 403 ms | Lebih cepat pada pengukuran ini; masing-masing satu sampel |
| Fungsi target di rank 1 | 3/6 | 3/6 | Belum meningkat |
| Fungsi target dalam top-5 | 5/6 | 5/6 | Belum meningkat |
| Median semantic melalui MCP | 2,278 detik | 2,736 detik | Belum terbukti lebih cepat; kondisi runtime tidak cukup terkontrol untuk menyatakan regresi |
| Median hybrid melalui MCP | 2,480 detik | 2,546 detik | Belum terbukti lebih cepat |

Hit rate enam pertanyaan ini bukan persentase akurasi umum. Satu fungsi kanonis dipilih untuk setiap pertanyaan, padahal beberapa implementasi lain juga relevan.

Temuan yang masih perlu diperhatikan:

- Query rollback default tetap tidak menampilkan fungsi closure Rust `Db.write_tx` dalam top-5. Hasilnya berupa wrapper dan tes yang berkaitan. Query tidak menyebut Rust, sehingga hasil lintas bahasa tidak otomatis salah.
- Pada backup, fungsi Redb turun dari rank 2 ke rank 4 karena dua hasil binding Kotlin ikut masuk. Rank 1 tetap implementasi LSM yang juga relevan. Coverage lebih lengkap tidak otomatis meningkatkan posisi target tertentu.
- Scope Rust mengembalikan backup Redb ke rank 2. Untuk rollback, scope bahasa saja belum cukup; scope path `crates/bkndb-core/src/db/**` menemukan fungsi `write_tx` di rank 3. Ini manfaat filter eksplisit, bukan kenaikan akurasi default.
- Metadata hybrid untuk enam kalimat perilaku tetap menunjukkan 0 kandidat lexical dan 100 kandidat semantic. Urutan hybrid mengikuti semantic; belum ada bukti peningkatan fusion untuk jenis query tersebut.
- Pertanyaan normalisasi dalam bahasa Indonesia tetap menemukan `prepare` di rank 1. Contoh tunggal ini belum mewakili kualitas multilingual secara umum.

Peningkatan operasional yang teramati adalah filter bahasa/path, progress di stderr, alasan truncation terstruktur, statistik kandidat, dan timing fase. Status pada sync panjang belum diuji ulang; jangan menganggap semua keterbatasan lock baseline sudah terbukti selesai.

## Klasifikasi: bug atau kekurangan kualitas?

| Perilaku | Klasifikasi berdasarkan bukti saat ini | Tindak lanjut |
| --- | --- | --- |
| Fungsi tujuan ada tetapi tidak masuk top-5 | Kekurangan relevansi/ranking; bukan otomatis bug fungsional | Periksa kandidat lebih luas, konteks chunk, dan ranking |
| Wrapper/tes mengalahkan Rust ketika query tidak membatasi bahasa | Ambiguitas tujuan dan ranking | Gunakan scope eksplisit; evaluasi intent implementasi versus contoh penggunaan |
| Target turun setelah corpus binding bertambah | Tradeoff ranking dan keragaman hasil | Kelompokkan hasil serupa tanpa membuang binding yang relevan |
| Hybrid sama dengan semantic karena lexical branch kosong | Keterbatasan manfaat hybrid pada query tersebut | Uji query campuran serta kontribusi tiap cabang |
| Latensi MCP beberapa detik | Peluang optimasi; belum terbukti sebagai regresi | Profilkan biaya internal dan end-to-end secara terpisah |
| Filter Rust tetap mengembalikan Python, atau stale chunk diklaim valid | Contoh pelanggaran kontrak yang akan menjadi bug | Belum ditemukan dalam kasus yang diuji; tambahkan regresi |

Pengujian freshness baseline menunjukkan chunk dari file yang berubah ditolak sebelum embedding sync dan hasil baru muncul setelah sync. Tidak ditemukan pelanggaran pada fixture tersebut. Ini tidak membuktikan seluruh skenario rename/delete/concurrency bebas bug.

## Apakah model embedding merupakan penyebabnya?

**Mungkin berpengaruh, tetapi belum dapat dipastikan sebagai penyebab utama.** Model menentukan kemiripan representasi query dan kode; hasil akhir juga ditentukan oleh cakupan indeks, informasi dalam chunk, batas kandidat, filter, fusion, dan penyusunan top-k.

Pada query rollback, istilah dalam wrapper bernama `rollback` mungkin lebih selaras dengan kalimat query daripada fungsi bernama `write_tx`. Ini hipotesis yang masuk akal, bukan penyebab internal yang telah dibuktikan. Belum ada perbandingan model pada corpus dan query yang identik.

| Kemungkinan penyebab | Bukti yang tersedia | Yang belum dibuktikan |
| --- | --- | --- |
| Representasi model kurang membedakan implementasi yang diinginkan | Target closure Rust tidak masuk top-5 default | Posisi target dalam top-100 dan apakah model lain memberi ranking lebih baik |
| Informasi chunk kurang menjelaskan perilaku | Target bernama write_tx; query menyebut rollback | Dampak penambahan/penyederhanaan konteks pada model yang sama |
| Scope query terlalu luas | Path scope membuat target masuk rank 3 | Bahwa scope otomatis tanpa instruksi pengguna akan selalu membantu |
| Kandidat mirip memenuhi top-k | Chunk impl/fungsi dan binding serupa muncul bersama | Manfaat deduplikasi terhadap evaluasi yang lebih luas |
| Model gagal memahami makna secara umum | Tidak didukung hasil saat ini: beberapa target ditemukan tanpa nama fungsi, termasuk contoh Indonesia | Kualitas pada benchmark lintas bahasa dan tipe tugas yang lebih besar |

Model, revision, dimensi dan pipeline pada metadata tetap sama antar-pengujian. Bertambahnya coverage sambil hit rate tetap sama tidak membuktikan model rusak ataupun bebas keterbatasan. Model yang berfungsi tetap dapat menghasilkan ranking yang kurang sesuai tujuan pengguna.

### Rencana diagnosis yang disarankan

1. **Periksa kandidat sebelum mengganti model.** Ambil top-100 untuk kasus rollback dan catat posisi target. Jika target tersedia tetapi tenggelam, prioritaskan ranking/reranking. Jika tidak tersedia, periksa representasi, scope dan tahap pengambilan kandidat.
2. **Uji konteks chunk dengan model tetap.** Bandingkan input saat ini dengan variasi signature, nama lengkap, modul, dokumentasi dan isi fungsi. Ubah satu faktor per eksperimen; konteks yang lebih panjang juga dapat menambah noise.
3. **Uji grouping dan ranking.** Bandingkan hasil per-chunk/per-simbol, perlakuan container versus anggota, dan scope eksplisit. Gunakan label relevansi yang menerima jawaban lintas backend/bahasa ketika pertanyaan memang tidak membatasi scope.
4. **Lakukan perbandingan model yang terkontrol.** Bekukan snapshot source, chunk, query, label relevansi, scope, batas kandidat, metric retrieval, dan aturan fusion/ranking sejauh kompatibel. Masing-masing model harus menggunakan indeksnya sendiri dengan preprocessing yang sesuai dan terversi. Jangan membandingkan model baru pada corpus berbeda lalu mengatribusikan semua selisih kepada model.
5. **Pisahkan kualitas representasi dan sistem retrieval.** Jika tersedia, gunakan pencarian exact pada evaluasi kecil untuk mengurangi pengaruh approximation indeks. Setelah itu uji kembali jalur produksi. Ini usulan eksperimen, bukan klaim tentang backend vector internal code-rcl yang digunakan sekarang.
6. **Nilai kualitas dan biaya bersama.** Bandingkan hit@k/MRR atau nDCG, latency query, throughput indexing, dan peak memory pada set evaluasi terpisah. Pergantian model layak bila perbaikan relevansinya konsisten dan biayanya sesuai kebutuhan.

Belum ada top-100 rollback, model alternatif, reranker, atau eksperimen representasi chunk baru yang dijalankan dalam laporan ini. Langkah-langkah di atas adalah rencana verifikasi. Tidak ada alasan kuat untuk langsung mengganti model sebelum faktor ranking, konteks, dan scope diuji.

## Status rekomendasi historis setelah pengujian ulang

| Rekomendasi baseline | Status terkini |
| --- | --- |
| Perbaiki coverage parser Kotlin | Kendala tidak lagi terlihat; metadata complete dan simbol Kotlin muncul |
| Tambahkan scope bahasa/path | Tersedia dan telah dicoba; subtree membutuhkan pola glob yang sesuai |
| Tambahkan progress dan timing | Terlihat pada run terbaru; status selama sync panjang masih perlu diuji |
| Tingkatkan ranking, grouping, dan manfaat hybrid | Masih relevan; hit rate default belum meningkat |
| Perbaiki explain saat budget kecil | Belum diuji ulang; bukan bug yang dinyatakan masih terjadi pada build sekarang |
| Bandingkan model embedding | Masih usulan; belum ada bukti perbandingan yang membenarkan pergantian |

## Baseline dan usulan awal — 27 Oktober 2026

Bagian bernomor berikut dipertahankan untuk jejak bukti awal. Status rekomendasi terkini mengacu pada bagian pembaruan di atas.

## 1. Dasar pengujian

Pengujian memakai CLI dan MCP pada Windows. Cache source sudah tersedia. Panggilan MCP tetap memverifikasi freshness source; `no_sync` tidak dipakai. Graph yang diperiksa memakai resolusi heuristic, tanpa verifikasi LSP precise.

Model embedding:

- Model: `embeddinggemma-300m-q4`.
- Revision: `5090578d9565bb06545b4552f76e6bc2c93e4a66`.
- Dimensi: 768.
- Pipeline: `symbol-v1-t512-o64-code-retrieval`.
- Setup memverifikasi model lokal yang tersedia; waktu download tidak diukur.

### Hasil indexing

| Pengukuran | Hasil |
| --- | --- |
| Kondisi awal | 1.535 chunk; 71 dari 158 file pending |
| Waktu indexing lanjutan | 546.194 ms, sekitar 9 menit 6 detik |
| Chunk baru | 914 |
| Throughput tahap tersebut | Sekitar 1,67 chunk/detik |
| Kondisi akhir | 2.449 chunk; 1 file pending; status partial |
| Sync tanpa source berubah | 2.385 ms; 0 chunk baru |

Waktu tersebut **bukan indexing dari nol**. Throughput mencakup keseluruhan proses, bukan hanya inference model.

### Hasil pencarian

Enam pertanyaan bahasa Inggris berbasis perilaku ditulis sebelum retrieval. Setiap pertanyaan diberi satu fungsi Rust kanonis sebagai target, kemudian dikirim utuh ke tiga mode dengan limit 5.

| Mode | Median latensi MCP | Target di rank 1 | Target di top-5 |
| --- | --- | --- | --- |
| Lexical | 2,529 detik | 0/6 | 0/6 |
| Semantic | 2,278 detik | 3/6 | 5/6 |
| Hybrid | 2,480 detik | 3/6 | 5/6 |

Angka lexical hanya berlaku untuk enam **kalimat utuh** tersebut. Pada kontrol identifier `pack_vector`, ketiga mode menemukan implementasinya. Tidak tepat menyimpulkan lexical buruk secara umum atau semantic lebih cepat berdasarkan sampel ini.

Penilaian target juga lebih ketat daripada relevansi biasa. Pada pertanyaan backup, implementasi LSM berada di rank 1 dan target Redb di rank 2; keduanya relevan. Pada pertanyaan rollback, wrapper dan tes Python/Go muncul, sementara fungsi closure Rust yang ditargetkan tidak masuk top-5. Pertanyaannya tidak menyebut Rust, sehingga hasil tersebut bukan otomatis jawaban salah.

Satu pertanyaan normalisasi dalam bahasa Indonesia menemukan fungsi `prepare` di rank 1. Sampel tunggal ini belum membuktikan kualitas multilingual secara umum.

## 2. Daftar prioritas

Prioritas di bawah adalah urutan pekerjaan yang disarankan, bukan klasifikasi insiden atau bukti tool tidak dapat digunakan.

| Prioritas | Pekerjaan | Dasar |
| --- | --- | --- |
| Utama | Buat evaluasi relevansi yang representatif | Enam pertanyaan belum cukup untuk mengukur akurasi umum |
| Utama | Perbaiki ranking, keragaman hasil, dan konteks simbol | Wrapper, tes, dan chunk impl dapat mengalahkan fungsi tujuan |
| Utama | Perbaiki parser Kotlin dan penjelasan cakupan | Satu file tetap pending setelah sync diulang |
| Utama | Pertahankan ringkasan saat output terpotong | Explain dapat mengembalikan data kosong untuk simbol yang ada |
| Berikutnya | Sediakan progres/status saat indexing | Status ditolak selama cache dikunci oleh sync |
| Berikutnya | Perkuat kontribusi lexical pada hybrid | Pada kalimat perilaku, lexical branch kosong |
| Berikutnya | Profilkan indexing dan latensi query | Indexing lanjutan memerlukan beberapa menit; lookup MCP beberapa detik |
| Setelah baseline tersedia | Bandingkan model dan reranker | Belum ada bukti bahwa model adalah penyebab utama kekurangan |

## 3. Peningkatan akurasi embedding dan retrieval

### 3.1 Bangun benchmark relevansi sebelum mengubah model

**Kondisi:** angka 5/6 top-5 berasal dari sampel kecil dengan satu target kanonis per pertanyaan. Angka ini adalah hit rate pada sampel tersebut, bukan akurasi umum atau recall atas seluruh kode relevan.

**Usulan:**

- Mulai dengan 60–100 pertanyaan yang mencakup implementasi, contoh penggunaan, konfigurasi, maintenance, error handling, dan lintas bahasa. Jumlah ini merupakan sasaran awal evaluasi, bukan ukuran yang sudah diuji.
- Beri beberapa label relevansi: implementasi langsung, wrapper relevan, tes/contoh relevan, dan tidak relevan.
- Pisahkan pertanyaan identifier, deskripsi perilaku, campuran identifier/perilaku, dan pertanyaan Indonesia.
- Pisahkan set tuning dari set evaluasi yang tidak dipakai memilih parameter.
- Ukur hit@1/hit@5, MRR, serta nDCG bila memakai label relevansi bertingkat. Ukur recall hanya jika daftar jawaban relevan cukup lengkap.
- Catat latency, jumlah kandidat, coverage, model revision, dimensi, pipeline, dan batas output pada setiap run.

**Kriteria penerimaan:** setiap perubahan ranking atau model memiliki perbandingan terhadap baseline pada pertanyaan yang sama, termasuk contoh yang membaik dan memburuk. Klaim peningkatan tidak hanya memakai rata-rata gabungan yang menyembunyikan penurunan pada satu bahasa atau jenis tugas.

### 3.2 Sesuaikan ranking dengan tujuan dan scope pengguna

**Kondisi:** pertanyaan rollback menampilkan hasil Python/Go, sementara tujuan evaluasi adalah implementasi Rust. Pertanyaan tersebut sendiri belum menyatakan batas bahasa.

**Usulan:**

- Dukung scope eksplisit berdasarkan bahasa, path/module, jenis simbol, dan kategori produksi/tes/generated code. Ini usulan kemampuan/filter; bukan klaim semua filter tersebut sudah tersedia pada API saat ini.
- Jika query menyebut Rust atau modul tertentu, gunakan sinyal tersebut untuk menyaring atau memberi bobot kandidat.
- Untuk pencarian implementasi, prioritaskan definisi fungsi yang langsung menjalankan perilaku; untuk pencarian contoh pemakaian, tetap izinkan tes dan wrapper berada di atas.
- Jangan menurunkan seluruh tes atau generated code secara permanen. Binding dapat menjadi jawaban utama saat pengguna menanyakan API bahasa tertentu.
- Tampilkan alasan singkat seperti bahasa, modul, jenis simbol dan asal kandidat, tanpa menyajikan cosine score sebagai probabilitas kebenaran.

**Kriteria penerimaan:** query yang dibatasi ke Rust menemukan implementasi Rust; query penggunaan Python tetap dapat menemukan wrapper dan tes Python. Hasil yang lintas bahasa masih dapat diperoleh ketika scope tidak dibatasi.

### 3.3 Kurangi duplikasi chunk dan tingkatkan keragaman hasil

**Kondisi:** chunk impl `Hnsw` dan fungsi di dalamnya dapat sama-sama menempati top-5. Ini mengurangi jumlah lokasi berbeda yang bisa diperiksa pengguna, meskipun keduanya relevan.

**Usulan:**

- Kelompokkan chunk dengan identitas simbol yang sama sebelum menyusun hasil akhir.
- Deteksi overlap besar antara chunk container dan anggota. Pilih fungsi anggota ketika sudah menjawab query secara spesifik; gunakan container sebagai konteks tambahan atau hasil yang dapat diperluas.
- Tetap simpan beberapa chunk jika fungsi panjang membutuhkan bagian yang berbeda untuk menjawab pertanyaan.
- Bandingkan hasil per-chunk dan per-simbol agar evaluasi tidak menghitung pengulangan yang hampir sama sebagai beberapa jawaban berguna.

**Kriteria penerimaan:** top-5 berisi lebih banyak lokasi relevan yang berbeda tanpa menghilangkan bagian fungsi panjang yang diperlukan. Untuk query penghapusan node vector, fungsi `remove` tetap mudah terlihat meskipun chunk impl memiliki kemiripan tinggi.

### 3.4 Perkaya representasi chunk secara terukur

**Usulan untuk eksperimen:** sertakan nama simbol lengkap, bahasa, path/module, signature, dokumentasi terdekat, serta isi fungsi. Pertimbangkan relasi caller/callee yang sudah tersedia sebagai konteks tambahan atau tahap perluasan kandidat, dengan tetap menghormati confidence dan cakupan graph.

Uji setiap tambahan secara terpisah. Path yang terlalu panjang, komentar berulang, container besar, atau relasi heuristic yang salah dapat menambah noise. Jangan menganggap konteks lebih banyak selalu meningkatkan retrieval.

**Kriteria penerimaan:** representasi baru memperbaiki ranking pada set evaluasi terpisah tanpa menurunkan pencarian identifier atau melipatgandakan jumlah chunk secara tidak terkendali. Perubahan format harus mempunyai pipeline version agar indeks lama tidak dipakai dengan representasi yang tidak cocok.

### 3.5 Perbaiki hybrid berdasarkan kontribusi kedua cabang

**Kondisi:** untuk enam kalimat perilaku, cabang lexical tidak menghasilkan kandidat dan urutan hybrid sama dengan semantic. Kontrol `pack_vector` menunjukkan hybrid tetap dapat menggabungkan kandidat identifier dengan semantic.

**Usulan:**

- Pertahankan pencarian exact identifier untuk query seperti `pack_vector`.
- Untuk pertanyaan panjang, uji ekstraksi identifier, pemisahan snake_case/camelCase, dan pencarian istilah pada nama, signature, serta dokumentasi. Pertahankan jalur exact agar tokenisasi tidak merusak identifier khusus.
- Tampilkan jumlah kandidat dari masing-masing cabang dan kontribusinya setelah fusion.
- Evaluasi query campuran, misalnya nama API ditambah deskripsi perilaku. Jangan menilai manfaat hybrid hanya dari kalimat yang tidak menghasilkan kandidat lexical.

**Kriteria penerimaan:** hybrid memberi tambahan kandidat relevan pada query campuran dan mempertahankan hasil exact identifier. Penambahan kandidat tidak boleh hanya meningkatkan noise dan latensi.

### 3.6 Uji reranking setelah memeriksa kandidat awal

Untuk kasus rollback, periksa apakah fungsi Rust tujuan sudah ada dalam kumpulan kandidat yang lebih luas. Pada konfigurasi yang diuji, metadata menyebut batas 100 kandidat per cabang.

- Jika target sudah masuk kandidat tetapi berada di bawah top-5, uji reranking dengan konteks query, bahasa, path dan isi fungsi.
- Jika target tidak masuk kandidat, perbaiki retrieval/representasi atau perluasan kandidat terlebih dahulu. Reranker tidak dapat mengangkat kandidat yang tidak tersedia.
- Bandingkan reranking berbasis aturan ringan dan model tambahan sebagai eksperimen terpisah; ukur biaya latensi dan memori.

**Kriteria penerimaan:** peningkatan ranking terlihat pada evaluasi terpisah dan biaya tambahan sesuai anggaran latency yang ditetapkan. Belum ada reranker yang diuji dalam sesi ini.

## 4. Coverage, freshness, dan konsistensi metadata

### 4.1 Perbaiki parser Kotlin atau klasifikasi file yang diblokir

**Bukti:** `bindings/uniffi/bkndb_ffi/bkndb_ffi.kt` dilewati karena parser error. Setelah sync ulang, indeks tetap partial dengan pending_files=1 dan excluded_files=0.

**Usulan:**

- Reproduksi parser error dari binding tersebut dan simpan kasus minimal sebagai regresi.
- Bedakan file yang menunggu embedding, file yang diblokir parser, file yang dikecualikan konfigurasi, dan file dengan chunk invalid.
- Sertakan alasan dan langkah pemulihan per file. Jangan memberi kesan bahwa sync berulang akan menyelesaikan file yang masih gagal diparse.
- Jika ada fallback ekstraksi yang lebih kasar, tandai jenis dan keterbatasannya secara eksplisit; jangan mengklaim graph presisi dari fallback teks.

**Kriteria penerimaan:** file berhasil dianalisis, atau status parsial menjelaskan secara tepat mengapa file belum tercakup dan pekerjaan apa yang diperlukan.

### 4.2 Jelaskan file yang diabaikan oleh konfigurasi

**Bukti:** pola `.gitignore` repo `bkndb-ffi*` menyebabkan sebagian source FFI tidak terinventaris dalam scope default. Ini pengaruh konfigurasi repo, bukan bug parser yang terkonfirmasi.

**Usulan:** tampilkan ringkasan scope, aturan ignore yang relevan, dan opsi pengecualian scope yang eksplisit. Jangan diam-diam memindai semua file yang sengaja diabaikan pengguna.

**Kriteria penerimaan:** pengguna dapat mengetahui bahwa source FFI ada tetapi tidak tercakup, tanpa harus menyimpulkannya dari hasil pencarian kosong.

### 4.3 Pertahankan invalidasi yang sudah bekerja dan perluas regresi

**Bukti positif:** pada fixture dua fungsi dalam satu file, perubahan source membuat semantic search mengembalikan `no valid embedded chunks; run embeddings sync`. Setelah sync embedding, fungsi baru ditemukan di rank 1 dan fungsi lama tidak muncul.

**Usulan regresi tambahan:** perubahan satu fungsi di antara banyak file, rename file/simbol, delete file, perubahan komentar/signature, perubahan pipeline/dimensi/model revision, pembatalan sync, kegagalan write, serta pemulihan setelah restart.

**Kriteria penerimaan:** stale chunks tidak dikembalikan sebagai valid; metadata membedakan freshness source dan embedding; indeks parsial hanya memakai chunk yang masih valid. Jangan menukar jaminan ini dengan percepatan yang tidak menjaga konsistensi.

### 4.4 Verifikasi konsistensi warning antar-run

**Observasi yang belum dijelaskan:** warning parser Kotlin muncul pada report awal, tetapi tidak muncul pada report ulang. Pada embedding sync, warning parser tersebut kembali dilaporkan.

Belum diketahui apakah perbedaannya memang kontrak masing-masing operasi, hasil cache, atau masalah propagasi warning. Periksa sebelum menyebutnya bug metadata.

**Kriteria penerimaan:** kontrak warning jelas; masalah yang masih berlaku tidak tersembunyi hanya karena parsing tidak dijalankan ulang. Pengguna dapat memeriksa health/coverage proyek secara konsisten.

## 5. Performa dan operasional

### 5.1 Sediakan progres yang dapat dibaca saat sync

**Bukti:** selama CLI sync mengunci cache, perintah `embeddings status` gagal karena cache sedang digunakan. Dalam run JSON yang diuji, tidak ada progres per-file/per-chunk hingga hasil akhir tersedia. Tidak semua endpoint pencarian diuji secara konkuren.

**Usulan:**

- Sediakan status operasi di luar lock database utama, atau mekanisme lain yang aman untuk membaca progres.
- Laporkan fase: verifikasi source/model, chunking, embedding, penyimpanan, dan finalisasi; sertakan jumlah selesai/total bila tersedia.
- Pertahankan JSON hasil akhir yang valid. Progres bisa memakai stderr atau mode event stream yang eksplisit agar tidak merusak konsumen JSON.
- Jelaskan proses pemegang lock dan langkah retry. Hindari estimasi waktu yang tidak didukung pengukuran.

**Kriteria penerimaan:** status tetap dapat dibaca ketika sync berlangsung; progres menunjukkan aktivitas nyata; pembatalan dan kegagalan memiliki hasil akhir yang jelas.

### 5.2 Profilkan bottleneck sebelum memilih optimasi

Waktu indexing lanjutan 914 chunk adalah 546,194 detik; lookup melalui MCP berada pada skala beberapa detik. Tidak ada pengukuran terpisah yang membuktikan apakah penyebab utamanya tokenization, inference, hashing, locking, storage, atau transport.

**Usulan instrumentation:** waktu startup/runtime, verifikasi model, scanning/hashing, parsing, chunking/tokenization, inference, write/commit, vector retrieval, fusion, dan serialisasi. Catat jumlah token/chunk, ukuran batch, peak RSS dan konfigurasi runtime.

Setelah profiling, bandingkan batching inference, penggunaan ulang runtime/model, cache embedding/query yang terversi, dan pengurangan commit overhead jika memang menjadi bottleneck. Cache query harus mengikuti model/pipeline; cache hasil retrieval juga harus mengikuti generation indeks dan filter.

**Kriteria penerimaan:** perbaikan memiliki penurunan waktu pada fase yang ditargetkan, bukan hanya fluktuasi wall-clock; kualitas hasil dan freshness tidak menurun. Uji corpus dan perangkat berbeda sebelum mengklaim speedup umum.

### 5.3 Lengkapi benchmark latency dan incremental indexing

Tambahkan pengukuran indexing dari nol, sync tanpa perubahan, perubahan satu simbol, perubahan satu file besar, restart proses, query pertama, query berulang dan query berbeda. Gunakan cukup pengulangan untuk median/p95; pisahkan biaya CLI/MCP dari waktu engine bila memungkinkan.

Fixture kecil dalam pengujian ini memerlukan sekitar 2,915 detik untuk sync awal dan 2,596 detik untuk sync setelah perubahan. Angka itu mencakup overhead MCP dan belum membuktikan efisiensi reuse embedding per-simbol. Log incremental sebaiknya menampilkan embedded/reused/invalidated chunks secara konsisten agar manfaat incremental dapat diperiksa.

## 6. Kontrak output dan kemudahan penggunaan

### 6.1 Hindari explain kosong saat budget output terlalu kecil

**Bukti:** `explain(search_vector_in)` dengan budget 16 KB mengembalikan `status=ok`, `data=[]`, `truncated=true`, `omitted_count=1`. Pada budget 256 KB, simbol yang sama muncul dengan 3 caller dan 13 hubungan keluar. Hubungan keluar tersebut mencakup references, bukan hanya function calls.

Ini perilaku pemotongan output yang teramati, bukan simbol yang gagal terindeks. Namun respons kosong mudah disalahartikan sebagai tidak ada hasil.

**Usulan:** pertahankan nama/path/signature ringkas dan jumlah hubungan, lalu potong daftar relasi secara bertahap. Sertakan alasan truncation dan petunjuk menaikkan budget atau mempersempit query. Jika seluruh item memang tidak dapat dimuat, nyatakan kondisi itu secara eksplisit dalam kontrak respons.

**Kriteria penerimaan:** query simbol yang ada tetap membedakan “tidak ditemukan” dari “ditemukan tetapi tidak muat”; output menghormati budget dan tetap dapat diparse.

### 6.2 Pisahkan alasan status partial dan truncation

Dalam pengujian semantic, partial berasal dari coverage embedding yang belum lengkap; truncated juga dapat berasal dari limit hasil. Ini berbeda dari source stale, parser error, atau budget serialisasi.

**Usulan:** sertakan alasan terstruktur per kondisi agar klien dapat menampilkan penjelasan yang tepat. Pertahankan metadata freshness source/embedding yang sudah terpisah. Jangan mengartikan similarity atau confidence heuristic sebagai persentase akurasi.

### 6.3 Buat report arsitektur lebih mudah difokuskan

Hub report pada repo ini banyak menampilkan generated UniFFI dan test/helper. Sediakan tampilan atau pembobotan scope aplikasi, tes dan generated code, dengan pilihan pengguna yang jelas. Pertahankan kemampuan melihat semuanya untuk analisis binding atau conformance.

**Kriteria penerimaan:** report dapat menjawab baik “modul produksi mana yang paling sentral?” maupun “binding atau tes mana yang paling terhubung?” tanpa mengandalkan interpretasi manual atas daftar gabungan.

## 7. Urutan implementasi yang disarankan

1. **Baseline dan kontrak:** siapkan benchmark relevansi, log fase performa, klasifikasi coverage, dan regresi output terpotong.
2. **Kualitas retrieval:** uji scope/intent ranking, deduplikasi simbol, representasi chunk dan kontribusi lexical pada hybrid. Ubah satu komponen per eksperimen.
3. **Operasional:** perbaiki parser Kotlin, status/progres saat sync, dan penjelasan lock/retry.
4. **Optimasi terukur:** pilih batching, cache atau storage changes berdasarkan profil; ukur kembali latency, throughput dan penggunaan memori.
5. **Model/reranker alternatif:** bandingkan setelah baseline stabil, memakai set evaluasi terpisah dan anggaran resource yang sama.

Tidak semua usulan harus dilakukan sekaligus. Dahulukan perbaikan yang membuat hasil terukur dan keterbatasan terlihat, kemudian pilih perubahan dengan manfaat terbesar pada workload pengguna.

## 8. Bukti dan batas laporan

Laporan pendukung historis yang dirujuk sesi lama, **tidak tersedia dalam checkout validasi ini**:

- `docs/CODE_RCL_EVALUATION_2026-10-27.md`: evaluasi tool dan graph.
- `docs/CODE_RCL_EMBEDDING_BENCHMARK_2026-10-27.md`: benchmark embedding, query dan freshness.
- `docs/CODE_RCL_EMBEDDING_RETEST_2026-10-28.md`: pengujian ulang 28 Oktober.

Bukti yang tersedia sekarang: [validasi checkout](docs/CODE_RCL_VALIDATION_2026-10-28.md), [evaluasi corpus beku](docs/RETRIEVAL-EVALUATION.md), [operasional](docs/OPERATIONS.md), dan [benchmark indexing](docs/INDEXING-BENCHMARK.md). Dokumen tersebut tidak menggantikan isi tiga dokumen historis yang hilang.

Log lokal, yang dapat hilang ketika cache dibersihkan:

- `.code-rcl/embedding-sync-audit.json`.
- `.code-rcl/embedding-sync-no-change.json`.
- `.code-rcl/embedding-tool-results.json`.
- `.code-rcl/embedding-freshness-probe/`.
- `.code-rcl/embedding-retest-2026-10-28.json`.
- `.code-rcl/embedding-retest-sync-2026-10-28.json`.

Belum diukur: indexing penuh dari nol, peak RAM, penggunaan GPU/energi, download model, beban konkuren menyeluruh, serta kualitas pada benchmark besar. Tidak ada source internal code-rcl yang diperiksa atau diperbaiki pada sesi ini. Semua rekomendasi implementasi perlu dicocokkan dengan arsitektur code-rcl yang sebenarnya sebelum diterapkan.
