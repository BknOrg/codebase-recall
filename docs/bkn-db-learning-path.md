# Learning Path: Membangun Embedded Graph Database (BknDb) dari Nol dengan Rust

> **Tujuan Belajar:** Berhenti melakukan *blind vibe-coding* dan benar-benar memahami setiap blok fondasi teknik perangkat lunak di balik sistem database: dari struktur data di memori, serialisasi biner, indeks kunci, hingga algoritma graf tingkat lanjut.

---

## 🗺️ Gambaran Peta Belajar (Curriculum Roadmap)

```
[Tahap 1: Fondasi Rust & Struktur Data] 
       │
       ▼
[Tahap 2: Teori Graf & Algoritma Topologi]
       │
       ▼
[Tahap 3: Penyimpanan Disk, Biner, & Serialisasi]
       │
       ▼
[Tahap 4: Desain Indeks Graf di Atas Key-Value Store]
       │
       ▼
[Tahap 5: Transaksi, Isolasi, & ACID]
       │
       ▼
[Tahap 6: Membangun Fluent Traversal API]
       │
       ▼
[Tahap 7: Dogfooding & Integrasi ke Dunia Nyata]
```

---

## 📌 Tahap 1: Pemrograman Sistem & Struktur Data Rust

Sebelum memikirkan database disk, Anda harus menguasai bagaimana struktur data graf hidup di memori RAM dan bagaimana Rust mengelola *ownership*, *lifetimes*, dan *aliasing*.

### 1.1 Masalah Klasik "Graph di Rust"
* **Kenapa sulit?** Dalam graf, satu Node bisa ditunjuk oleh banyak Edge (multiple ownership). Di bahasa seperti Python/Java/JS, ini tinggal pakai pointer objek `node.neighbors.push(other_node)`. Di Rust, *borrow checker* akan menolak karena aturan single-owner.
* **Materi yang Wajib Dipahami:**
  * Mengapa graf di Rust **TIDAK** menggunakan `Rc<RefCell<Node>>` untuk sistem produksi (lambat, boros memori, rawan memory leak akibat circular reference).
  * **Arena Allocation & Index-based Graphs:** Menggunakan `Vec<Node>` dan menggunakan integer ID (`u32` / `u64`) atau `generational-arena` sebagai pointer.
* **Latihan Praktek:**
  * Buat `struct Graph` di memori murni:
    ```rust
    struct Graph {
        nodes: Vec<NodeData>,
        // Adjacency list: node_index -> daftar edge
        edges: Vec<Vec<EdgeData>>,
    }
    ```
  * Tambahkan 100 node dan hubungkan antar node tanpa menggunakan `unsafe` atau `Rc/RefCell`.

---

## 📌 Tahap 2: Teori Graf & Algoritma Traversal

Jangan menulis kueri sebelum paham cara menjelajahi jaringan node dan edge.

### 2.1 Algoritma Pencarian Inti
* **Breadth-First Search (BFS):**
  * Konsep: Menggunakan antrean (`std::collections::VecDeque`).
  * Kapan dipakai: Menemukan jarak/hop terpendek (*shortest path unweighted*), atau mencari semua tetangga dalam radius $N$ kedalaman (seperti fitur `dump -r` di `codebase-recall`).
* **Depth-First Search (DFS):**
  * Konsep: Menggunakan rekursi atau tumpukan manual (`Vec`).
  * Kapan dipakai: Menemukan hierarki pohon (*call tree* seperti di fitur `impact`), topological sorting.

### 2.2 Tantangan Krusial: Siklus (*Cycle Detection*)
* **Masalah:** Jika Node A memanggil B, B memanggil C, dan C memanggil A (`A -> B -> C -> A`), BFS/DFS biasa akan mengalami *infinite loop* atau *stack overflow*.
* **Materi yang Harus Dipahami:**
  * State tracking: `visited: HashSet<NodeId>` atau three-color marking (`White`, `Gray`, `Black`).
* **Latihan Praktek:**
  * Implementasikan fungsi `find_cycles(&self) -> Vec<Vec<NodeId>>` untuk mendeteksi siklus dependensi antar modul kode.
  * Implementasikan perhitungan derajat: `in_degree(node_id)` dan `out_degree(node_id)` (*dasar fitur `digest`*).

---

## 📌 Tahap 3: Penyimpanan Disk, Encoding Biner, & Serialisasi

Database bukanlah database jika datanya hilang saat komputer dimatikan.

### 3.1 Serialisasi Data ke Byte
* Bagaimana struct Rust diubah menjadi deretan bita (`&[u8]`) di disk?
* **Materi:**
  * Pelajari crate **`serde`** dan **`bincode`** (paling sederhana dan cepat untuk pemula).
  * Pahami mengapa format teks (JSON/YAML) tidak cocok untuk storage engine internal (lambat di-parse, boros ukuran, tidak fixed-size).
* **Latihan Praktek:**
  * Buat struct `NodeRecord`, serialisasikan ke file biner menggunakan `bincode::serialize_into`, lalu baca kembali dengan `bincode::deserialize_from`.

### 3.2 Memahami B-Tree & Key-Value Storage
* Jangan menulis sistem paging disk dari nol. Pelajari cara kerja **Key-Value Store**.
* **Materi yang Wajib Dipelajari:**
  * B-Tree basics: Kenapa B-Tree digunakan di database (karena data tersortir secara leksikografis, memungkinkan *range scan* yang sangat cepat).
  * Crate **`redb`**: Buka dokumentasi `redb`, pahami konsep `TableDefinition`, `ReadableTable`, dan transaksi `begin_read()` / `begin_write()`.

---

## 📌 Tahap 4: Desain Indeks Graf di Atas Key-Value

Ini adalah **rahasia terbesar** dari database graf seperti Neo4j/Kùzu/CozoDB: bagaimana memetakan struktur graf yang fleksibel ke dalam tabel Key-Value yang kaku.

### 4.1 Skema Prefix Kunci Majemuk (Compound Keys)
Pahami bagaimana menyusun kunci biner agar pencarian relasi menjadi $O(\log N)$:

1. **Tabel Simpul (`nodes`):**
   * Key: `u64` (`node_id`)
   * Value: `bincode([label, properties])`
2. **Indeks Forward Edges (`adj_out`):**
   * Key: `(from_node: u64, edge_type: String, to_node: u64)`
   * Value: `edge_id: u64` atau `properties`
3. **Indeks Inverted Edges (`adj_in`):**
   * Key: `(to_node: u64, edge_type: String, from_node: u64)`
   * Value: `edge_id: u64`

### 4.2 Kueri dengan Range Scan
* Jika ingin tahu: *"Siapa saja yang dipanggil oleh Node 42 dengan jenis relasi 'calls'?"*
* Anda tidak perlu membaca seluruh database. Anda cukup melakukan scan pada tabel `adj_out` pada rentang:
  `(42, "calls", 0) ..= (42, "calls", u64::MAX)`
* **Latihan Praktek:**
  * Buat database `redb` sederhana yang menyimpan 3 tabel di atas. Coba masukkan relasi sederhana dan lakukan kueri pencarian tetangga dengan Range Scan.

---

## 📌 Tahap 5: Transaksi, Atomisitas, & Cascade Deletion

Di sinilah Anda belajar berpikir seperti seorang *Database Engineer*, bukan sekadar *Application Programmer*.

### 5.1 Atomisitas & Konsistensi (ACID)
* Bayangkan saat Anda membuat Edge `A -> B`. Anda harus menulis ke dua tempat: `adj_out` dan `adj_in`.
* Apa yang terjadi jika listrik mati setelah menulis ke `adj_out` tapi belum sempat menulis ke `adj_in`? Graf menjadi korup (*inconsistent*)!
* **Materi:** Pelajari bagaimana transaksi `WriteTransaction` di `redb` menjamin operasi *all-or-nothing* (commit atau rollback).

### 5.2 Cascade Deletion
* Saat Node `A` dihapus:
  1. Hapus Node `A` dari tabel `nodes`.
  2. Cari semua edge keluar dari `A` di `adj_out`, lalu hapus pasangan pasangannya di `adj_in`.
  3. Cari semua edge masuk ke `A` di `adj_in`, lalu hapus pasangannya di `adj_out`.
* Semua langkah di atas harus berada dalam **satu transaksi atomik**.

---

## 📌 Tahap 6: Merancang Fluent Traversal API

Pengguna database Anda tidak boleh dipaksa menulis operasi biner atau range scan manual. Anda harus menyediakan API yang elegan dan ekspresif.

### 6.1 Pola Iterator & Builder di Rust
* **Materi:**
  * Memahami trait `Iterator` di Rust.
  * Merancang chaining method:
    ```rust
    let callers = db.traversal()
        .start(symbol_id)
        .incoming("calls")
        .max_depth(3)
        .run()?;
    ```
* **Latihan Praktek:**
  * Buat `struct TraversalBuilder` yang menampung parameter pencarian, lalu pada method `.run()` ia menjalankan algoritma BFS/DFS dari Tahap 2 di atas data dari Tahap 4.

---

## 📌 Tahap 7: Dogfooding ke Proyek Nyata (`bkn-db` -> `codebase-recall`)

Langkah pembuktian tertinggi adalah saat pustaka yang Anda buat digunakan oleh aplikasi nyata.

1. **Uji Validasi:**
   * Buka file `src/cache/mod.rs` di `codebase-recall`.
   * Ganti `rusqlite` dengan crate `bkn-db` yang Anda buat.
2. **Jalankan Uji Coba:**
   * `code-rcl sync` -> Pastikan parsing simbol dan dependensi tersimpan rapi di `.bkndb`.
   * `code-rcl impact` -> Pastikan hasil pohon pemanggil identik dengan versi SQLite.
   * `code-rcl digest` -> Pastikan daftar arsitektur hub terhitung dengan tepat.

---

## 📚 Buku & Referensi yang Sangat Dianjurkan

1. **Rust Internals & Systems:**
   * *"Rust for Rustaceans"* oleh Jon Gjengset (khusus bab tentang *Data Structures*, *Type Systems*, dan *Unsafe/Memory Layout*).
2. **Teori Database:**
   * *"Designing Data-Intensive Applications"* (DDIA) oleh Martin Kleppmann (Bab 3: *Storage and Retrieval* — wajib dibaca untuk memahami LSM-Tree vs B-Tree).
3. **Open-Source Code untuk Dibaca:**
   * Repository [`redb`](https://github.com/cberner/redb): Pelajari bagaimana B-Tree diimplementasikan murni di Rust.
   * Repository [`petgraph`](https://github.com/petgraph/petgraph): Pelajari arsitektur internal representasi graf in-memory.
