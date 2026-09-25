# Ferrite — Phase 1: Single-Node Storage Engine

## Project context

Ferrite is a long-term learning project: a distributed key-value database
built in phases, aimed at pushing the author's skills in Rust and
distributed systems (both new to them at project start). The full
project decomposes into:

1. **Phase 1 (this spec)** — single-node, embedded, crash-safe LSM-tree
   storage engine.
2. Phase 2 — networking layer (async RPC between nodes/clients).
3. Phase 3 — Raft consensus, replicating the Phase 1 engine across nodes.
4. Phase 4 — cluster hardening: snapshots, membership changes, client
   library/CLI.

Each phase gets its own spec, plan, and build cycle. This document
covers Phase 1 only.

**Workflow model:** the author writes all production code themselves.
Claude's role is teaching (explaining concepts and API shape before each
piece is written) and review (reading the author's code and tests like a
PR reviewer, not rewriting it). This is not an agent-executed build.

## Goals

- Produce a working, durable, crash-safe embedded key-value store with
  `put` / `get` / `delete` / `scan` (range).
- Use the build to teach: Rust fundamentals (ownership, `Result`/`Option`,
  structs/enums/traits, byte-level I/O), and core storage-engine
  concepts (write-ahead logging, sorted immutable files, compaction,
  crash recovery) that Phase 3 (Raft) and real distributed databases
  depend on.
- Every milestone should be independently testable and reviewable.

## Non-goals (explicitly deferred)

- Networking / RPC (Phase 2).
- Concurrency: no threads, no locks, no background compaction thread.
  Compaction runs synchronously, triggered inline by the write that
  crosses the threshold. Concurrent access is a Phase 2 concern (the
  `Db` will get wrapped for concurrent use there).
- Transactions spanning multiple keys.
- TTL/expiry, compression, encryption.
- Bloom filters and sparse indexing — noted as stretch goals only, not
  required for Phase 1 to be considered complete.
- Generic key/value types — keys and values are `Vec<u8>` throughout, to
  keep the focus on storage-engine mechanics rather than Rust's generics
  and trait-bound system.

## Architecture

```
put(k,v) -> append to WAL (fsync) -> insert into memtable (BTreeMap, in-RAM, sorted)
memtable exceeds size threshold -> flush: write sorted contents to a new SSTable file, clear memtable
get(k)   -> check memtable -> check SSTables newest-to-oldest -> first hit wins (tombstone = deleted)
compaction -> triggered when SSTable count crosses a threshold -> merge N files into 1,
              drop overwritten/deleted keys -> write new file (tmp + fsync + rename), delete old ones
```

On startup (`Db::open`):
1. Discover existing SSTable files on disk by filename sequence number.
2. Replay the WAL into a fresh memtable, stopping at the first corrupt
   (torn) entry rather than erroring — a torn final write is an expected
   crash artifact, not corruption of earlier data.

## On-disk formats

All formats are hand-rolled binary (not `serde`/`bincode`) — designing
the byte layout is a deliberate part of the learning goal. All
multi-byte integers are little-endian.

### WAL entry

```
crc32(u32) | entry_len(u32) | op(u8: 0=Put, 1=Delete) | key_len(u32) | key | value_len(u32) | value
```
- `value_len`/`value` are omitted when `op == Delete`.
- `crc32` is computed over everything after itself in the entry. On
  replay, a CRC mismatch on the *last* entry in the file is treated as a
  torn write (truncate and stop); a CRC mismatch on any earlier entry is
  a genuine corruption error.

### SSTable

```
[data block]  sorted sequence of: key_len(u32) | key | value_len(u32) | value_or_tombstone
[index block] sorted sequence of: key_len(u32) | key | offset(u64)      (dense: one entry per key)
[footer]      index_block_offset(u64) | index_block_len(u64) | magic(u32)
```
- Reader loads the footer first (fixed size, known offset from EOF),
  then the index block, and binary-searches the index for lookups.
- Dense indexing (not sparse) for Phase 1 — sparse indexing is a stretch
  goal once correctness is solid.

### File naming / ordering

Files are named `NNNNNN.sst` with a monotonically increasing sequence
number; higher number = newer. This doubles as the manifest — no
separate manifest file is needed for Phase 1. New files (both flush
output and compaction output) are written to a temp filename, fsynced,
then atomically renamed into place, so a crash mid-write never leaves a
partially-written file visible under its real name.

## Module layout

```
Cargo.toml
src/
  lib.rs         public Db API: open, put, get, delete, scan, close
  wal.rs         entry encode/decode, append, replay (with torn-write handling)
  memtable.rs    BTreeMap<Vec<u8>, Value> wrapper + byte-size tracking
  sstable.rs     writer + reader (footer/index/binary search)
  compaction.rs  N-way merge of sorted SSTables, tombstone/overwrite elimination
  db.rs          orchestration: wires WAL+memtable+SSTables, recovery-on-open, flush/compact triggers
  error.rs       hand-rolled Error enum + From impls (no `thiserror`)
  bin/
    cli.rs       minimal REPL over Db (get/put/delete/scan/exit) for manual testing
```

`Value` here means "either a byte payload or a tombstone marker" — used
internally by the memtable and SSTable; the public `Db` API surfaces
`Option<Vec<u8>>` and does not leak tombstones to callers.

## Error handling

A hand-rolled `Error` enum (not a crate like `thiserror`, since writing
the `From` impls by hand is part of the Rust learning goal):

```rust
enum Error {
    Io(std::io::Error),
    Corruption(String), // CRC mismatch on a non-final WAL entry, bad SSTable footer/magic, etc.
}
type Result<T> = std::result::Result<T, Error>;
```

`NotFound` is not an error — `get` returns `Result<Option<Vec<u8>>>`, and
`None` is a normal, successful "key absent" result.

## Testing strategy

- **Unit tests per module**: WAL entry encode/decode round-trips,
  memtable ordering and size tracking, SSTable writer/reader round-trips
  and binary search correctness.
- **Integration test (restart)**: open a `Db`, write N keys (mix of
  puts/deletes), drop it (simulating process exit), reopen at the same
  path, verify all data is present and correct — this is what proves WAL
  replay and SSTable discovery actually work together.
- **Crash-injection test**: write WAL entries, truncate the file at a
  byte offset partway through the last entry, open the `Db`, verify
  recovery discards only the torn entry and all earlier data survives
  intact.
- **Property-based model test** (`proptest`): generate random sequences
  of put/delete/get/scan operations, run them against both `Db` and a
  reference `HashMap<Vec<u8>, Vec<u8>>` model, assert they always agree.
  This is the same technique used to fuzz production storage engines.

## Milestones

Each milestone: concept walkthrough (Claude) → implementation (author)
→ review (Claude) → move on. Milestones are independently testable.

0. **Rust fundamentals + toolchain setup.** Install `rustup`. Small,
   throwaway exercises (not part of `ferrite`) covering ownership/
   borrowing, `Option`/`Result`/`?`, structs/enums/traits, slices and
   byte manipulation — targeted at exactly what the later milestones
   need.
1. **WAL**: entry encode/decode, append, replay with torn-write
   detection.
2. **MemTable**: `BTreeMap` wrapper with byte-size tracking for the
   flush threshold.
3. **SSTable**: writer (memtable → sorted file + index + footer) and
   reader (footer → index → binary search).
4. **Db orchestration**: wire `put`/`get`/`delete` through WAL +
   memtable + SSTables (newest-first read path); flush on threshold.
5. **Recovery**: `Db::open` replays WAL and discovers SSTables by
   filename sequence.
6. **Compaction**: N-way merge of SSTables, dropping obsolete/tombstoned
   keys, crash-safe file swap.
7. **Range scan**: `scan(start..end)` merging iterators across memtable
   + SSTables in key order.
8. **CLI**: minimal REPL binary for manual poking.
9. **Hardening**: crash-injection tests + `proptest` model tests.
   Stretch goals (optional, not required for Phase 1 completion): bloom
   filter, sparse index.

## Definition of done (Phase 1)

- All milestones 0–9 complete; full test suite (unit + integration +
  crash-injection + property-based) passes.
- The CLI can durably store and retrieve data across process restarts.
- Author can explain, in their own words, what each on-disk format is
  for and why the crash-safety measures (CRC + torn-write handling,
  tmp+fsync+rename) are necessary — this is a learning project, so
  understanding is part of "done," not just passing tests.
