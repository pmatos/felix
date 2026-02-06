// SPDX-License-Identifier: MIT
use num_format::{Locale, ToFormattedString};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::datasource::SessionMetadata;
use crate::sampler::accumulator::ComputedFrame;
use crate::tui::theme::{BLOCK_CHARS, BLOCK_FULL, Theme};

const NANOSECONDS_IN_SECOND: f64 = 1_000_000_000.0;
const SCALE: f64 = 1000.0;
const SCALE_STR: &str = "ms/second";

fn load_style(load: f32, theme: &Theme) -> ratatui::style::Style {
    if load >= 75.0 {
        theme.load_high
    } else if load >= 50.0 {
        theme.load_medium
    } else {
        theme.load_normal
    }
}

fn cycles_to_ms(cycles: u64, freq: f64) -> u64 {
    if freq <= 0.0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss)]
    let cycles_f = cycles as f64;
    let cycles_per_ms = freq / 1000.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let result = (cycles_f / cycles_per_ms) as u64;
    result
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn build_bar(load: f32, bar_width: usize) -> String {
    let clamped = load.clamp(0.0, 100.0);
    let percentage_per_pip = 100.0 / bar_width as f32;
    let rounded_down = (clamped / 10.0).floor() * 10.0;
    let full_pips = (rounded_down / percentage_per_pip) as usize;
    let digit_percent = (clamped - rounded_down) as usize;

    let mut bar = String::with_capacity(bar_width * 4);
    for i in 0..bar_width {
        if i < full_pips {
            bar.push(BLOCK_FULL);
        } else if i == full_pips && digit_percent < BLOCK_CHARS.len() {
            bar.push(BLOCK_CHARS[digit_percent]);
        } else {
            bar.push(' ');
        }
    }
    bar
}

fn render_thread_loads<'a>(
    data: &ComputedFrame,
    metadata: &SessionMetadata,
    theme: &Theme,
    bar_width: usize,
) -> Vec<Line<'a>> {
    #[allow(clippy::cast_precision_loss)]
    let freq = metadata.cycle_counter_frequency as f64;
    let mut lines: Vec<Line<'a>> = Vec::new();

    lines.push(Line::from(format!(
        "Top {} threads executing ({} total)",
        data.thread_loads.len(),
        data.threads_sampled,
    )));

    for tl in &data.thread_loads {
        let load = tl.load_percent.min(100.0);
        let bar = build_bar(load, bar_width);
        let ms = cycles_to_ms(tl.total_cycles, freq);

        let style = load_style(tl.load_percent, theme);
        let bar_span = Span::styled(format!("[{bar}]"), style);
        let info_span = Span::raw(format!(
            ": {load:.2}% ({ms} ms/S, {} cycles)",
            tl.total_cycles
        ));
        lines.push(Line::from(vec![bar_span, info_span]));
    }

    lines
}

#[allow(clippy::cast_precision_loss)]
fn render_aggregate_stats<'a>(data: &ComputedFrame, metadata: &SessionMetadata) -> Vec<Line<'a>> {
    let freq = metadata.cycle_counter_frequency as f64;
    let max_active = if data.threads_sampled == 0 {
        1.0
    } else {
        let ts = data.threads_sampled as f64;
        let hc = metadata.hardware_concurrency as f64;
        ts.min(hc)
    };

    let jit_seconds = data.total_jit_time as f64 / freq;
    let signal_seconds = data.total_signal_time as f64 / freq;
    let cache_read_lock_seconds = data.total_cache_read_lock_time as f64 / freq;
    let cache_write_lock_seconds = data.total_cache_write_lock_time as f64 / freq;

    let sample_period_ns_f64 = data.sample_period_ns as f64;
    let sigbus_per_second =
        data.total_sigbus_count as f64 * (sample_period_ns_f64 / NANOSECONDS_IN_SECOND);
    let cache_miss_per_second =
        data.total_cache_miss_count as f64 * (sample_period_ns_f64 / NANOSECONDS_IN_SECOND);
    let jit_cnt_per_second =
        data.total_jit_count as f64 * (sample_period_ns_f64 / NANOSECONDS_IN_SECOND);

    let sample_period_ms = data.sample_period_ns / 1_000_000;
    let jit_pct = jit_seconds / max_active * 100.0;
    let signal_pct = signal_seconds / max_active * 100.0;
    let rd_pct = cache_read_lock_seconds / max_active * 100.0;
    let wr_pct = cache_write_lock_seconds / max_active * 100.0;

    let softfloat_fmt = data
        .total_float_fallback_count
        .to_formatted_string(&Locale::en);
    let total_invocations_fmt = data.total_jit_invocations.to_formatted_string(&Locale::en);
    let total_jit_time_all = data.total_jit_time + data.total_signal_time;

    vec![
        Line::from(format!(
            "Total ({sample_period_ms} millisecond sample period):"
        )),
        Line::from(format!(
            "       JIT Time: {:.6} {SCALE_STR} ({jit_pct:.2} percent)",
            jit_seconds * SCALE,
        )),
        Line::from(format!(
            "    Signal Time: {:.6} {SCALE_STR} ({signal_pct:.2} percent)",
            signal_seconds * SCALE,
        )),
        Line::from(format!(
            "     SIGBUS Cnt: {} ({sigbus_per_second:.2} per second)",
            data.total_sigbus_count,
        )),
        Line::from(format!("        SMC Cnt: {}", data.total_smc_count)),
        Line::from(format!("  Softfloat Cnt: {softfloat_fmt}")),
        Line::from(format!(
            "  CacheMiss Cnt: {} ({cache_miss_per_second:.2} per second) ({total_invocations_fmt} total JIT invocations)",
            data.total_cache_miss_count,
        )),
        Line::from(format!(
            "    $RDLck Time: {:.6} {SCALE_STR} ({rd_pct:.2} percent)",
            cache_read_lock_seconds * SCALE,
        )),
        Line::from(format!(
            "    $WRLck Time: {:.6} {SCALE_STR} ({wr_pct:.2} percent)",
            cache_write_lock_seconds * SCALE,
        )),
        Line::from(format!(
            "        JIT Cnt: {} ({jit_cnt_per_second:.2} per second)",
            data.total_jit_count,
        )),
        Line::from(format!(
            "FEX JIT Load:    {:.6} (cycles: {total_jit_time_all})",
            data.fex_load_percent,
        )),
    ]
}

pub fn render(
    frame: &mut ratatui::Frame,
    area: Rect,
    data: &ComputedFrame,
    metadata: &SessionMetadata,
    theme: &Theme,
) {
    if area.height < 2 || area.width < 10 {
        return;
    }

    let bar_width = (area.width.saturating_sub(20) as usize).clamp(4, 48);

    let mut lines = render_thread_loads(data, metadata, theme, bar_width);
    lines.push(Line::from(""));
    lines.extend(render_aggregate_stats(data, metadata));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}
