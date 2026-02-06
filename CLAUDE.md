# felix

## Overview

felix is a Rust/ratatui real-time profiling TUI for [FEX-Emu](https://github.com/FEX-Emu/FEX), an x86-on-ARM64 emulator. It attaches to a running FEX process by PID, reads profiling data from POSIX shared memory, and displays live JIT compilation statistics, memory usage, and load histograms. It also supports recording sessions to disk and replaying them with full playback controls.

## Usage

```
cargo run -- live <pid>                      # Monitor a live FEX process
cargo run -- live <pid> -r session.felixr    # Monitor + record
cargo run -- replay session.felixr           # Replay a recording
cargo run -- record <pid> -o session.felixr  # Headless recording
cargo run -- watch                           # Auto-detect FEX processes
cargo run -- pick                            # Pick a FEX process interactively
cargo run -- export session.felixr -o out.csv # Export to CSV
```

## Build

```
cargo build --release
cargo test
cargo clippy -- -D warnings
```

Strict lints are enforced: `#![deny(warnings, clippy::all, clippy::pedantic)]`.

## Architecture

```
src/
  main.rs              # CLI (clap), subcommand dispatch, event loops
  datasource.rs        # DataSource trait (abstracts live vs replay)
  fex/
    types.rs           # FEX shared memory structs (repr(C, align(16)))
    shm.rs             # POSIX shm reader with volatile/atomic reads
    platform.rs        # ARM64 cycle counter, memory barriers
    smaps.rs           # /proc/<pid>/smaps parser for FEX memory regions
  sampler/
    thread_stats.rs    # Per-thread delta computation
    mem_stats.rs       # Background smaps sampling thread
    accumulator.rs     # Load calculation, histogram entries
  recording/
    format.rs          # File format (postcard + zstd, length-prefixed frames)
    writer.rs          # Streaming recording writer
    reader.rs          # Recording reader + ReplaySource
  tui/
    app.rs             # App state, panel management, render dispatch
    input.rs           # Key bindings (live + replay modes)
    layout.rs          # Collapsible panel layout
    theme.rs           # Colors, Unicode block characters
    replay_controls.rs # Playback speed, seek, progress bar
    panels/
      header.rs        # Status bar (PID, FEX version, type, head, size)
      jit_stats.rs     # Per-thread load bars + aggregate counters
      mem_stats.rs     # FEX memory breakdown
      histogram.rs     # Scrolling JIT load histogram
```

### Key Design Decisions

- **Shared memory safety**: All reads from mmap'd memory use `ptr::read_volatile`. 16-byte aligned copies exploit ARMv8.4 single-copy atomicity (`u128` loads on aarch64).
- **Recording format**: postcard serialization + zstd streaming compression. Length-prefixed frames for streaming read/write.
- **DataSource trait**: Abstracts live vs replay so the TUI code is identical in both modes.
- **Background smaps thread**: `/proc/<pid>/smaps` parsing runs on a separate thread since it's expensive I/O.

## FEX Shared Memory Layout

The canonical definition lives in FEX at `FEXCore/include/FEXCore/Utils/SHMStats.h`. felix mirrors these structs in `src/fex/types.rs`. The SHM segment is located at `/dev/shm/fex-<pid>-stats`.

### ThreadStatsHeader (64 bytes, starts at offset 0)

| Offset | FEX C++ field | Felix Rust field | Type |
|--------|---------------|------------------|------|
| 0 | `Version` | `version` | `u8` |
| 1 | `app_type` | `app_type` | `u8` (AppType enum) |
| 2 | `ThreadStatsSize` | `thread_stats_size` | `u16` |
| 4 | `fex_version` | `fex_version` | `[u8; 48]` (null-terminated) |
| 52 | `Head` | `head` | `u32` (atomic in FEX) |
| 56 | `Size` | `size` | `u32` (atomic in FEX) |
| 60 | `Pad` | `pad` | `u32` |

`Head` is the byte offset of the first `ThreadStats` entry. `Size` is the total SHM size in bytes (grows as threads are created).

### ThreadStats (80 bytes, 16-byte aligned, linked list via `Next`)

| Offset | FEX C++ field | Felix Rust field | Type | Semantics |
|--------|---------------|------------------|------|-----------|
| 0 | `Next` | `next` | `u32` (atomic) | Byte offset of next entry, 0 = end |
| 4 | `TID` | `tid` | `u32` (atomic) | Thread ID |
| 8 | `AccumulatedJITTime` | `accumulated_jit_time` | `u64` | Cycles in JIT compiler |
| 16 | `AccumulatedSignalTime` | `accumulated_signal_time` | `u64` | Cycles in signal handlers |
| 24 | `AccumulatedSIGBUSCount` | `sigbus_count` | `u64` | SIGBUS event count |
| 32 | `AccumulatedSMCCount` | `smc_count` | `u64` | Self-modifying code invalidations |
| 40 | `AccumulatedFloatFallbackCount` | `float_fallback_count` | `u64` | Softfloat fallback count |
| 48 | `AccumulatedCacheMissCount` | `accumulated_cache_miss_count` | `u64` | JIT cache misses |
| 56 | `AccumulatedCacheReadLockTime` | `accumulated_cache_read_lock_time` | `u64` | Cycles holding read lock |
| 64 | `AccumulatedCacheWriteLockTime` | `accumulated_cache_write_lock_time` | `u64` | Cycles holding write lock |
| 72 | `AccumulatedJITCount` | `accumulated_jit_count` | `u64` | Number of JIT compilations |

### Cycle counter

- FEX measures time using `CNTVCT_EL0` (arm64) / `__rdtscp` (x86_64) via `GetCycleCounter()`.
- felix reads the corresponding frequency from `CNTFRQ_EL0` (arm64) to convert cycles to real time. On x86_64 the frequency is stubbed to 1.
- All time fields are in **unscaled counter ticks**, not CPU clock cycles.

### Reading conventions

- felix reads the header and thread stats via `ptr::read_volatile` (`src/fex/shm.rs`).
- On aarch64, `ThreadStats` copies use 128-bit aligned volatile loads to exploit ARMv8.4 single-copy atomicity.
- The thread list is a singly-linked list starting at `head` byte offset, terminated by `next == 0`.
- FEX may grow the SHM via `ftruncate`; felix detects this via `header.size` and remaps (`ShmReader::check_resize`).

## Key Details

- **License**: MIT
- **Target platform**: ARM64 Linux (primary), x86_64 Linux (dev/testing with stub implementations)
- **Stats version**: Must match `FEXCore::Profiler::STATS_VERSION` (currently 2)
- **Rust edition**: 2024
- **App types supported**: Linux32, Linux64, arm64ec, wow64
