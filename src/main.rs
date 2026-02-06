// SPDX-License-Identifier: MIT
#![deny(warnings)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

mod datasource;
mod fex;
mod recording;
mod sampler;
mod tui;

use std::io::{self, BufRead, IsTerminal, Stdout, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::datasource::{DataSource, SessionMetadata};
use crate::fex::platform::{cycle_counter_frequency, store_memory_barrier};
use crate::fex::shm::ShmReader;
use crate::fex::types::STATS_VERSION;
use crate::recording::format::Frame;
use crate::recording::reader::{RecordingReader, ReplaySource};
use crate::recording::writer::RecordingWriter;
use crate::sampler::accumulator::Accumulator;
use crate::sampler::mem_stats::MemStatsWorker;
use crate::sampler::thread_stats::ThreadSampler;
use crate::tui::app::App;
use crate::tui::input::{Action, handle_key};

const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(10);
const WATCH_POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEADLESS_STATUS_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "felix", about = "felix: FEX-Emu profiler and recorder")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Monitor a running FEX process
    Live {
        pid: i32,
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        #[arg(short, long)]
        record: Option<PathBuf>,
    },
    /// Replay a recorded session
    Replay { path: PathBuf },
    /// Record without TUI (headless)
    Record {
        pid: i32,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        #[arg(long, default_value = "0")]
        duration: u64,
    },
    /// Watch for FEX processes and auto-attach
    Watch {
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        #[arg(short, long)]
        record: Option<PathBuf>,
    },
    /// Export a recording to CSV
    Export {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Pick a running FEX process interactively
    Pick {
        #[arg(short, long, default_value = "1000")]
        sample_period: u64,
        #[arg(short, long)]
        record: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Live {
            pid,
            sample_period,
            record,
        } => cmd_live(pid, sample_period, record.as_deref()),
        Commands::Replay { path } => cmd_replay(&path),
        Commands::Record {
            pid,
            output,
            sample_period,
            duration,
        } => cmd_record(pid, &output, sample_period, duration),
        Commands::Watch {
            sample_period,
            record,
        } => cmd_watch(sample_period, record.as_deref()),
        Commands::Export { input, output } => cmd_export(&input, &output),
        Commands::Pick {
            sample_period,
            record,
        } => cmd_pick(sample_period, record.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

fn install_signal_handler() -> Result<Arc<AtomicBool>> {
    let shutdown = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))
        .context("failed to register SIGINT handler")?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown))
        .context("failed to register SIGTERM handler")?;
    Ok(shutdown)
}

// ---------------------------------------------------------------------------
// Process liveness check
// ---------------------------------------------------------------------------

fn process_alive(pid: i32) -> bool {
    // kill(pid, 0) checks if the process exists without sending a signal
    unsafe { libc::kill(pid, 0) == 0 }
}

// ---------------------------------------------------------------------------
// Terminal setup / teardown
// ---------------------------------------------------------------------------

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)
        .context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared: build metadata from SHM header
// ---------------------------------------------------------------------------

fn build_metadata(shm: &ShmReader, pid: i32) -> Result<SessionMetadata> {
    let header = shm.read_header();

    if header.version != STATS_VERSION {
        bail!(
            "unsupported stats version {} (expected {STATS_VERSION})",
            header.version
        );
    }

    Ok(SessionMetadata {
        pid,
        fex_version: header.fex_version,
        app_type: header.app_type,
        stats_version: header.version,
        cycle_counter_frequency: cycle_counter_frequency(),
        hardware_concurrency: hardware_concurrency(),
        recording_start: SystemTime::now(),
        head: header.head,
        size: header.size,
    })
}

fn hardware_concurrency() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

// ---------------------------------------------------------------------------
// Live subcommand
// ---------------------------------------------------------------------------

fn cmd_live(pid: i32, sample_period_ms: u64, record_path: Option<&Path>) -> Result<()> {
    let shutdown = install_signal_handler()?;
    let mut shm = ShmReader::open(pid)?;
    let metadata = build_metadata(&shm, pid)?;
    let sample_period = Duration::from_millis(sample_period_ms);
    #[allow(clippy::cast_possible_truncation)]
    let period_nanos = sample_period.as_nanos() as u64;

    let mut mem_worker = MemStatsWorker::spawn(pid, sample_period)?;
    let mut thread_sampler = ThreadSampler::new();
    let accumulator = Accumulator::new(
        #[allow(clippy::cast_precision_loss)]
        {
            metadata.cycle_counter_frequency as f64
        },
        metadata.hardware_concurrency,
    );

    let mut writer = match record_path {
        Some(p) => Some(RecordingWriter::create(p, &metadata)?),
        None => None,
    };

    let mut terminal = setup_terminal()?;
    let mut app = App::new(metadata, false);
    let mut total_jit_invocations: u64 = 0;
    let mut last_sample = Instant::now();

    let result = run_live_loop(
        &shutdown,
        pid,
        &mut shm,
        &mut thread_sampler,
        &accumulator,
        &mut mem_worker,
        &mut app,
        &mut writer,
        &mut terminal,
        &mut total_jit_invocations,
        &mut last_sample,
        sample_period,
        period_nanos,
    );

    mem_worker.shutdown();
    if let Some(w) = writer {
        let _ = w.finish();
    }
    restore_terminal(&mut terminal)?;

    result
}

#[allow(clippy::too_many_arguments)]
fn run_live_loop(
    shutdown: &Arc<AtomicBool>,
    pid: i32,
    shm: &mut ShmReader,
    thread_sampler: &mut ThreadSampler,
    accumulator: &Accumulator,
    mem_worker: &mut MemStatsWorker,
    app: &mut App,
    writer: &mut Option<RecordingWriter>,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    total_jit_invocations: &mut u64,
    last_sample: &mut Instant,
    interval: Duration,
    period_nanos: u64,
) -> Result<()> {
    loop {
        if shutdown.load(Ordering::Relaxed) || app.should_quit {
            break;
        }

        if !process_alive(pid) {
            break;
        }

        let elapsed = last_sample.elapsed();
        let poll_timeout = if elapsed >= interval {
            Duration::ZERO
        } else {
            EVENT_POLL_TIMEOUT.min(interval.checked_sub(elapsed).unwrap())
        };

        if event::poll(poll_timeout).context("failed to poll events")?
            && let Event::Key(key) = event::read().context("failed to read event")?
            && key.kind == KeyEventKind::Press
        {
            let action = handle_key(key.code, false);
            handle_sample_period_action(&action, app);
            app.handle_action(&action);
        }

        if last_sample.elapsed() >= interval {
            take_live_sample(
                shm,
                thread_sampler,
                accumulator,
                mem_worker,
                app,
                writer,
                total_jit_invocations,
                period_nanos,
            )?;
            *last_sample = Instant::now();
        }

        terminal
            .draw(|f| app.render(f))
            .context("failed to draw frame")?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn take_live_sample(
    shm: &mut ShmReader,
    thread_sampler: &mut ThreadSampler,
    accumulator: &Accumulator,
    mem_worker: &mut MemStatsWorker,
    app: &mut App,
    writer: &mut Option<RecordingWriter>,
    total_jit_invocations: &mut u64,
    period_nanos: u64,
) -> Result<()> {
    store_memory_barrier();
    shm.check_resize()?;

    let raw_stats = shm.read_thread_stats();
    let now = Instant::now();
    let sample = thread_sampler.sample(&raw_stats, now);
    let mem = mem_worker.latest();

    *total_jit_invocations = total_jit_invocations
        .wrapping_add(sample.per_thread.iter().map(|d| d.jit_count).sum::<u64>());

    let frame = accumulator.compute_frame(&sample, &mem, period_nanos, *total_jit_invocations);

    if let Some(ref mut w) = *writer {
        let rec_frame = Frame {
            computed: frame.clone(),
            per_thread_deltas: sample.per_thread,
        };
        w.write_frame(&rec_frame)?;
    }

    app.update_frame(frame);
    Ok(())
}

fn handle_sample_period_action(_action: &Action, _app: &mut App) {
    // Sample period adjustment is a no-op for now; the C++ version
    // supported +/- keys to change sample period at runtime, but the
    // current architecture uses a fixed period per session.
}

// ---------------------------------------------------------------------------
// Replay subcommand
// ---------------------------------------------------------------------------

fn cmd_replay(path: &Path) -> Result<()> {
    let shutdown = install_signal_handler()?;
    let reader = RecordingReader::open(path)?;
    let total = reader.frame_count();
    let metadata = reader.metadata().clone();

    let mut app = App::new(metadata, true);
    app.set_replay_total_frames(total);

    let mut source = ReplaySource::new(reader);
    let mut terminal = setup_terminal()?;

    let result = run_replay_loop(&shutdown, &mut app, &mut source, &mut terminal);

    restore_terminal(&mut terminal)?;
    result
}

fn run_replay_loop(
    shutdown: &Arc<AtomicBool>,
    app: &mut App,
    source: &mut ReplaySource,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    loop {
        if shutdown.load(Ordering::Relaxed) || app.should_quit {
            break;
        }

        if event::poll(EVENT_POLL_TIMEOUT).context("failed to poll events")?
            && let Event::Key(key) = event::read().context("failed to read event")?
            && key.kind == KeyEventKind::Press
        {
            let action = handle_key(key.code, true);
            app.handle_action(&action);
        }

        sync_replay_state(app, source);

        if let Some(frame) = source.next_frame() {
            app.update_frame(frame);
            if let Some(controls) = app.replay_controls_mut() {
                controls.update_position(source.current_index());
            }
        }

        terminal
            .draw(|f| app.render(f))
            .context("failed to draw frame")?;
    }
    Ok(())
}

fn sync_replay_state(app: &App, source: &mut ReplaySource) {
    if let Some(controls) = app.replay_controls() {
        source.set_speed(controls.speed);
        if controls.paused != source.is_paused() {
            source.toggle_pause();
        }
        if controls.current_frame != source.current_index() {
            source.seek_to(controls.current_frame);
        }
    }
}

// ---------------------------------------------------------------------------
// Record (headless) subcommand
// ---------------------------------------------------------------------------

fn cmd_record(pid: i32, output: &Path, sample_period_ms: u64, duration_secs: u64) -> Result<()> {
    let shutdown = install_signal_handler()?;
    let mut shm = ShmReader::open(pid)?;
    let metadata = build_metadata(&shm, pid)?;
    let sample_period = Duration::from_millis(sample_period_ms);
    #[allow(clippy::cast_possible_truncation)]
    let period_nanos = sample_period.as_nanos() as u64;

    let mut mem_worker = MemStatsWorker::spawn(pid, sample_period)?;
    let mut thread_sampler = ThreadSampler::new();
    let accumulator = Accumulator::new(
        #[allow(clippy::cast_precision_loss)]
        {
            metadata.cycle_counter_frequency as f64
        },
        metadata.hardware_concurrency,
    );

    let mut writer = RecordingWriter::create(output, &metadata)?;
    let mut total_jit_invocations: u64 = 0;

    let max_duration = if duration_secs > 0 {
        Some(Duration::from_secs(duration_secs))
    } else {
        None
    };

    let start = Instant::now();
    let mut last_status = Instant::now();
    let mut frames_recorded: u64 = 0;

    eprintln!("Recording PID {pid} to {} ...", output.display());

    loop {
        if shutdown.load(Ordering::Relaxed) {
            eprintln!("\nInterrupted.");
            break;
        }
        if !process_alive(pid) {
            eprintln!("\nProcess {pid} exited.");
            break;
        }
        if let Some(max) = max_duration
            && start.elapsed() >= max
        {
            eprintln!("\nDuration limit reached.");
            break;
        }

        std::thread::sleep(sample_period);

        store_memory_barrier();
        shm.check_resize()?;

        let raw_stats = shm.read_thread_stats();
        let now = Instant::now();
        let sample = thread_sampler.sample(&raw_stats, now);
        let mem = mem_worker.latest();

        total_jit_invocations = total_jit_invocations
            .wrapping_add(sample.per_thread.iter().map(|d| d.jit_count).sum::<u64>());

        let frame = accumulator.compute_frame(&sample, &mem, period_nanos, total_jit_invocations);

        let rec_frame = Frame {
            computed: frame,
            per_thread_deltas: sample.per_thread,
        };
        writer.write_frame(&rec_frame)?;
        frames_recorded += 1;

        if last_status.elapsed() >= HEADLESS_STATUS_INTERVAL {
            print_recording_status(start.elapsed(), frames_recorded, output);
            last_status = Instant::now();
        }
    }

    mem_worker.shutdown();
    writer.finish()?;

    eprintln!(
        "Finished: {frames_recorded} frames written to {}",
        output.display()
    );
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn print_recording_status(elapsed: Duration, frames: u64, path: &Path) {
    let secs = elapsed.as_secs();
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    eprintln!(
        "  [{secs}s] {frames} frames, {:.1} KB",
        size as f64 / 1024.0
    );
}

// ---------------------------------------------------------------------------
// Watch subcommand
// ---------------------------------------------------------------------------

fn cmd_watch(sample_period_ms: u64, record_path: Option<&Path>) -> Result<()> {
    let shutdown = install_signal_handler()?;

    eprintln!("Watching for FEX processes...");

    loop {
        if shutdown.load(Ordering::Relaxed) {
            bail!("interrupted while watching for FEX processes");
        }

        if let Some(pid) = find_fex_process() {
            eprintln!("Found FEX process with PID {pid}");
            return cmd_live(pid, sample_period_ms, record_path);
        }

        std::thread::sleep(WATCH_POLL_INTERVAL);
    }
}

fn find_all_fex_processes() -> Vec<i32> {
    let Some(read_dir) = std::fs::read_dir("/dev/shm").ok() else {
        return Vec::new();
    };
    let mut candidates: Vec<i32> = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(rest) = name_str.strip_prefix("fex-")
            && let Some(pid_str) = rest.strip_suffix("-stats")
            && let Ok(pid) = pid_str.parse::<i32>()
            && process_alive(pid)
        {
            candidates.push(pid);
        }
    }

    candidates.sort_unstable();
    candidates
}

fn find_fex_process() -> Option<i32> {
    find_all_fex_processes().last().copied()
}

fn read_process_cmdline(pid: i32) -> String {
    let path = format!("/proc/{pid}/cmdline");
    std::fs::read(&path).map_or_else(
        |_| String::new(),
        |bytes| {
            bytes
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        },
    )
}

fn read_process_ppid(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Format: pid (comm) state ppid ... — comm can contain ')' so find the last one
    let after_comm = &stat[stat.rfind(')')? + 2..];
    // Fields after comm: state ppid ...
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

// ---------------------------------------------------------------------------
// Pick subcommand
// ---------------------------------------------------------------------------

fn cmd_pick(sample_period_ms: u64, record_path: Option<&Path>) -> Result<()> {
    let pids = find_all_fex_processes();

    if pids.is_empty() {
        bail!("no running FEX processes found");
    }

    let color = io::stderr().is_terminal();

    let pid = if pids.len() == 1 {
        let pid = pids[0];
        let cmdline = read_process_cmdline(pid);
        if color {
            eprintln!(
                "Only one FEX process found: \x1b[1;32mPID {pid}\x1b[0m  {cmdline}"
            );
        } else {
            eprintln!("Only one FEX process found: PID {pid}  {cmdline}");
        }
        pid
    } else {
        let ordered = print_process_tree(&pids, color);
        prompt_selection(&ordered)?
    };

    cmd_live(pid, sample_period_ms, record_path)
}

fn print_process_tree(pids: &[i32], color: bool) -> Vec<i32> {
    let pid_set: std::collections::HashSet<i32> = pids.iter().copied().collect();
    let mut children_map: std::collections::HashMap<i32, Vec<i32>> =
        std::collections::HashMap::new();
    let mut roots = Vec::new();

    for &pid in pids {
        if let Some(ppid) = read_process_ppid(pid)
            && pid_set.contains(&ppid)
        {
            children_map.entry(ppid).or_default().push(pid);
        } else {
            roots.push(pid);
        }
    }

    for v in children_map.values_mut() {
        v.sort_unstable();
    }
    roots.sort_unstable();

    eprintln!("Running FEX processes:");
    let mut ordered = Vec::new();
    let mut counter = 0;

    for &root in &roots {
        print_tree_node(
            root,
            &children_map,
            &mut ordered,
            &mut counter,
            "",
            true,
            false,
            color,
        );
    }

    ordered
}

#[allow(clippy::too_many_arguments)]
fn print_tree_node(
    pid: i32,
    children_map: &std::collections::HashMap<i32, Vec<i32>>,
    ordered: &mut Vec<i32>,
    counter: &mut usize,
    prefix: &str,
    is_root: bool,
    is_last: bool,
    color: bool,
) {
    let idx = *counter;
    *counter += 1;
    ordered.push(pid);
    let cmdline = read_process_cmdline(pid);

    if is_root {
        if color {
            eprintln!(
                "  \x1b[33m[{idx}]\x1b[0m \x1b[1;32mPID {pid}\x1b[0m  {cmdline}"
            );
        } else {
            eprintln!("  [{idx}] PID {pid}  {cmdline}");
        }
    } else {
        let connector = if is_last { "└── " } else { "├── " };
        if color {
            eprintln!(
                "  {prefix}\x1b[2m{connector}\x1b[0m\x1b[33m[{idx}]\x1b[0m \x1b[36mPID {pid}\x1b[0m  {cmdline}"
            );
        } else {
            eprintln!("  {prefix}{connector}[{idx}] PID {pid}  {cmdline}");
        }
    }

    if let Some(kids) = children_map.get(&pid) {
        let child_prefix = if !is_root && !is_last {
            format!("{prefix}│   ")
        } else {
            format!("{prefix}    ")
        };
        for (i, &child) in kids.iter().enumerate() {
            print_tree_node(
                child,
                children_map,
                ordered,
                counter,
                &child_prefix,
                false,
                i == kids.len() - 1,
                color,
            );
        }
    }
}

fn prompt_selection(pids: &[i32]) -> Result<i32> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        eprint!("Select process [0-{}] (q to quit): ", pids.len() - 1);
        io::stderr().flush()?;

        let Some(line) = lines.next() else {
            bail!("unexpected end of input");
        };
        let line = line.context("failed to read from stdin")?;
        let input = line.trim();

        if input.eq_ignore_ascii_case("q") {
            bail!("selection cancelled");
        }

        if let Ok(idx) = input.parse::<usize>()
            && idx < pids.len()
        {
            return Ok(pids[idx]);
        }

        eprintln!("Invalid selection: {input}");
    }
}

// ---------------------------------------------------------------------------
// Export subcommand
// ---------------------------------------------------------------------------

fn cmd_export(input: &Path, output: &Path) -> Result<()> {
    let reader = RecordingReader::open(input)?;
    let total = reader.frame_count();

    let mut out = std::fs::File::create(output)
        .with_context(|| format!("failed to create {}", output.display()))?;

    write_csv_header(&mut out)?;

    for i in 0..total {
        if let Some(frame) = reader.frame_at(i) {
            write_csv_row(&mut out, i, &frame.computed)?;
        }
    }

    eprintln!(
        "Exported {total} frames from {} to {}",
        input.display(),
        output.display()
    );
    Ok(())
}

fn write_csv_header(out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "frame,timestamp_ns,sample_period_ns,threads_sampled,\
         total_jit_time,total_signal_time,total_sigbus_count,\
         total_smc_count,total_float_fallback_count,\
         total_cache_miss_count,total_cache_read_lock_time,\
         total_cache_write_lock_time,total_jit_count,\
         total_jit_invocations,fex_load_percent,\
         mem_total_anon,mem_jit_code,mem_op_dispatcher,\
         mem_frontend,mem_cpu_backend,mem_lookup,mem_lookup_l1,\
         mem_thread_states,mem_block_links,mem_misc,\
         mem_jemalloc,mem_unaccounted"
    )
    .context("failed to write CSV header")
}

fn write_csv_row(
    out: &mut impl Write,
    index: usize,
    f: &sampler::accumulator::ComputedFrame,
) -> Result<()> {
    writeln!(
        out,
        "{index},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{}",
        f.timestamp_ns,
        f.sample_period_ns,
        f.threads_sampled,
        f.total_jit_time,
        f.total_signal_time,
        f.total_sigbus_count,
        f.total_smc_count,
        f.total_float_fallback_count,
        f.total_cache_miss_count,
        f.total_cache_read_lock_time,
        f.total_cache_write_lock_time,
        f.total_jit_count,
        f.total_jit_invocations,
        f.fex_load_percent,
        f.mem.total_anon,
        f.mem.jit_code,
        f.mem.op_dispatcher,
        f.mem.frontend,
        f.mem.cpu_backend,
        f.mem.lookup,
        f.mem.lookup_l1,
        f.mem.thread_states,
        f.mem.block_links,
        f.mem.misc,
        f.mem.jemalloc,
        f.mem.unaccounted,
    )
    .context("failed to write CSV row")
}
