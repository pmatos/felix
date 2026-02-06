# WTF (what-the-FEX) Rewrite Plan — Rust + Ratatui

## Context

WTF is a real-time profiling TUI for [FEX-Emu](https://github.com/FEX-Emu/FEX), an x86-on-ARM64
emulator. It attaches to a running FEX process by PID, reads profiling data from a POSIX shared
memory region (`/dev/shm/fex-<pid>-stats`), and displays live JIT compilation statistics, memory
usage breakdowns, and load histograms in a terminal UI.

This plan covers a ground-up rewrite in Rust, replacing the C++20/ncurses implementation with
Rust/ratatui, and adding recording and replay capabilities.

### Goals

1. **Feature parity** with the existing C++ TUI (three panels: JIT stats, memory, histogram)
2. **Recording** — capture all sampled data to disk for offline analysis
3. **Replay** — play back recordings in the same TUI with playback controls
4. **Extensibility** — clean architecture for adding new stat types (CPU state, emulated
   instructions, etc.)
5. **Correctness** — proper handling of shared memory safety (volatile/atomic reads)

### Technology Stack

| Component | Crate | Purpose |
|---|---|---|
| TUI framework | `ratatui` + `crossterm` | Terminal rendering, widgets, input |
| CLI | `clap` (derive) | Subcommands and argument parsing |
| Shared memory | `nix` | `shm_open`, `mmap`, `fstat` |
| Struct casting | `zerocopy` | Safe reinterpretation of mmap'd bytes |
| smaps parsing | manual (BufReader) | Parse `/proc/<pid>/smaps` for FEX memory regions |
| Signal handling | `signal-hook` | SIGINT/SIGQUIT graceful shutdown |
| Serialization | `postcard` + `serde` | Recording format (compact, stable wire format) |
| Compression | `zstd` | Streaming compression of recordings |
| Process watch | `libc::syscall(SYS_pidfd_open)` | Detect FEX process exit |
| Timestamps | `std::time::Instant` | Monotonic sample timestamps |
| Error handling | `anyhow` | Application-level error propagation |
| Number formatting | `num-format` | Comma-separated integer display |

---

## Phase 0: Project Scaffolding

### 0.0 — Archive the C++ implementation

Move the existing C++ source code into a `legacy/` directory so the repo root is clean for the
Rust project. This preserves the code as a reference during the rewrite without interfering with
`Cargo.toml`, `src/`, or the Rust build.

```
mkdir legacy
git mv Src/ legacy/Src/
git mv CMakeLists.txt legacy/CMakeLists.txt
git mv watch.sh legacy/watch.sh
```

Also clean up the CMake build artifacts that are currently untracked (and should not be committed):
```
rm -rf CMakeCache.txt CMakeFiles/ Makefile build.ninja cmake_install.cmake compile_commands.json
```

The built `WTF` binary is also untracked — delete it or add it to `.gitignore`.

Add to `.gitignore`:
```
/target/
/WTF
*.ninja*
CMakeCache.txt
CMakeFiles/
compile_commands.json
```

Commit this as a standalone step before any Rust code is written.

### 0.1 — Initialize Cargo project

Create the Rust project at the repo root (not in a subdirectory — this is now the primary build).

```
├── Cargo.toml
├── legacy/            # Archived C++ implementation (reference only)
│   ├── Src/
│   ├── CMakeLists.txt
│   └── watch.sh
├── src/
│   ├── main.rs          # CLI entrypoint, subcommand dispatch
│   ├── fex/
│   │   ├── mod.rs
│   │   ├── types.rs     # FEX shared memory struct definitions
│   │   ├── shm.rs       # Shared memory open/mmap/read
│   │   ├── smaps.rs     # /proc/<pid>/smaps parser
│   │   └── platform.rs  # ARM64 cycle counter, memory barriers
│   ├── sampler/
│   │   ├── mod.rs
│   │   ├── thread_stats.rs  # Thread stats sampling + delta computation
│   │   ├── mem_stats.rs     # Memory stats (smaps-based) sampling
│   │   └── accumulator.rs   # Load calculation, histogram data
│   ├── recording/
│   │   ├── mod.rs
│   │   ├── format.rs    # Recording file format types
│   │   ├── writer.rs    # Streaming writer (postcard + zstd)
│   │   └── reader.rs    # Streaming reader (decompress + deserialize)
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── app.rs       # Application state, event loop
│   │   ├── input.rs     # Keyboard input handling
│   │   ├── layout.rs    # Panel layout and collapse logic
│   │   ├── theme.rs     # Color scheme, Unicode symbol sets
│   │   ├── panels/
│   │   │   ├── mod.rs
│   │   │   ├── jit_stats.rs   # JIT stats panel
│   │   │   ├── mem_stats.rs   # Memory usage panel
│   │   │   ├── histogram.rs   # Load histogram panel
│   │   │   └── header.rs      # Status bar (PID, FEX version, mode)
│   │   └── replay_controls.rs # Playback UI (seek bar, speed, etc.)
│   └── datasource.rs    # Trait abstracting live vs replay data
```

### 0.2 — Cargo.toml dependencies

```toml
[package]
name = "wtf"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
crossterm = { version = "0.28", features = ["event-stream"] }
libc = "0.2"
nix = { version = "0.29", features = ["mman", "fs"] }
num-format = "0.4"
postcard = { version = "1", features = ["use-std"] }
ratatui = "0.30"
serde = { version = "1", features = ["derive"] }
signal-hook = "0.3"
zerocopy = { version = "0.8", features = ["derive"] }
zstd = "0.13"
```

### 0.3 — CI setup

- GitHub Actions workflow: `cargo clippy`, `cargo test`, `cargo build --release`
- Target: `aarch64-unknown-linux-gnu` (primary), `x86_64-unknown-linux-gnu` (dev/testing)
- Clippy with `-D warnings`

### 0.4 — License and metadata

- MIT license (matching the existing project)
- SPDX headers in all source files

---

## Phase 1: FEX Data Types and Shared Memory Reader

This phase produces a library that can open a FEX process's shared memory, safely read the header
and thread stats, and detect process exit. No TUI yet — validated via integration tests and a
simple `println!` main.

### 1.1 — FEX profiler struct definitions (`fex/types.rs`)

Reimplement the C++ structs from `ThreadStats.hpp` as Rust types. These must match FEX's memory
layout exactly.

```rust
pub const STATS_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppType {
    Linux32 = 0,
    Linux64 = 1,
    WinArm64ec = 2,
    WinWow64 = 3,
}

/// Header at the start of the shared memory region.
/// Fields `head` and `size` are atomically updated by FEX.
#[repr(C)]
pub struct ThreadStatsHeader {
    pub version: u8,
    pub app_type: AppType,
    pub thread_stats_size: u16,
    pub fex_version: [u8; 48],
    pub head: u32,   // AtomicU32 — read with volatile/atomic
    pub size: u32,   // AtomicU32 — read with volatile/atomic
    pub pad: u32,
}

/// Per-thread profiling counters. Linked list via `next` offset.
/// All fields are accumulated monotonically by FEX.
/// The struct size must be a multiple of 16 bytes for atomic copy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[repr(C, align(16))]
pub struct ThreadStats {
    pub next: u32,
    pub tid: u32,
    pub accumulated_jit_time: u64,
    pub accumulated_signal_time: u64,
    pub sigbus_count: u64,
    pub smc_count: u64,
    pub float_fallback_count: u64,
    pub accumulated_cache_miss_count: u64,
    pub accumulated_cache_read_lock_time: u64,
    pub accumulated_cache_write_lock_time: u64,
    pub accumulated_jit_count: u64,
}
```

Key decisions:
- Use `#[repr(C, align(16))]` to match the C++ `static_assert(sizeof(ThreadStats) % 16 == 0)`
- Derive `Serialize`/`Deserialize` on `ThreadStats` for recording (but NOT on `ThreadStatsHeader`
  since that's only read from shm)
- Use `zerocopy::FromBytes` derive to validate layout compatibility at compile time
- Do NOT use Rust `AtomicU32` in the struct definition itself — the struct matches FEX's C++ layout
  which uses `std::atomic<uint32_t>` (same memory representation as plain `u32` on all targets, but
  we read via `read_volatile` or atomic pointer cast)

### 1.2 — Shared memory reader (`fex/shm.rs`)

Encapsulate all shared memory access in a `ShmReader` struct.

```rust
pub struct ShmReader {
    fd: OwnedFd,
    base: *const u8,
    size: usize,
    thread_stats_copy_size: usize,
}
```

Operations:
- `ShmReader::open(pid: i32) -> Result<Self>` — calls `shm_open("/fex-{pid}-stats", O_RDONLY)`,
  `fstat` for initial size, `mmap(PROT_READ, MAP_SHARED)`. Validates minimum size.
- `ShmReader::header(&self) -> HeaderSnapshot` — reads the header using `ptr::read_volatile` on
  individual fields. Returns an owned `HeaderSnapshot` (not a reference into mmap'd memory).
- `ShmReader::read_thread_stats(&self) -> Vec<ThreadStats>` — walks the linked list from
  `header.head`, reads each `ThreadStats` via 16-byte aligned volatile copies, collects into a Vec.
  Bounds-checks offsets against `self.size`.
- `ShmReader::check_resize(&mut self)` — reads `header.size` atomically, calls `munmap` + `mmap`
  if the region grew (matching the C++ `check_shm_update_necessary`).

Safety approach for reading shared memory written by another process:
- NEVER form `&ThreadStatsHeader` or `&ThreadStats` references to the mmap'd memory
- Use `ptr::read_volatile` for all reads from the mmap region
- For the 16-byte atomic copy (matching C++ `__uint128_t` single-copy atomicity on ARMv8.4):

```rust
#[cfg(target_arch = "aarch64")]
unsafe fn volatile_copy_16b_aligned(dst: *mut u8, src: *const u8, len: usize) {
    debug_assert!(len % 16 == 0);
    debug_assert!(src as usize % 16 == 0);
    let n = len / 16;
    let s = src as *const u128;
    let d = dst as *mut u128;
    for i in 0..n {
        d.add(i).write_volatile(s.add(i).read_volatile());
    }
}
```

On x86, fall back to byte-wise `read_volatile` (no single-copy atomicity guarantee needed since
x86 is only used for development/testing, not real FEX profiling).

### 1.3 — ARM64 platform support (`fex/platform.rs`)

```rust
#[cfg(target_arch = "aarch64")]
pub fn cycle_counter_frequency() -> u64 {
    let value: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) value) };
    value
}

#[cfg(target_arch = "aarch64")]
pub fn store_memory_barrier() {
    unsafe { core::arch::asm!("dmb ishst", options(nostack, preserves_flags)) };
}

#[cfg(target_arch = "x86_64")]
pub fn cycle_counter_frequency() -> u64 { 1 }

#[cfg(target_arch = "x86_64")]
pub fn store_memory_barrier() {}
```

### 1.4 — Process lifecycle monitoring

```rust
pub struct ProcessWatcher {
    pidfd: RawFd,
}

impl ProcessWatcher {
    pub fn new(pid: i32) -> Result<Self>;       // syscall(SYS_pidfd_open, pid, 0)
    pub fn has_exited(&self) -> bool;            // poll(pidfd, POLLHUP, timeout=0)
}
```

### 1.5 — Validation

Write tests that:
- Verify struct sizes and alignments match the C++ definitions:
  `assert_eq!(size_of::<ThreadStats>(), 96)` (or whatever the actual C++ size is — verify)
  `assert_eq!(size_of::<ThreadStats>() % 16, 0)`
  `assert_eq!(size_of::<ThreadStatsHeader>(), 64)` (verify against C++)
- Mock a shared memory region in a temp file and verify `ShmReader` can parse it
- Test `cycle_counter_frequency()` returns nonzero on aarch64

---

## Phase 2: Sampling Engine

This phase builds the sampling logic that reads raw thread stats and computes the derived metrics
(deltas, load percentages, histogram data). It mirrors the C++ `AccumulateJITStats()` function
but with cleaner separation of concerns.

### 2.1 — Thread stats sampler (`sampler/thread_stats.rs`)

```rust
pub struct ThreadSampler {
    previous: BTreeMap<u32, ThreadStats>,  // TID -> previous sample
    last_seen: BTreeMap<u32, Instant>,     // TID -> last seen time
    stale_timeout: Duration,               // Default: 10 seconds
}
```

Methods:
- `sample(&mut self, raw_stats: &[ThreadStats], now: Instant) -> SampleResult` — for each thread:
  1. Compute deltas (current - previous) for all accumulated fields
  2. Update `previous` map
  3. Track `last_seen` timestamp
  4. Evict threads not seen for `stale_timeout`
  5. Return the delta stats per thread

```rust
pub struct SampleResult {
    pub timestamp: Instant,
    pub per_thread: Vec<ThreadDelta>,
    pub threads_sampled: usize,
}

pub struct ThreadDelta {
    pub tid: u32,
    pub jit_time: u64,
    pub signal_time: u64,
    pub sigbus_count: u64,
    pub smc_count: u64,
    pub float_fallback_count: u64,
    pub cache_miss_count: u64,
    pub cache_read_lock_time: u64,
    pub cache_write_lock_time: u64,
    pub jit_count: u64,
}
```

### 2.2 — Memory stats sampler (`sampler/mem_stats.rs`)

Parses `/proc/<pid>/smaps` for FEX-specific anonymous memory regions. Runs on its own cadence
(typically 1s) since smaps I/O is expensive.

```rust
pub struct MemSampler {
    pid: i32,
    fd: Option<File>,  // Keep fd open, seek to 0 on each read
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemSnapshot {
    pub total_anon: u64,
    pub jit_code: u64,
    pub op_dispatcher: u64,
    pub frontend: u64,
    pub cpu_backend: u64,
    pub lookup: u64,
    pub lookup_l1: u64,
    pub thread_states: u64,
    pub block_links: u64,
    pub misc: u64,
    pub jemalloc: u64,
    pub unaccounted: u64,
    pub largest_anon: LargestAnon,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LargestAnon {
    pub begin: u64,
    pub end: u64,
    pub size: u64,
}
```

The parser recognizes these region name patterns in smaps mapping lines:
- `[anon:FEXMemJIT]` → `jit_code`
- `[anon:FEXMem_OpDispatcher]` → `op_dispatcher`
- `[anon:FEXMem_Frontend]` → `frontend`
- `[anon:FEXMem_CPUBackend]` → `cpu_backend`
- `[anon:FEXMem_Lookup_L1]` → `lookup_l1`
- `[anon:FEXMem_Lookup]` → `lookup`
- `[anon:FEXMem_ThreadState]` → `thread_states`
- `[anon:FEXMem_BlockLinks]` → `block_links`
- `[anon:FEXMem_Misc]` → `misc`
- `[anon:FEXMem...]` (anything else with FEXMem prefix) → `unaccounted`
- `[anon:JEMalloc]` or `[anon:FEXAllocator]` → `jemalloc`

For each recognized region, accumulate the `Rss:` line value (converted from kB to bytes).

Improvement over C++: keep the fd open and `seek(0)` + re-read each cycle instead of re-opening.
The C++ already does this — preserve that optimization.

### 2.3 — Load accumulator (`sampler/accumulator.rs`)

Computes derived metrics from raw deltas. This is where the C++ `AccumulateJITStats` logic lives,
but decomposed into pure functions.

```rust
pub struct Accumulator {
    cycle_freq: f64,
    hardware_concurrency: usize,
    histogram: VecDeque<HistogramEntry>,  // Fixed-capacity ring buffer
    histogram_capacity: usize,            // Default: 200 (matching C++)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ComputedFrame {
    pub timestamp_ns: u64,            // Nanoseconds since recording start
    pub sample_period_ns: u64,        // Actual elapsed time for this sample
    pub threads_sampled: usize,

    // Aggregated totals (deltas summed across all threads)
    pub total_jit_time: u64,
    pub total_signal_time: u64,
    pub total_sigbus_count: u64,
    pub total_smc_count: u64,
    pub total_float_fallback_count: u64,
    pub total_cache_miss_count: u64,
    pub total_cache_read_lock_time: u64,
    pub total_cache_write_lock_time: u64,
    pub total_jit_count: u64,
    pub total_jit_invocations: u64,   // Absolute (not delta)

    // Derived percentages
    pub fex_load_percent: f64,        // Overall JIT load as percentage

    // Per-thread load (sorted hottest first, capped at hardware_concurrency)
    pub thread_loads: Vec<ThreadLoad>,

    // Memory snapshot (may be stale if smaps hasn't updated yet)
    pub mem: MemSnapshot,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ThreadLoad {
    pub tid: u32,
    pub load_percent: f32,
    pub total_cycles: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HistogramEntry {
    pub load_percent: f32,
    pub high_jit_load: bool,
    pub high_invalidation_or_smc: bool,
    pub high_sigbus: bool,
    pub high_softfloat: bool,
}
```

Methods:
- `compute_frame(&mut self, sample: &SampleResult, mem: &MemSnapshot, prev_time: Instant) -> ComputedFrame`
  - Mirrors the C++ load calculation:
    ```
    MaxCyclesInSamplePeriod = cycle_freq * (sample_period_ns / 1e9)
    MaxCoresThreadsPossible = min(hardware_concurrency, threads_sampled)
    fex_load = (total_jit_time / (MaxCyclesInSamplePeriod * MaxCoresThreadsPossible)) * 100
    ```
  - Per-thread: `load = (thread_jit_time / MaxCyclesInSamplePeriod) * 100`
  - Sorts threads by total time descending, caps at `hardware_concurrency`
  - Pushes `HistogramEntry` with condition flags:
    - `high_jit_load`: total_jit_time >= MaxCyclesInSamplePeriod (more than one core of JIT load)
    - `high_invalidation_or_smc`: SMC count >= 500
    - `high_sigbus`: SIGBUS count >= 5,000
    - `high_softfloat`: float fallback count >= 1,000,000
  - Returns `ComputedFrame` containing everything needed for display or recording

### 2.4 — Unified data source trait (`datasource.rs`)

Abstract over live sampling vs replay so the TUI doesn't need to know.

```rust
pub trait DataSource {
    /// Get the next frame of data. Blocks or returns None if not ready.
    fn next_frame(&mut self) -> Option<ComputedFrame>;

    /// Metadata about the source.
    fn metadata(&self) -> &SessionMetadata;

    /// Is this source live (vs replay)?
    fn is_live(&self) -> bool;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub pid: i32,
    pub fex_version: String,
    pub app_type: AppType,
    pub stats_version: u8,
    pub cycle_counter_frequency: u64,
    pub hardware_concurrency: usize,
    pub recording_start: SystemTime,  // Wall-clock time of first sample
}
```

### 2.5 — Live data source

```rust
pub struct LiveSource {
    shm: ShmReader,
    thread_sampler: ThreadSampler,
    mem_sampler: MemSampler,
    accumulator: Accumulator,
    process_watcher: ProcessWatcher,
    sample_period: Duration,
    last_sample: Instant,
    metadata: SessionMetadata,
    // Memory stats are updated by a background thread and read atomically
    latest_mem: Arc<Mutex<MemSnapshot>>,
    shutdown: Arc<AtomicBool>,
}
```

The live source spawns a background thread for smaps parsing (matching the C++ design where
`ResidentFEXAnonSampling` runs on its own thread). The main sampling loop:
1. Check `process_watcher.has_exited()`
2. `shm.check_resize()`
3. `platform::store_memory_barrier()`
4. `shm.read_thread_stats()` → raw stats
5. `thread_sampler.sample(raw_stats, now)` → deltas
6. Read latest `mem` snapshot from the background thread
7. `accumulator.compute_frame(deltas, mem, prev_time)` → `ComputedFrame`

---

## Phase 3: Recording System

### 3.1 — Recording file format (`recording/format.rs`)

Design a streaming format that supports:
- Append-only writes (no seeking during recording)
- Streaming reads (no need to load entire file into memory)
- Forward compatibility (new fields can be added to frames)

Format structure:
```
[FileHeader]                    — 1 per file, fixed-size + variable
[Frame 0][Frame 1]...[Frame N]  — variable number of frames
[EOF marker]                    — sentinel to distinguish clean close from truncation
```

```rust
pub const MAGIC: [u8; 4] = *b"WTFR";  // WTF Recording
pub const FORMAT_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub format_version: u8,
    pub metadata: SessionMetadata,
}

#[derive(Serialize, Deserialize)]
pub struct Frame {
    pub computed: ComputedFrame,
    // The full per-thread raw deltas, for detailed offline analysis
    pub per_thread_deltas: Vec<ThreadDelta>,
}

pub const EOF_MARKER: [u8; 4] = *b"WEOF";
```

Why postcard + zstd:
- postcard has a stable wire format, supports serde, and is compact for numeric data
- zstd streaming compression wraps the entire postcard byte stream
- At ~800 KB/s uncompressed, zstd level 3 reduces this to ~200-400 KB/s with negligible CPU cost

### 3.2 — Recording writer (`recording/writer.rs`)

```rust
pub struct RecordingWriter {
    encoder: zstd::stream::write::Encoder<'static, BufWriter<File>>,
}

impl RecordingWriter {
    pub fn create(path: &Path, metadata: &SessionMetadata) -> Result<Self>;
    pub fn write_frame(&mut self, frame: &Frame) -> Result<()>;
    pub fn finish(self) -> Result<()>;  // Writes EOF marker, flushes zstd, closes file
}
```

Each frame is serialized with postcard and length-prefixed (4-byte little-endian length prefix
before each serialized frame) to allow streaming deserialization.

### 3.3 — Recording reader (`recording/reader.rs`)

```rust
pub struct RecordingReader {
    decoder: zstd::stream::read::Decoder<'static, BufReader<File>>,
    metadata: SessionMetadata,
    frames: Vec<Frame>,        // All frames loaded into memory (for seeking)
    current_index: usize,
}

impl RecordingReader {
    pub fn open(path: &Path) -> Result<Self>;     // Reads header, loads all frames
    pub fn metadata(&self) -> &SessionMetadata;
    pub fn frame_count(&self) -> usize;
    pub fn frame_at(&self, index: usize) -> Option<&Frame>;
    pub fn duration(&self) -> Duration;           // Last frame timestamp - first
}
```

For a 1-hour recording at 1Hz, this is ~3,600 frames — trivially fits in memory. For higher-rate
recordings (100Hz for 1 hour = 360,000 frames), this is still only ~100-200 MB in memory. If
this becomes a problem in the future, switch to an indexed approach.

### 3.4 — Replay data source

```rust
pub struct ReplaySource {
    reader: RecordingReader,
    current_index: usize,
    playback_speed: f64,        // 1.0 = real-time, 2.0 = 2x, 0.0 = paused
    last_emitted: Instant,
    state: PlaybackState,
}

pub enum PlaybackState {
    Playing,
    Paused,
    Finished,
}

impl DataSource for ReplaySource { ... }
```

The replay source emits frames according to their original timestamp spacing, scaled by
`playback_speed`. When paused, `next_frame()` returns `None`.

---

## Phase 4: TUI Framework

### 4.1 — Application state (`tui/app.rs`)

```rust
pub struct App {
    pub mode: AppMode,
    pub source: Box<dyn DataSource>,
    pub latest_frame: Option<ComputedFrame>,
    pub histogram: VecDeque<HistogramEntry>,
    pub panels: Vec<PanelState>,
    pub selected_panel: usize,
    pub should_quit: bool,
    pub recording_writer: Option<RecordingWriter>,  // Some if --record is active
}

pub enum AppMode {
    Live,
    Replay { controls: ReplayControls },
}

pub struct PanelState {
    pub name: &'static str,
    pub collapsed: bool,
}

pub struct ReplayControls {
    pub speed: f64,           // 0.25, 0.5, 1.0, 2.0, 4.0
    pub paused: bool,
    pub current_frame: usize,
    pub total_frames: usize,
}
```

### 4.2 — Event loop (`tui/app.rs`)

Use a tick-based event loop with crossterm:

```rust
enum AppEvent {
    Key(KeyEvent),
    Tick,
    NewFrame(ComputedFrame),
    ProcessExited,
}
```

Architecture:
- Main thread runs the ratatui render loop at ~30 FPS (33ms tick)
- Data sampling happens on tick (for live mode) or frame emission (for replay)
- Input is polled non-blocking via `crossterm::event::poll`

Loop structure:
```
loop {
    terminal.draw(|frame| ui::render(frame, &app))?;

    if crossterm::event::poll(tick_rate)? {
        handle_input(&mut app, crossterm::event::read()?);
    }

    if let Some(frame) = app.source.next_frame() {
        app.update(frame);
        if let Some(writer) = &mut app.recording_writer {
            writer.write_frame(&frame)?;
        }
    }

    if app.should_quit { break; }
}
```

### 4.3 — Theme and symbols (`tui/theme.rs`)

Centralize all visual constants:

```rust
pub struct Theme {
    pub load_normal: Style,
    pub load_medium: Style,     // Yellow, >= 50%
    pub load_high: Style,       // Red, >= 75%
    pub histo_jit_load: Style,  // Magenta
    pub histo_smc: Style,       // Blue
    pub histo_sigbus: Style,    // Cyan
    pub histo_softfloat: Style, // Green
    pub border: Style,
    pub selected: Style,
    pub title: Style,
}

pub const BLOCK_CHARS: [&str; 9] = [
    " ", "▁", "▁", "▂", "▃", "▄", "▅", "▆", "▇",
];
pub const BLOCK_FULL: &str = "█";

pub const SELECTED_MARKER: [&str; 2] = ["☐", "*"];
pub const COLLAPSED_MARKER: [&str; 2] = ["▼", "►"];
```

Note: the C++ uses the same `▁` for both 10% and 20% levels. Preserve this behavior for parity,
but consider using the full `bar::NINE_LEVELS` from ratatui's Sparkline in the future.

### 4.4 — Layout system (`tui/layout.rs`)

Use ratatui's `Layout` with dynamic constraints:

```rust
fn build_layout(panels: &[PanelState], area: Rect) -> Vec<Rect> {
    let constraints: Vec<Constraint> = panels.iter().map(|p| {
        if p.collapsed {
            Constraint::Length(3)  // Title bar only (border + title + border)
        } else {
            Constraint::Min(p.min_height())
        }
    }).collect();

    Layout::vertical(constraints).split(area).to_vec()
}
```

Improvement over C++: ratatui's constraint solver handles resize automatically. No manual
`wresize`/`mvderwin` needed.

### 4.5 — Input handling (`tui/input.rs`)

```
Key bindings:
  Up/Down    — Navigate between panels
  Right      — Toggle collapse/expand selected panel
  q / Ctrl-C — Quit
  +/-        — Adjust sample period (live mode)

Replay-only key bindings:
  Space      — Pause/resume playback
  Left/Right — Seek backward/forward by 1 frame (when paused)
  [/]        — Decrease/increase playback speed (0.25x, 0.5x, 1x, 2x, 4x)
  Home/End   — Jump to start/end of recording
```

---

## Phase 5: TUI Panels — JIT Stats

### 5.1 — JIT stats panel (`tui/panels/jit_stats.rs`)

This is the largest and most complex panel. It renders:

**Top section: Per-thread load bars**

For each thread in `frame.thread_loads` (sorted hottest-first, capped at `hardware_concurrency`):
```
[████████▃                                        ]: 83.40% (234 ms/S, 1234567 cycles)
[████▅                                            ]: 45.50% (128 ms/S, 876543 cycles)
```

Implementation using ratatui:
- Use a custom widget (or `Paragraph` with styled `Spans`) for each thread bar
- The bar is built character-by-character: full block chars for complete cells, partial block char
  for the fractional cell, space for the remainder
- Color: Red if >= 75%, Yellow if >= 50%, default otherwise

Improvement over C++: display the thread TID next to each bar for identification.

**Bottom section: Aggregate counters**

```
Total (1000 millisecond sample period):
       JIT Time: 0.234567 ms/second (12.34 percent)
    Signal Time: 0.012345 ms/second (0.65 percent)
     SIGBUS Cnt: 150 (150.00 per second)
        SMC Cnt: 42
  Softfloat Cnt: 1,234,567
  CacheMiss Cnt: 89 (89.00 per second) (12,345,678 total JIT invocations)
    $RDLck Time: 0.001234 ms/second (0.06 percent)
    $WRLck Time: 0.000567 ms/second (0.03 percent)
        JIT Cnt: 156 (156.00 per second)
FEX JIT Load:    45.670000 (cycles: 987654321)
```

Conversions:
- Cycles to seconds: `cycles / cycle_counter_frequency`
- Scale factor: 1000.0 (to display as ms/second)
- Per-second rates: `count * (sample_period_ns / 1e9)`

**Title bar:**
```
┌ * ▼ FEX JIT Stats                                              PID: 52954 ┐
```

Panel dynamically adjusts height: `2 (border) + 1 (header) + thread_count + 11 (counter lines)`.

### 5.2 — Number formatting utility

Port the C++ `CustomPrintInteger` to Rust using the `num-format` crate:

```rust
use num_format::{Locale, ToFormattedString};

fn format_integer(n: u64) -> String {
    n.to_formatted_string(&Locale::en)
}
```

### 5.3 — Cycles-to-time conversion utility

```rust
fn cycles_to_milliseconds(cycles: u64, freq: f64) -> u64 {
    let cycles_per_ms = freq / 1000.0;
    (cycles as f64 / cycles_per_ms) as u64
}

fn cycles_to_seconds(cycles: u64, freq: f64) -> f64 {
    cycles as f64 / freq
}
```

---

## Phase 6: TUI Panels — Memory and Histogram

### 6.1 — Memory usage panel (`tui/panels/mem_stats.rs`)

Renders the FEX memory breakdown from `MemSnapshot`:

```
┌ ☐ ▼ FEX Memory Usage ──────────────────────────────────────────────────────┐
│ Total FEX Anon memory resident: 245 MiB                                    │
│     JIT resident:             128 MiB                                      │
│     OpDispatcher resident:      8 MiB                                      │
│     Frontend resident:          4 MiB                                      │
│     CPUBackend resident:       12 MiB                                      │
│     Lookup cache resident:     32 MiB                                      │
│     Lookup L1 cache resident:  16 MiB                                      │
│     ThreadStates resident:      2 MiB                                      │
│     BlockLinks resident:        4 MiB                                      │
│           Misc resident:        1 MiB                                      │
│     JEMalloc resident:         28 MiB                                      │
│     Unaccounted resident:      10 MiB                                      │
│                  Largest:       8 MiB [0x..., 0x...)                        │
└────────────────────────────────────────────────────────────────────────────-┘
```

Implementation: Simple `Paragraph` widget with styled lines. Use right-alignment for the values
to make columns line up.

Improvement over C++: use a proper human-readable size formatter that handles GiB as well (the
C++ version only handles MiB and KiB, and has a bug where sub-1024 byte values would use an
uninitialized `Granule` pointer).

```rust
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GiB", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}
```

When memory data is not yet available (initial state), display "Waiting for data..." instead of
"Couldn't detect" (clearer UX).

### 6.2 — Load histogram panel (`tui/panels/histogram.rs`)

Renders a scrolling bar chart showing FEX JIT load over time, matching the C++ `HandleHistogram`.

```
┌ ☐ ▼ Total JIT usage ──────────────────────────────────────────────────────-┐
│                                                                   ▃        │
│                                                          ▅   █  ▂ █ ▅      │
│                                                    ▂   ▃ █   █  █ █ █  ▃   │
│                                              ▂   ▄ █ ▅ █ █   █  █ █ █  █   │
│                                         ▃  ▅ █ ▅ █ █ █ █ █ ▃ █  █ █ █  █ █ │
│                                    ▂  ▅ █  █ █ █ █ █ █ █ █ █ █ ▃█ █ █  █ █ │
│                               ▃  ▄ █  █ █  █ █ █ █ █ █ █ █ █ █ ██ █ █  █ █ │
│                          ▂  ▆ █  █ █  █ █  █ █ █ █ █ █ █ █ █ █ ██ █ █  █ █ │
│                     ▃  ▇ █  █ █  █ █  █ █  █ █ █ █ █ █ █ █ █ █ ██ █ █ ▆█ █ │
│               ▂  █  █  █ █  █ █  █ █  █ █  █ █ █ █ █ █ █ █ █ █ ██ █ █ ██ █ │
└────────────────────────────────────────────────────────────────────────────-┘
```

Implementation:
- The histogram is a `VecDeque<HistogramEntry>` with fixed capacity (200 entries)
- Each column renders bottom-to-top using Unicode block characters
- Color thresholds: Red >= 75%, Yellow >= 50%
- Color overlays at the base of each column for pathological conditions:
  - Magenta: high JIT load (> 1 core worth)
  - Blue: high invalidation/SMC
  - Cyan: high SIGBUS
  - Green: high softfloat fallback

Implementation with ratatui: use a `Canvas` widget or render cell-by-cell into a `Buffer` via a
custom `Widget` implementation. The custom widget approach gives the most control and matches the
C++ behavior exactly.

```rust
impl Widget for HistogramWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = area.inner(Margin::new(1, 1)); // Inside the border
        for (col_idx, entry) in self.data.iter().rev().enumerate() {
            if col_idx >= inner.width as usize { break; }
            let x = inner.right() - 1 - col_idx as u16;
            render_histogram_column(buf, x, inner.y, inner.height, entry, &self.theme);
        }
    }
}
```

Improvement over C++: add a legend bar at the bottom showing what the colors mean:
```
[■ High JIT] [■ SMC/Invalidation] [■ SIGBUS] [■ Softfloat]
```

### 6.3 — Header/status bar (`tui/panels/header.rs`)

Add a status bar at the very top of the screen (new — the C++ doesn't have this):

```
WTF v0.1.0 | PID: 52954 | FEX: FEX-2501-42-g1234abc | Type: wow64 | Sample: 1000ms | ● REC
```

Shows:
- WTF version
- Target PID
- FEX version string (from shm header)
- App type (Linux32/Linux64/arm64ec/wow64)
- Current sample period
- Recording indicator (red dot when recording)
- In replay mode: `▶ 1.0x | Frame 150/3600 | 00:02:30 / 01:00:00`

---

## Phase 7: Replay Controls

### 7.1 — Replay control bar (`tui/replay_controls.rs`)

When in replay mode, render a control bar at the bottom of the screen:

```
┌ Playback ─────────────────────────────────────────────────────────────────-┐
│ ▶ 1.0x  ▏██████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▕  00:02:30    │
│ [Space] Play/Pause  [←/→] Step  [+/-] Speed  [Home/End] Jump             │
└────────────────────────────────────────────────────────────────────────────┘
```

Components:
- Play/pause indicator with current speed
- Seek bar (ratatui `Gauge` or `LineGauge` widget) showing position within recording
- Current time / total duration
- Key binding hints

### 7.2 — Seek logic

When the user seeks (arrow keys while paused, Home/End):
- Update `current_index` in `ReplaySource`
- Emit the frame at the new index immediately so the TUI updates
- The histogram should rebuild from the frames around the seek position (show the last N frames
  up to and including the current position)

For seeking to work with the histogram, the `ReplaySource` needs to be able to reconstruct the
histogram state at any given frame index. Two approaches:

**Approach A (simple)**: Always replay the histogram from the start up to the current frame.
For a 3,600-frame recording, this means iterating 3,600 entries on each seek — trivially fast.

**Approach B (optimized)**: Store the histogram `VecDeque` state in each frame. Increases recording
size but gives O(1) seek. Only needed if recordings get very large.

Start with Approach A.

### 7.3 — Speed control

Supported speeds: 0.25x, 0.5x, 1x, 2x, 4x, 8x, 16x

When speed changes, recalculate the frame emission interval:
```
emission_interval = original_sample_period / playback_speed
```

At 16x with 1s sample period, frames are emitted every 62.5ms — well within the 30 FPS render
budget.

---

## Phase 8: CLI and Integration

### 8.1 — CLI with clap (`main.rs`)

```rust
#[derive(Parser)]
#[command(name = "wtf", about = "What-the-FEX: FEX-Emu profiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Monitor a running FEX process (default)
    Live {
        /// PID of the FEX process
        pid: i32,
        /// Sample period in milliseconds
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        /// Record session to file
        #[arg(short, long)]
        record: Option<PathBuf>,
    },
    /// Replay a recorded session
    Replay {
        /// Path to recording file
        path: PathBuf,
    },
    /// Record without TUI (headless)
    Record {
        /// PID of the FEX process
        pid: i32,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
        /// Sample period in milliseconds
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        /// Stop after N seconds (0 = until process exits)
        #[arg(long, default_value = "0")]
        duration: u64,
    },
    /// Watch for a FEX process by name and auto-attach
    Watch {
        /// Process name to watch for (searched via /dev/shm/fex-*-stats)
        #[arg(short, long)]
        name: Option<String>,
        /// Same options as `live`
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        #[arg(short, long)]
        record: Option<PathBuf>,
    },
    /// Export a recording to CSV
    Export {
        /// Path to recording file
        input: PathBuf,
        /// Output CSV path
        #[arg(short, long)]
        output: PathBuf,
    },
}
```

### 8.2 — Watch mode

Replaces the broken `watch.sh` script with proper Rust implementation:
1. Scan `/dev/shm/` for `fex-*-stats` entries
2. For each, read the PID and check if the process is alive
3. Optionally filter by process name (read `/proc/<pid>/cmdline`)
4. If multiple matches, present a selection list
5. If no matches, poll every second until one appears
6. Attach and start live monitoring

### 8.3 — Headless recording mode

The `record` subcommand runs without a TUI — just sampling and writing to disk. Useful for
unattended recording or scripted profiling sessions.

Output to stderr: periodic status line showing elapsed time, frames recorded, file size.

### 8.4 — CSV export

The `export` subcommand converts a `.wtfr` recording to CSV for external analysis tools.

CSV columns:
```
timestamp_ms,sample_period_ms,threads_sampled,fex_load_percent,
total_jit_time,total_signal_time,total_sigbus_count,total_smc_count,
total_float_fallback_count,total_cache_miss_count,
total_cache_read_lock_time,total_cache_write_lock_time,
total_jit_count,total_jit_invocations,
mem_total_anon,mem_jit_code,mem_op_dispatcher,mem_frontend,
mem_cpu_backend,mem_lookup,mem_lookup_l1,mem_thread_states,
mem_block_links,mem_misc,mem_jemalloc,mem_unaccounted
```

One row per frame. Per-thread data is flattened: include the top-N thread loads as extra columns
(`thread_0_load,thread_0_cycles,...`).

### 8.5 — Signal handling and graceful shutdown

```rust
let shutdown = Arc::new(AtomicBool::new(false));
signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))?;
signal_hook::flag::register(signal_hook::consts::SIGQUIT, Arc::clone(&shutdown))?;
```

On shutdown:
1. Set `shutdown` flag
2. If recording, call `writer.finish()` to flush and write EOF marker
3. Restore terminal state (`crossterm::terminal::disable_raw_mode`, show cursor)
4. Join background threads (smaps sampler)

---

## Phase 9: Future Extensions (Not in Initial Rewrite)

These are architectural considerations for future work. The design in Phases 1-8 should
accommodate these without major refactoring.

### 9.1 — Extended FEX stats

FEX may expose additional stats in future versions:
- **CPU state counters**: NZCV flag usage, register pressure metrics
- **Emulated instruction counters**: breakdown by x86 instruction category (ALU, FPU, SSE, AVX,
  branch, memory, syscall, etc.)
- **Block statistics**: number of translated blocks, average block size, block invalidation rates

The `ThreadStats` struct in FEX uses `ThreadStatsSize` in the header to support forward
compatibility — WTF reads `min(header.thread_stats_size, sizeof(ThreadStats))`. The Rust
implementation should do the same, allowing it to read older or newer FEX versions gracefully.

When new fields are added to `ThreadStats`:
1. Add the fields to the Rust struct (with defaults for older versions)
2. Add new derived metrics to `ComputedFrame`
3. Add new panel(s) or extend existing panels
4. Bump `FORMAT_VERSION` in the recording format

### 9.2 — Parquet export

For heavy analytical workloads, add a Parquet export option using `arrow-rs` and `parquet-rs`.
This enables SQL queries via DataFusion or direct loading into Python/Polars.

### 9.3 — Multiple process support

Monitor multiple FEX processes simultaneously (e.g., Wine + game). Each process gets its own
data source, and the TUI could show them in tabs or side-by-side.

### 9.4 — Remote monitoring

Accept data over a TCP socket so WTF can run on a different machine than the FEX process. The
sender side would be a small agent that reads shm and streams frames over the network.

### 9.5 — Annotations

Allow users to mark points in time during live monitoring (e.g., press 'M' to add a marker with
a note). Markers are saved in recordings and visible during replay. Useful for correlating
performance events with game actions (loading screen, boss fight, etc.).

---

## Implementation Order Summary

| Phase | What | Depends On | Status |
|---|---|---|---|
| 0 | Project scaffolding | — | DONE |
| 1 | FEX types + shm reader | 0 | DONE |
| 2 | Sampling engine | 1 | DONE |
| 3 | Recording system | 2 | DONE |
| 4 | TUI framework | 2 | DONE |
| 5 | JIT stats panel | 4, 2 | DONE |
| 6 | Memory + histogram panels | 4, 2 | DONE |
| 7 | Replay controls | 4, 3 | DONE |
| 8 | CLI + integration | all | DONE |

All phases completed. 31 tests passing, zero clippy warnings (pedantic mode).
