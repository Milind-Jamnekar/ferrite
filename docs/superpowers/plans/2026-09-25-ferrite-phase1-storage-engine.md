# Ferrite Phase 1 — Storage Engine Implementation Plan

> **Execution model — read this before anything else:** This plan is
> **not** executed by an autonomous agent or subagent. Per the spec, the
> author writes every line of production code themselves; Claude's role
> is a concept walkthrough before each task and a code review after it.
> Do NOT invoke `subagent-driven-development` or `executing-plans` for
> this plan — those assume an agent writes the code, which contradicts
> the whole point of this project. Work through tasks in order,
> in-session, one at a time.
>
> **Why "implement" steps below don't contain full solution code:**
> Standard plans give complete code for every step because an autonomous
> executor has no other source of judgment. Here the opposite is true —
> handing over finished code would remove the exercise. Each task instead
> gives you the exact interface (types/signatures other tasks depend on)
> and exact test code (the tests **are** the specification — if your
> implementation passes them and matches the written algorithm
> description, it's correct). The "Concept" line names what Claude
> explains live before you start writing. Ask questions freely — this is
> meant to be worked interactively, not read linearly.

**Goal:** A working, crash-safe, single-node embedded LSM-tree key-value
store in Rust, with `put`/`get`/`delete`/`scan`, that the author both
wrote and understands.

**Architecture:** WAL for durability → in-memory sorted memtable →
flushed to immutable sorted SSTable files on disk → newest-first read
path across memtable then SSTables → periodic compaction merges
SSTables and drops obsolete data. Fully single-threaded.

**Tech Stack:** Rust (stable), `crc32fast` (checksum only — the WAL/
SSTable *formats* are still hand-rolled), `proptest` + `tempfile` (dev
dependencies, tests only).

**Spec:** `docs/superpowers/specs/2026-09-25-ferrite-phase1-storage-engine-design.md`

## Global Constraints

- All multi-byte integers in on-disk formats are little-endian
  (`to_le_bytes()` / `from_le_bytes()`), per spec "On-disk formats".
- Keys and values are `Vec<u8>` everywhere — no generics (spec Non-goals).
- No threads, no locks, no background compaction — everything runs
  synchronously on the caller's thread (spec Non-goals).
- No `serde`/`bincode` for WAL or SSTable encoding, no `thiserror` for
  the error type — both are hand-rolled by design (spec On-disk formats,
  Error handling).
- New on-disk files (SSTable flush output, compaction output) are always
  written to a temp path, fsynced, then atomically renamed into place
  (spec File naming / ordering).
- SSTable indexing is dense (one index entry per key) for Phase 1; bloom
  filters and sparse indexing are explicitly out of scope (spec
  Non-goals).

---

### Task 1: Toolchain + Rust warm-up

**Files:**
- Create: `examples/warmup.rs` (throwaway — not part of the shipped
  library, safe to leave in place afterward)

**Concept:** why Rust for this project (ownership prevents the class of
bugs — use-after-free, data races — that make storage engines notoriously
hard to get right in C/C++), and a tour of the four idea clusters this
project leans on hardest: ownership/borrowing, `Option`/`Result` +
`?`, structs/enums/traits, and byte slices (`&[u8]`).

- [ ] **Step 1: Install the toolchain**

Run: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
then restart your shell.
Verify: `cargo --version` and `rustc --version` both print a version
(1.7x or newer).

- [ ] **Step 2: Scaffold the crate**

Run (inside `/Users/milindjamnekar/Developer/Projects/ferrite`):
`cargo init --name ferrite --lib`
Expected: creates `Cargo.toml`, `src/lib.rs`, and a `.gitignore`
containing `/target`. This won't disturb the existing `docs/` directory
or git history.

- [ ] **Step 3: Ownership/borrowing exercise**

In `examples/warmup.rs`, write and run:
```rust
fn first_word(s: &str) -> &str {
    // returns the substring before the first space, or the whole
    // string if there's no space
    todo!()
}

fn main() {
    let sentence = String::from("hello world");
    let word = first_word(&sentence);
    println!("{word}"); // must print: hello
    println!("{sentence}"); // must still compile and print: hello world
}
```
Implement `first_word` (delete the `todo!()`). Run with
`cargo run --example warmup`. The point: `word` borrows from `sentence`
without taking ownership, so both are usable afterward — that's the
core borrow-checker idea you'll lean on in `wal.rs` and `sstable.rs`.

- [ ] **Step 4: `Option`/`Result`/`?` exercise**

Extend `warmup.rs`:
```rust
fn parse_and_double(input: &str) -> Result<i32, std::num::ParseIntError> {
    let n: i32 = input.parse()?;
    Ok(n * 2)
}

fn find_first_even(nums: &[i32]) -> Option<i32> {
    // returns the first even number in nums, or None
    todo!()
}
```
Implement `find_first_even`, call both functions in `main` with a few
inputs (including a bad one like `"abc"` for `parse_and_double`) and
print the results. Note how `?` on `Result` mirrors early-return the way
`None` short-circuits `Option` — you'll use exactly this pattern in
`Error` conversions later.

- [ ] **Step 5: Struct/enum/trait exercise**

Extend `warmup.rs`:
```rust
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

trait Area {
    fn area(&self) -> f64;
}

impl Area for Shape {
    fn area(&self) -> f64 {
        todo!() // match on self, compute area per variant
    }
}
```
Implement `Area for Shape`, construct one of each variant, print both
areas. This is the exact shape you'll use for the memtable's
`Value::Data(Vec<u8>) | Value::Tombstone` enum in Task 5.

- [ ] **Step 6: Byte slice exercise**

Extend `warmup.rs`:
```rust
fn main_bytes() {
    let n: u32 = 42;
    let bytes: [u8; 4] = n.to_le_bytes();
    println!("{bytes:?}"); // [42, 0, 0, 0]
    let back = u32::from_le_bytes(bytes);
    assert_eq!(back, n);

    let buf: Vec<u8> = vec![1, 2, 3, 4, 5];
    let slice: &[u8] = &buf[1..3];
    println!("{slice:?}"); // [2, 3]
}
```
Run it (call `main_bytes()` from `main`). This `to_le_bytes`/
`from_le_bytes` pair is *exactly* how every integer in the WAL and
SSTable formats gets encoded — no crate needed for that part.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore examples/warmup.rs
git commit -m "Scaffold crate, complete Rust warm-up exercises"
```

---

### Task 2: Error type

**Files:**
- Create: `src/error.rs`
- Modify: `src/lib.rs` — add `mod error; pub use error::{Error, Result};`

**Concept:** why Rust encodes fallibility in the type system (`Result<T, E>`)
instead of exceptions, what `impl From<X> for Error` buys you (the `?`
operator auto-converts via `From`), and why `NotFound` is *not* one of
our error variants (spec: absence is a normal `Ok(None)`, not an error).

**Interfaces produced** (used by every later task):
```rust
pub enum Error {
    Io(std::io::Error),
    Corruption(String),
}
pub type Result<T> = std::result::Result<T, Error>;
// impl std::fmt::Display for Error
// impl std::error::Error for Error
// impl From<std::io::Error> for Error
```

- [ ] **Step 1: Write the failing test**

In `src/error.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_and_displays() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("missing file"));
    }

    #[test]
    fn corruption_displays_message() {
        let err = Error::Corruption("bad crc".to_string());
        assert_eq!(err.to_string(), "corruption: bad crc");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test error::tests`
Expected: fails to compile (`Error` doesn't exist yet).

- [ ] **Step 3: Implement**

Write `Error`, `Result<T>`, `Display`, `std::error::Error`, and
`From<std::io::Error>` matching the interface above. Requirements:
- `Display` for `Error::Corruption(msg)` must render exactly
  `"corruption: {msg}"` (the second test above checks this literally).
- `Display` for `Error::Io(e)` should include `e`'s own message (e.g. via
  `write!(f, "io error: {e}")`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test error::tests`
Expected: both tests PASS.

- [ ] **Step 5: Wire into `lib.rs` and commit**

`src/lib.rs` should contain `mod error;` and `pub use error::{Error, Result};`.
```bash
git add src/error.rs src/lib.rs
git commit -m "Add hand-rolled Error type"
```

---

### Task 3: WAL entry encode/decode

**Files:**
- Create: `src/wal.rs`
- Modify: `src/lib.rs` — add `pub mod wal;` (public: Task 13's
  integration tests need `ferrite::wal::{WalEntry, encode_entry}`
  directly, since integration tests compile as a separate crate)
- Modify: `Cargo.toml` — add `crc32fast = "1"` under `[dependencies]`

**Concept:** walk the exact byte layout from the spec
(`crc32|entry_len|op|key_len|key|value_len|value`), and why the CRC
covers everything *after* itself — so a reader can verify integrity
before trusting `entry_len` to know how much more to read.

**Interfaces produced:**
```rust
pub enum WalEntry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

pub fn encode_entry(entry: &WalEntry) -> Vec<u8>;
// Decodes one entry starting at the front of `bytes`. Returns the
// entry and the number of bytes consumed. Errors with
// Error::Corruption if the CRC doesn't match the entry's own bytes.
pub fn decode_entry(bytes: &[u8]) -> crate::Result<(WalEntry, usize)>;
```

- [ ] **Step 1: Write the failing tests**

In `src/wal.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_round_trips() {
        let entry = WalEntry::Put { key: b"hello".to_vec(), value: b"world".to_vec() };
        let bytes = encode_entry(&entry);
        let (decoded, consumed) = decode_entry(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        match decoded {
            WalEntry::Put { key, value } => {
                assert_eq!(key, b"hello");
                assert_eq!(value, b"world");
            }
            _ => panic!("expected Put"),
        }
    }

    #[test]
    fn delete_round_trips() {
        let entry = WalEntry::Delete { key: b"gone".to_vec() };
        let bytes = encode_entry(&entry);
        let (decoded, consumed) = decode_entry(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        match decoded {
            WalEntry::Delete { key } => assert_eq!(key, b"gone"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn corrupt_crc_is_rejected() {
        let entry = WalEntry::Put { key: b"k".to_vec(), value: b"v".to_vec() };
        let mut bytes = encode_entry(&entry);
        // flip a bit in the key, leaving the CRC stale
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let result = decode_entry(&bytes);
        assert!(matches!(result, Err(crate::Error::Corruption(_))));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test wal::tests`
Expected: fails to compile (`WalEntry`, `encode_entry`, `decode_entry`
don't exist yet).

- [ ] **Step 3: Implement**

Requirements (exact byte layout, all integers little-endian):
- Layout after the 4-byte CRC: `entry_len(u32) | op(u8) | key_len(u32) | key | value_len(u32) | value`.
  `op` is `0` for `Put`, `1` for `Delete`. For `Delete`, omit
  `value_len`/`value` entirely.
- `entry_len` is the byte length of everything *after* `entry_len`
  itself (i.e. `op` through the end of `value`) — this is what a reader
  uses to know how many bytes to read before re-deriving the CRC.
- `crc32fast::hash(&bytes_after_crc)` computed over exactly those bytes;
  stored as the first 4 bytes (LE) of the encoded entry.
- `decode_entry` must: read the CRC, read `entry_len`, slice out
  exactly `entry_len` bytes, recompute the CRC over that slice, compare
  — mismatch is `Err(Error::Corruption("wal entry checksum mismatch"))`.
  On match, parse `op`/`key`/`value` from the slice and return the
  entry plus total bytes consumed (`4 + 4 + entry_len`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test wal::tests`
Expected: all three PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/wal.rs src/lib.rs
git commit -m "Add WAL entry encode/decode with CRC validation"
```

---

### Task 4: WAL file — append and replay

**Files:**
- Modify: `src/wal.rs`
- Modify: `Cargo.toml` — add `tempfile = "3"` under `[dev-dependencies]`

**Concept:** the difference between "encode one entry" (Task 3, pure
function) and "durably persist a sequence of entries to a real file"
(this task) — where `fsync` fits, and why replay must tolerate a torn
final entry (the process can die mid-`write()`) but must *not* tolerate
corruption in the middle of the file (that indicates a real bug, not a
crash artifact).

**Interfaces produced:**
```rust
pub struct Wal { /* private fields */ }

impl Wal {
    pub fn create(path: &std::path::Path) -> crate::Result<Self>;
    pub fn open_append(path: &std::path::Path) -> crate::Result<Self>;
    pub fn append(&mut self, entry: &WalEntry) -> crate::Result<()>; // encodes + writes + fsyncs
    // Reads every valid entry from `path` in order. If the final
    // bytes in the file don't form a complete, checksum-valid entry,
    // they are silently discarded (torn write). A checksum failure
    // anywhere *before* the last entry is Err(Error::Corruption(_)).
    pub fn replay(path: &std::path::Path) -> crate::Result<Vec<WalEntry>>;
}
```

- [ ] **Step 1: Write the failing tests**

Add to `src/wal.rs`'s `tests` module:
```rust
use std::io::Write;

#[test]
fn append_then_replay_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::create(&path).unwrap();
    wal.append(&WalEntry::Put { key: b"a".to_vec(), value: b"1".to_vec() }).unwrap();
    wal.append(&WalEntry::Delete { key: b"b".to_vec() }).unwrap();
    drop(wal);

    let entries = Wal::replay(&path).unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn replay_discards_torn_final_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::create(&path).unwrap();
    wal.append(&WalEntry::Put { key: b"a".to_vec(), value: b"1".to_vec() }).unwrap();
    drop(wal);

    // simulate a crash mid-write: append a second, well-formed entry,
    // then truncate the file partway through it
    let good_bytes = encode_entry(&WalEntry::Put { key: b"b".to_vec(), value: b"2".to_vec() });
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&good_bytes[..good_bytes.len() - 2]).unwrap(); // drop last 2 bytes

    let entries = Wal::replay(&path).unwrap();
    assert_eq!(entries.len(), 1); // only the first, complete entry survives
}

#[test]
fn open_append_adds_to_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.wal");

    Wal::create(&path).unwrap()
        .append(&WalEntry::Put { key: b"a".to_vec(), value: b"1".to_vec() }).unwrap();

    let mut wal = Wal::open_append(&path).unwrap();
    wal.append(&WalEntry::Put { key: b"b".to_vec(), value: b"2".to_vec() }).unwrap();
    drop(wal);

    assert_eq!(Wal::replay(&path).unwrap().len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test wal::tests`
Expected: fails to compile (`Wal` doesn't exist yet).

- [ ] **Step 3: Implement**

Requirements:
- `Wal::create` opens the file with `create_new` semantics (fails if it
  already exists — a fresh WAL should never silently overwrite one).
- `Wal::append` calls `encode_entry`, writes the bytes, then calls
  `.sync_all()` on the file handle (this is the fsync).
- `Wal::replay` reads the whole file into memory, then loops calling
  `decode_entry` on successively later offsets. For each call: if it
  succeeds, keep the entry and advance by `consumed`; if it fails with
  `Error::Corruption` *and* the remaining bytes are shorter than what a
  complete header would require, OR the CRC just doesn't match, treat it
  as a torn tail **only when it's the last attempted read** — i.e. stop
  and return everything decoded so far without propagating the error.
  If a decode fails and there are still more bytes after it that
  themselves decode successfully, that's a real corruption in the
  middle — propagate `Err(Error::Corruption(_))` instead of skipping it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test wal::tests`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/wal.rs
git commit -m "Add WAL file append and crash-tolerant replay"
```

---

### Task 5: MemTable

**Files:**
- Create: `src/memtable.rs`
- Modify: `src/lib.rs` — add `mod memtable;`

**Concept:** why the memtable needs to track its own approximate byte
size (that's what decides when to flush), and why deletes are stored as
an explicit tombstone rather than just removing the key — an SSTable on
disk still has the old value, so "absent from the memtable" doesn't mean
"absent from the database."

**Interfaces produced:**
```rust
pub enum Value {
    Data(Vec<u8>),
    Tombstone,
}

pub struct MemTable { /* private: BTreeMap<Vec<u8>, Value>, size_bytes: usize */ }

impl MemTable {
    pub fn new() -> Self;
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>);
    pub fn delete(&mut self, key: Vec<u8>);
    pub fn get(&self, key: &[u8]) -> Option<&Value>;
    pub fn size_bytes(&self) -> usize;
    // Sorted by key ascending — this is what the flush step (Task 6) iterates.
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Value)>;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_returns_data() {
        let mut mt = MemTable::new();
        mt.put(b"k".to_vec(), b"v".to_vec());
        match mt.get(b"k") {
            Some(Value::Data(v)) => assert_eq!(v, b"v"),
            other => panic!("expected Some(Data), got {other:?}"),
        }
    }

    #[test]
    fn delete_stores_tombstone() {
        let mut mt = MemTable::new();
        mt.put(b"k".to_vec(), b"v".to_vec());
        mt.delete(b"k".to_vec());
        assert!(matches!(mt.get(b"k"), Some(Value::Tombstone)));
    }

    #[test]
    fn missing_key_returns_none() {
        let mt = MemTable::new();
        assert!(mt.get(b"nope").is_none());
    }

    #[test]
    fn size_bytes_grows_with_puts() {
        let mut mt = MemTable::new();
        assert_eq!(mt.size_bytes(), 0);
        mt.put(b"key".to_vec(), b"value".to_vec());
        assert!(mt.size_bytes() > 0);
    }

    #[test]
    fn iter_is_sorted_by_key() {
        let mut mt = MemTable::new();
        mt.put(b"c".to_vec(), b"3".to_vec());
        mt.put(b"a".to_vec(), b"1".to_vec());
        mt.put(b"b".to_vec(), b"2".to_vec());
        let keys: Vec<_> = mt.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }
}
```
(`Value` needs `#[derive(Debug)]` for the `panic!("... {other:?}")` in
the first test to compile.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test memtable::tests`
Expected: fails to compile.

- [ ] **Step 3: Implement**

Requirements:
- Backed by `std::collections::BTreeMap<Vec<u8>, Value>` (this is what
  gives you sorted iteration for free).
- `size_bytes` requirement: increases by `key.len() + value.len()` on
  each `put` (approximate — exact accounting of overwrites/deletes
  shrinking it is not required for Phase 1, only monotonic growth
  driving the flush threshold matters).
- `delete` inserts `Value::Tombstone` at that key (does not remove the
  map entry — the tombstone itself must be visible to `get` and, later,
  must survive being flushed to an SSTable so reads of older SSTables
  are correctly shadowed).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test memtable::tests`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/memtable.rs src/lib.rs
git commit -m "Add MemTable with tombstone-aware delete"
```

---

### Task 6: SSTable writer

**Files:**
- Create: `src/sstable.rs`
- Modify: `src/lib.rs` — add `mod sstable;`

**Concept:** the on-disk layout from the spec (data block → dense index
block → fixed footer) and why the footer is written *last* but read
*first* — its fixed size and known offset from EOF is what lets a reader
find the index without scanning the whole file.

**Interfaces produced:**
```rust
// Writes `entries` (must already be sorted by key ascending — the
// caller, e.g. MemTable::iter, guarantees this) to `path` using the
// temp-file + fsync + rename pattern. Tombstones are encoded with a
// value_len sentinel of u32::MAX (no bytes follow) so the reader can
// tell "empty value" apart from "deleted."
pub fn write_sstable(
    path: &std::path::Path,
    entries: impl Iterator<Item = (Vec<u8>, crate::memtable::Value)>,
) -> crate::Result<()>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::Value;

    #[test]
    fn writes_file_that_exists_and_is_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("000001.sst");
        let entries = vec![
            (b"a".to_vec(), Value::Data(b"1".to_vec())),
            (b"b".to_vec(), Value::Tombstone),
            (b"c".to_vec(), Value::Data(b"3".to_vec())),
        ];
        write_sstable(&path, entries.into_iter()).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
        // no leftover temp file
        let tmp = path.with_extension("sst.tmp");
        assert!(!tmp.exists());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test sstable::tests`
Expected: fails to compile.

- [ ] **Step 3: Implement**

Requirements (all integers little-endian):
- **Data block**: for each entry in order, write
  `key_len(u32) | key | value_len(u32) | value`. For a tombstone, write
  `value_len = u32::MAX` and no value bytes.
- While writing the data block, record each key's *starting offset*
  (the offset of `key_len`, not of the value) — you need these for the
  index block.
- **Index block**: for each entry, write `key_len(u32) | key | offset(u64)`
  where `offset` is that key's recorded starting offset in the data
  block.
- **Footer** (fixed 20 bytes): `index_block_offset(u64) | index_block_len(u64) | magic(u32)`.
  Pick any constant `u32` for `magic` (e.g. `0xF3A5_1234`) and reuse the
  exact same constant in Task 7's reader.
- Write everything to `path.with_extension("sst.tmp")` (or a sibling tmp
  path — anything that ends up in the same directory so the rename is
  atomic), call `.sync_all()`, then `std::fs::rename` to the real path.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test sstable::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sstable.rs src/lib.rs
git commit -m "Add SSTable writer (data block + dense index + footer)"
```

---

### Task 7: SSTable reader

**Files:**
- Modify: `src/sstable.rs`

**Concept:** the read path — load the footer, load the index into
memory once, then binary-search it per lookup instead of re-reading the
whole file every time. This is the shape every real LSM engine's reader
takes.

**Interfaces produced:**
```rust
pub struct SsTable { /* private: file handle, in-memory index Vec<(Vec<u8>, u64)> */ }

impl SsTable {
    // Reads the footer and index block into memory; does not read the
    // data block yet.
    pub fn open(path: &std::path::Path) -> crate::Result<Self>;
    // Binary-searches the in-memory index; on a hit, seeks into the
    // data block and reads that one entry's value (or tombstone).
    pub fn get(&mut self, key: &[u8]) -> crate::Result<Option<crate::memtable::Value>>;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn round_trips_through_writer_and_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("000001.sst");
    let entries = vec![
        (b"a".to_vec(), Value::Data(b"1".to_vec())),
        (b"b".to_vec(), Value::Tombstone),
        (b"c".to_vec(), Value::Data(b"3".to_vec())),
    ];
    write_sstable(&path, entries.into_iter()).unwrap();

    let mut table = SsTable::open(&path).unwrap();
    match table.get(b"a").unwrap() {
        Some(Value::Data(v)) => assert_eq!(v, b"1"),
        other => panic!("expected Some(Data), got {other:?}"),
    }
    assert!(matches!(table.get(b"b").unwrap(), Some(Value::Tombstone)));
    assert!(table.get(b"missing").unwrap().is_none());
}

#[test]
fn rejects_file_with_bad_magic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.sst");
    std::fs::write(&path, b"not an sstable at all, too short").unwrap();
    assert!(SsTable::open(&path).is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test sstable::tests`
Expected: fails to compile / fails (methods don't exist).

- [ ] **Step 3: Implement**

Requirements:
- `open`: seek to `EOF - 20`, read the footer, verify `magic` matches
  the writer's constant exactly (else `Err(Error::Corruption("bad sstable magic"))`,
  which also naturally covers files too short to even hold a footer —
  check the file length before seeking and return that same error if
  it's under 20 bytes). Then seek to `index_block_offset` and parse
  `index_block_len` bytes into an in-memory `Vec<(Vec<u8>, u64)>`.
- `get`: binary search the in-memory index by key (`Vec<(Vec<u8>, u64)>`
  is already sorted, since it was written from sorted input). On a
  match, seek the file to that offset and parse one data-block entry
  (`key_len|key|value_len|value`) to recover the `Value` — remember
  `value_len == u32::MAX` decodes to `Value::Tombstone`. No match →
  `Ok(None)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test sstable::tests`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sstable.rs
git commit -m "Add SSTable reader with binary-search lookup"
```

---

### Task 8: Db — put/get/delete orchestration with flush

**Files:**
- Create: `src/db.rs`
- Modify: `src/lib.rs` — add `mod db; pub use db::Db;`

**Concept:** how the pieces built so far compose — every write goes
through the WAL *before* the memtable (durability first, visibility
second), every read checks the memtable before any SSTable (newest data
wins), and why flush swaps in a brand new empty memtable rather than
trying to clear the old one in place (the old one becomes exactly the
input to `write_sstable` from Task 6).

**Interfaces produced:**
```rust
pub struct Db { /* private: dir, memtable, wal, sstables: Vec<SsTable> (newest first), next_seq: u64 */ }

impl Db {
    // For this task: creates `dir` if needed, starts with an empty
    // memtable and a fresh WAL. (Task 9 adds real recovery — for now
    // it's fine for open() to assume a fresh, empty directory.)
    pub fn open(dir: &std::path::Path) -> crate::Result<Self>;
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> crate::Result<()>;
    pub fn delete(&mut self, key: Vec<u8>) -> crate::Result<()>;
    pub fn get(&mut self, key: &[u8]) -> crate::Result<Option<Vec<u8>>>;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_from_memtable() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path()).unwrap();
        db.put(b"k".to_vec(), b"v".to_vec()).unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn delete_makes_key_absent() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path()).unwrap();
        db.put(b"k".to_vec(), b"v".to_vec()).unwrap();
        db.delete(b"k".to_vec()).unwrap();
        assert_eq!(db.get(b"k").unwrap(), None);
    }

    #[test]
    fn flush_moves_data_to_sstable_and_stays_readable() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path()).unwrap();
        // write enough data to force at least one flush; exact
        // threshold is an implementation constant (see Step 3), so
        // write comfortably past any reasonable choice
        for i in 0..2000u32 {
            let k = format!("key{i:05}").into_bytes();
            let v = vec![b'x'; 200];
            db.put(k, v).unwrap();
        }
        assert!(std::fs::read_dir(dir.path()).unwrap()
            .any(|e| e.unwrap().path().extension().map_or(false, |ext| ext == "sst")));
        // spot check: an early key should have been flushed out of the
        // memtable by now, but still readable via the SSTable path
        assert_eq!(db.get(b"key00000").unwrap(), Some(vec![b'x'; 200]));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test db::tests`
Expected: fails to compile.

- [ ] **Step 3: Implement**

Requirements:
- `open`: `std::fs::create_dir_all(dir)`, then `Wal::create(dir.join("wal.log"))`,
  empty `MemTable::new()`, empty `sstables: Vec::new()`, `next_seq: 0`.
- `put`/`delete`: append a `WalEntry` to the WAL first, then apply the
  same change to the memtable, then check `memtable.size_bytes()`
  against a flush threshold — pick a constant here (a good choice is
  small enough to actually trigger in tests without writing gigabytes,
  e.g. `256 * 1024` bytes) and name it `const FLUSH_THRESHOLD_BYTES`.
  When crossed: call `write_sstable` (Task 6) with the memtable's sorted
  `iter()` to `dir.join(format!("{:06}.sst", next_seq))`, increment
  `next_seq`, `SsTable::open` the new file and push it to the *front* of
  `self.sstables` (front = newest, matching the newest-first read
  order), replace `self.memtable` with a fresh `MemTable::new()`, and
  start a fresh WAL for the new memtable generation (rotate: rename or
  just truncate — for Phase 1 it's fine to `Wal::create` a new
  `wal.log` after removing the old one, since everything in it is now
  durably in the SSTable).
- `get`: check `self.memtable.get(key)` first — `Some(Value::Data(v))` →
  `Ok(Some(v))`, `Some(Value::Tombstone)` → `Ok(None)` (short-circuit,
  do not check SSTables). If the memtable has no entry, iterate
  `self.sstables` newest-to-oldest calling `.get(key)`; the first
  `Some(_)` result wins (`Data` → `Ok(Some(v))`, `Tombstone` →
  `Ok(None)`); if every SSTable returns `None`, the key is absent →
  `Ok(None)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test db::tests`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs src/lib.rs
git commit -m "Add Db orchestration: put/get/delete with flush-on-threshold"
```

---

### Task 9: Recovery on open

**Files:**
- Modify: `src/db.rs`

**Concept:** why recovery has two independent jobs — rediscovering
*existing* SSTables (just a directory listing, sorted by the sequence
number embedded in each filename) and *replaying* the WAL to rebuild
whatever memtable state hadn't been flushed yet — and why doing both
correctly is what makes restarting the process safe.

**Interfaces produced:** `Db::open`'s contract changes (signature is
unchanged) — it must now behave correctly against a *non-empty*
directory from a previous run, not just create a fresh one.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn reopen_after_restart_preserves_all_data() {
    let dir = tempfile::tempdir().unwrap();

    {
        let mut db = Db::open(dir.path()).unwrap();
        db.put(b"a".to_vec(), b"1".to_vec()).unwrap();
        db.put(b"b".to_vec(), b"2".to_vec()).unwrap();
        db.delete(b"a".to_vec()).unwrap();
        // db dropped here — simulates process exit without a clean
        // shutdown hook, on purpose
    }

    let mut db = Db::open(dir.path()).unwrap();
    assert_eq!(db.get(b"a").unwrap(), None); // delete survived restart
    assert_eq!(db.get(b"b").unwrap(), Some(b"2".to_vec()));
}

#[test]
fn reopen_after_flush_finds_sstables() {
    let dir = tempfile::tempdir().unwrap();

    {
        let mut db = Db::open(dir.path()).unwrap();
        for i in 0..2000u32 {
            db.put(format!("key{i:05}").into_bytes(), vec![b'x'; 200]).unwrap();
        }
        // confirm the test actually forced a flush before dropping
        assert!(std::fs::read_dir(dir.path()).unwrap()
            .any(|e| e.unwrap().path().extension().map_or(false, |ext| ext == "sst")));
    }

    let mut db = Db::open(dir.path()).unwrap();
    assert_eq!(db.get(b"key00000").unwrap(), Some(vec![b'x'; 200]));
    assert_eq!(db.get(b"key01999").unwrap(), Some(vec![b'x'; 200]));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test db::tests::reopen`
Expected: FAIL (likely `Wal::create` panicking/erroring on an
already-existing `wal.log`, and no SSTable discovery yet).

- [ ] **Step 3: Implement**

Requirements for the new `open`:
- List `dir`'s entries; collect filenames matching `NNNNNN.sst`, parse
  the sequence number, sort **descending** (newest first), `SsTable::open`
  each and collect into `sstables` in that order. Set `next_seq` to one
  past the highest sequence number found (or `0` if none).
- For the WAL: if `dir.join("wal.log")` exists, call `Wal::replay` on it
  to get the entries, fold them into a fresh `MemTable` (`Put` → `put`,
  `Delete` → `delete`, in file order so later entries correctly
  overwrite earlier ones for the same key), then reopen the same file
  with `Wal::open_append` (Task 4) so future writes append rather than
  clobbering it. If it doesn't exist, `Wal::create` a new one and start
  with an empty memtable — same as before.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test db::tests`
Expected: all PASS, including the Task 8 tests (no regressions).

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "Add recovery: replay WAL and rediscover SSTables on open"
```

---

### Task 10: Compaction

**Files:**
- Create: `src/compaction.rs`
- Modify: `src/lib.rs` — add `mod compaction;`
- Modify: `src/db.rs` — trigger compaction from `put`/`delete`
- Modify: `src/sstable.rs` — add a `keys()` accessor (see Step 3)

**Concept:** why compaction is what keeps read latency and disk usage
bounded (without it, `get` would eventually scan an ever-growing list of
SSTables, most of them full of dead/overwritten data), and the key
invariant: when the same key appears in multiple input SSTables, the one
from the **newer** file wins — which is exactly why `db.rs` keeps
`sstables` ordered newest-first.

**Interfaces produced:**
```rust
// Merges `inputs` (ordered newest-first, same convention as
// Db::sstables) into a single new SSTable at `output_path`, keeping
// only the newest value for each key and dropping tombstones (a
// tombstone at the *newest* input for a key means "deleted" — the key
// disappears entirely from the output, since there's nothing older
// than these inputs left to shadow). Uses the same temp+fsync+rename
// pattern as write_sstable.
pub fn compact(
    inputs: &mut [crate::sstable::SsTable],
    output_path: &std::path::Path,
) -> crate::Result<()>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sstable::{write_sstable, SsTable};
    use crate::memtable::Value;

    #[test]
    fn newer_input_wins_and_tombstones_disappear() {
        let dir = tempfile::tempdir().unwrap();

        // older file: a=1, b=2
        let older_path = dir.path().join("000001.sst");
        write_sstable(&older_path, vec![
            (b"a".to_vec(), Value::Data(b"1".to_vec())),
            (b"b".to_vec(), Value::Data(b"2".to_vec())),
        ].into_iter()).unwrap();

        // newer file: a=99 (overwrite), b=tombstone (delete), c=3 (new)
        let newer_path = dir.path().join("000002.sst");
        write_sstable(&newer_path, vec![
            (b"a".to_vec(), Value::Data(b"99".to_vec())),
            (b"b".to_vec(), Value::Tombstone),
            (b"c".to_vec(), Value::Data(b"3".to_vec())),
        ].into_iter()).unwrap();

        let mut inputs = vec![SsTable::open(&newer_path).unwrap(), SsTable::open(&older_path).unwrap()];
        let output_path = dir.path().join("000003.sst");
        compact(&mut inputs, &output_path).unwrap();

        let mut merged = SsTable::open(&output_path).unwrap();
        match merged.get(b"a").unwrap() {
            Some(Value::Data(v)) => assert_eq!(v, b"99"),
            other => panic!("expected a=99, got {other:?}"),
        }
        assert!(merged.get(b"b").unwrap().is_none()); // tombstoned key is gone
        match merged.get(b"c").unwrap() {
            Some(Value::Data(v)) => assert_eq!(v, b"3"),
            other => panic!("expected c=3, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test compaction::tests`
Expected: fails to compile.

- [ ] **Step 3: Implement**

Requirements:
- You need each input SSTable's full sorted key list to merge them —
  add a method to `SsTable` (in `sstable.rs`) if you don't already have
  one that exposes its in-memory index's keys in order, e.g.
  `pub fn keys(&self) -> impl Iterator<Item = &Vec<u8>>` returning the
  index you built in Task 7 (it's already sorted).
- Do a standard k-way merge over `inputs` (already newest-first) by key:
  for each distinct key across all inputs, take the value from
  whichever input is **earliest in the `inputs` slice** (that's the
  newest one, by the newest-first convention) that contains it. If that
  value is `Value::Tombstone`, omit the key from the output entirely;
  otherwise include `(key, Value::Data(v))`.
- Feed the merged, sorted `(key, Value)` sequence into `write_sstable`
  (Task 6) targeting `output_path`.
- In `db.rs`: after a flush, if `self.sstables.len()` exceeds a chosen
  threshold constant (e.g. `const COMPACTION_THRESHOLD: usize = 4`),
  call `compact` on all of them into one new file at the next sequence
  number, delete the old SSTable files from disk, and replace
  `self.sstables` with a single-element vec containing the newly opened
  merged table.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test compaction::tests`
Expected: PASS. Also rerun `cargo test` (full suite) to confirm the
`db.rs` wiring didn't break Tasks 8–9's tests.

- [ ] **Step 5: Commit**

```bash
git add src/compaction.rs src/sstable.rs src/db.rs src/lib.rs
git commit -m "Add compaction: k-way merge with newest-wins and tombstone drop"
```

---

### Task 11: Range scan

**Files:**
- Modify: `src/db.rs`
- Modify: `src/sstable.rs` — add a range-bounded entry accessor (see Step 3)

**Concept:** why a scan can't just concatenate each source's matching
entries — the same key can appear in the memtable *and* multiple
SSTables, so a scan needs the same "newest wins" merge logic as a
point `get`, just applied across a range instead of one key.

**Interfaces produced:**
```rust
impl Db {
    // Returns all live (non-tombstoned) key-value pairs with
    // key >= start and key < end, sorted by key ascending.
    pub fn scan(&mut self, start: &[u8], end: &[u8]) -> crate::Result<Vec<(Vec<u8>, Vec<u8>)>>;
}
```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn scan_merges_memtable_and_sstables_newest_wins() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::open(dir.path()).unwrap();

    // force these into an SSTable
    for i in 0..2000u32 {
        db.put(format!("key{i:05}").into_bytes(), vec![b'x'; 200]).unwrap();
    }
    // overwrite one flushed key from the (now fresh) memtable
    db.put(b"key00001".to_vec(), b"overwritten".to_vec()).unwrap();
    // delete another flushed key from the memtable
    db.delete(b"key00002".to_vec()).unwrap();

    let results = db.scan(b"key00000", b"key00005").unwrap();
    let map: std::collections::HashMap<_, _> = results.into_iter().collect();

    assert_eq!(map.get(b"key00000".as_slice()), Some(&vec![b'x'; 200]));
    assert_eq!(map.get(b"key00001".as_slice()), Some(&b"overwritten".to_vec()));
    assert_eq!(map.get(b"key00002".as_slice()), None); // deleted, must not appear
    assert_eq!(map.get(b"key00003".as_slice()), Some(&vec![b'x'; 200]));
    assert_eq!(map.len(), 4); // 00 through 04 minus the deleted 02
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test db::tests::scan`
Expected: fails to compile (`scan` doesn't exist).

- [ ] **Step 3: Implement**

Requirements:
- Simplest correct approach for Phase 1 (true streaming iterators across
  sources are a fine stretch goal later, not required now): collect
  candidates from every source into one `BTreeMap<Vec<u8>, Value>` in
  **oldest-to-newest** order (iterate `self.sstables` in reverse — i.e.
  oldest first — then the memtable last), so that inserting into the
  map naturally lets a newer source's entry for the same key overwrite
  an older one, matching newest-wins.
- Add a method to `SsTable` (or reuse `keys()` from Task 10 plus `get`)
  to enumerate its entries restricted to `[start, end)` — a simple
  approach is to binary-search the in-memory index for the start bound,
  then walk forward through the index (which is already sorted) reading
  each entry until the key reaches `end`.
- After building the merged map, filter to `key >= start && key < end`,
  drop any `Value::Tombstone` entries, convert remaining
  `Value::Data(v)` entries to `(key, v)`, and return them sorted by key
  (a `BTreeMap`'s iteration order already guarantees this).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test db::tests`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs src/sstable.rs
git commit -m "Add range scan merging memtable and SSTables"
```

---

### Task 12: CLI REPL

**Files:**
- Create: `src/bin/cli.rs`

**Concept:** nothing new conceptually — this is where you get to
actually *use* the thing you built, which is worth doing before diving
into Task 13's harder tests.

**Interfaces produced:** a `ferrite` binary (via `cargo run --bin cli`)
with commands `put <key> <value>`, `get <key>`, `delete <key>`,
`scan <start> <end>`, `exit`.

- [ ] **Step 1: Implement the REPL**

No TDD here (it's an interactive tool, not a unit) — but exercise it
manually per Step 2 below. Requirements:
- Take the database directory as `std::env::args().nth(1)`, defaulting
  to `./ferrite-data` if not given.
- `Db::open` it once at startup.
- Loop: print a `> ` prompt, read a line from stdin, split on
  whitespace, dispatch to the matching `Db` method, print the result
  (or `(nil)` for a missing key on `get`), print any `Error` on failure
  without crashing the loop, `exit`/`quit` breaks the loop.

- [ ] **Step 2: Manually exercise it**

Run: `cargo run --bin cli`
Try: `put foo bar`, `get foo`, `delete foo`, `get foo` (should print
`(nil)`), `scan a z`. Then quit, rerun `cargo run --bin cli` again
against the same directory and `get foo` — confirms persistence
end-to-end from the CLI, not just from tests.

- [ ] **Step 3: Commit**

```bash
git add src/bin/cli.rs
git commit -m "Add CLI REPL for manual testing"
```

---

### Task 13: Crash-injection and property-based hardening tests

**Files:**
- Create: `tests/crash_recovery.rs`
- Create: `tests/proptest_model.rs`
- Modify: `Cargo.toml` — add `proptest = "1"` under `[dev-dependencies]`
  (if not already added in Task 4)

**Concept:** why these two tests matter more than everything before
them — `crash_recovery` proves the specific torn-write handling from
Task 4 actually protects a *real* `Db`, not just the isolated `Wal`, and
the `proptest` model test is the same fuzzing technique real storage
engines use: instead of hand-picking test cases, it generates thousands
of random operation sequences and checks the invariant "Ferrite always
agrees with a trivial reference `HashMap`" — the spec's actual
definition of correctness for the read/write path.

- [ ] **Step 1: Write the crash-recovery integration test**

`tests/crash_recovery.rs`:
```rust
use std::io::Write;

#[test]
fn torn_wal_write_does_not_corrupt_earlier_data() {
    let dir = tempfile::tempdir().unwrap();

    {
        let mut db = ferrite::Db::open(dir.path()).unwrap();
        db.put(b"safe".to_vec(), b"data".to_vec()).unwrap();
    }

    // simulate a crash mid-append: hand-append a well-formed entry to
    // the WAL, then truncate it partway through
    let wal_path = dir.path().join("wal.log");
    let entry_bytes = ferrite::wal::encode_entry(
        &ferrite::wal::WalEntry::Put { key: b"torn".to_vec(), value: b"nope".to_vec() },
    );
    let mut file = std::fs::OpenOptions::new().append(true).open(&wal_path).unwrap();
    file.write_all(&entry_bytes[..entry_bytes.len() - 3]).unwrap();
    drop(file);

    let mut db = ferrite::Db::open(dir.path()).unwrap();
    assert_eq!(db.get(b"safe").unwrap(), Some(b"data".to_vec())); // earlier data intact
    assert_eq!(db.get(b"torn").unwrap(), None); // torn entry correctly discarded
}
```
This relies on `pub mod wal;` from Task 3 and the `pub use db::Db;`
re-export from Task 8 — no further visibility changes needed.

- [ ] **Step 2: Run it, confirm it passes against your existing implementation**

Run: `cargo test --test crash_recovery`
Expected: PASS (this exercises code from Tasks 4/8/9 together — if it
fails, the bug is in how they compose, not in any single task).

- [ ] **Step 3: Write the property-based model test**

`tests/proptest_model.rs`:
```rust
use proptest::prelude::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum Op {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    Get(Vec<u8>),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let key = prop::collection::vec(any::<u8>(), 1..4)
        .prop_map(|v| v.into_iter().map(|b| b % 5).collect::<Vec<u8>>()); // small keyspace -> forces collisions/overwrites
    prop_oneof![
        (key.clone(), prop::collection::vec(any::<u8>(), 0..8)).prop_map(|(k, v)| Op::Put(k, v)),
        key.clone().prop_map(Op::Delete),
        key.prop_map(Op::Get),
    ]
}

proptest! {
    #[test]
    fn matches_hashmap_reference(ops in prop::collection::vec(op_strategy(), 1..200)) {
        let dir = tempfile::tempdir().unwrap();
        let mut db = ferrite::Db::open(dir.path()).unwrap();
        let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        for op in ops {
            match op {
                Op::Put(k, v) => {
                    db.put(k.clone(), v.clone()).unwrap();
                    model.insert(k, v);
                }
                Op::Delete(k) => {
                    db.delete(k.clone()).unwrap();
                    model.remove(&k);
                }
                Op::Get(k) => {
                    let expected = model.get(&k).cloned();
                    let actual = db.get(&k).unwrap();
                    prop_assert_eq!(actual, expected);
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run it**

Run: `cargo test --test proptest_model`
Expected: PASS. If it fails, `proptest` prints a minimized failing case
— read it as a bug report against the specific task whose logic it
exercises (usually `get`'s memtable/SSTable precedence in Task 8, or the
flush/compaction boundary in Tasks 8/10).

- [ ] **Step 5: Run the full suite and commit**

Run: `cargo test`
Expected: every test across every module and every file in `tests/`
passes.
```bash
git add Cargo.toml Cargo.lock tests/crash_recovery.rs tests/proptest_model.rs src/lib.rs
git commit -m "Add crash-injection and property-based model tests"
```

---

## Definition of done

All 13 tasks checked off, `cargo test` fully green, the CLI persists
data across restarts when exercised manually, and — per the spec — you
can explain in your own words what the WAL/SSTable formats are for and
why the CRC + torn-write handling and temp+fsync+rename patterns are
each necessary. That last part isn't a formality: it's the actual goal
of Phase 1.
