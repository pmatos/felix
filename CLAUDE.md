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

## Key Details

- **License**: MIT
- **Target platform**: ARM64 Linux (primary), x86_64 Linux (dev/testing with stub implementations)
- **Stats version**: Must match `FEXCore::Profiler::STATS_VERSION` (currently 2)
- **Rust edition**: 2024
- **App types supported**: Linux32, Linux64, arm64ec, wow64
