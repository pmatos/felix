# felix

Real-time profiling TUI for [FEX-Emu](https://github.com/FEX-Emu/FEX), an x86-on-ARM64 emulator.

felix attaches to a running FEX process by PID, reads profiling data from POSIX shared memory, and displays live JIT compilation statistics, memory usage, and load histograms. It also supports recording sessions to disk and replaying them with full playback controls.

## Usage

```
felix live <pid>                      # Monitor a live FEX process
felix live <pid> -r session.felixr    # Monitor + record
felix replay session.felixr           # Replay a recording
felix record <pid> -o session.felixr  # Headless recording
felix watch                           # Auto-detect FEX processes
felix pick                            # Pick a FEX process interactively
felix export session.felixr -o out.csv # Export to CSV
```

### `pick` subcommand

When a game spawns many FEX processes, `pick` shows a tree view of all running FEX processes with their parent-child relationships and command lines, so you can identify and select the right one:

```
Running FEX processes:
  [0] PID 18553  /bin/sh -c .../steam-launch-wrapper ...
      ├── [1] PID 18554  .../proton waitforexitandrun ...
      │   └── [2] PID 18760  Z:\...\MirrorsEdge.exe
      └── [3] PID 18555  .../steamwebhelper ...
Select process [0-3] (q to quit):
```

Root processes are highlighted in green, child PIDs in cyan.

### Replay controls

| Key           | Action              |
|---------------|---------------------|
| `Space`       | Pause / resume      |
| `Left`/`Right`| Seek backward/forward |
| `+`/`-`       | Speed up/down       |
| `Home`/`End`  | Seek to start/end   |

### General controls

| Key       | Action                    |
|-----------|---------------------------|
| `q`       | Quit                      |
| `Up`/`Down` | Select panel           |
| `Enter`   | Collapse/expand panel     |

## Building

```
cargo build --release
```

### Running tests

```
cargo test
cargo clippy -- -D warnings
```

## What does felix measure?

felix reads FEX-Emu's shared memory profiling stats which track:

- **JIT compilation time** - time spent translating x86 code to ARM64
- **Signal handler time** - time in signal handlers (e.g. SIGILL traps)
- **Softfloat fallback count** - FP operations using software emulation
- **SMC (self-modifying code) count** - code invalidation events
- **SIGBUS count** - bus error signals
- **Cache statistics** - JIT code cache misses and lock contention
- **Memory breakdown** - per-region RSS from `/proc/<pid>/smaps`

Note: JIT load measures **compilation overhead**, not total CPU utilization. Once a game finishes its initial JIT compilation, load drops to zero even while the game runs normally on cached translated code.

## Requirements

- **Platform**: ARM64 Linux (primary), x86_64 Linux (dev/testing with stubs)
- **Rust edition**: 2024
- **FEX stats version**: Must match `FEXCore::Profiler::STATS_VERSION` (currently 2)

## License

MIT
