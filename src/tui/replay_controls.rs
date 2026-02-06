// SPDX-License-Identifier: MIT
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

use super::theme::Theme;

const SPEED_STEPS: [f64; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
const DEFAULT_SPEED_INDEX: usize = 2; // 1.0x
const NANOS_PER_SECOND: u64 = 1_000_000_000;

pub struct ReplayControls {
    pub speed: f64,
    pub paused: bool,
    pub current_frame: usize,
    pub total_frames: usize,
    speed_index: usize,
}

impl ReplayControls {
    #[must_use]
    pub fn new(total_frames: usize) -> Self {
        Self {
            speed: SPEED_STEPS[DEFAULT_SPEED_INDEX],
            paused: false,
            current_frame: 0,
            total_frames,
            speed_index: DEFAULT_SPEED_INDEX,
        }
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn speed_up(&mut self) {
        if self.speed_index + 1 < SPEED_STEPS.len() {
            self.speed_index += 1;
            self.speed = SPEED_STEPS[self.speed_index];
        }
    }

    pub fn speed_down(&mut self) {
        if self.speed_index > 0 {
            self.speed_index -= 1;
            self.speed = SPEED_STEPS[self.speed_index];
        }
    }

    pub fn seek_forward(&mut self) {
        if self.total_frames > 0 {
            self.current_frame = (self.current_frame + 1).min(self.total_frames - 1);
        }
    }

    pub fn seek_backward(&mut self) {
        self.current_frame = self.current_frame.saturating_sub(1);
    }

    pub fn seek_start(&mut self) {
        self.current_frame = 0;
    }

    pub fn seek_end(&mut self) {
        if self.total_frames > 0 {
            self.current_frame = self.total_frames - 1;
        }
    }

    pub fn update_position(&mut self, index: usize) {
        self.current_frame = index;
    }

    #[must_use]
    pub fn progress_fraction(&self) -> f64 {
        if self.total_frames <= 1 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let fraction = self.current_frame as f64 / (self.total_frames - 1) as f64;
        fraction
    }
}

fn format_time(frame_index: usize, sample_period_ns: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let total_ns = frame_index as f64 * sample_period_ns as f64;
    #[allow(clippy::cast_precision_loss)]
    let nanos_per_sec = NANOS_PER_SECOND as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total_seconds = (total_ns / nanos_per_sec) as u64;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub fn render(
    frame: &mut ratatui::Frame,
    area: Rect,
    controls: &ReplayControls,
    sample_period_ns: u64,
    theme: &Theme,
) {
    if area.height < 4 || area.width < 20 {
        return;
    }

    let block = Block::default()
        .title(" Playback ")
        .borders(Borders::ALL)
        .border_style(theme.border_normal)
        .title_style(theme.title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let status_icon = if controls.paused {
        "\u{23F8}"
    } else {
        "\u{25B6}"
    };

    let time_str = format_time(controls.current_frame, sample_period_ns);
    let label = format!(" {status_icon} {:.2}x  ", controls.speed);

    let ratio = controls.progress_fraction().clamp(0.0, 1.0);

    let gauge_label = format!("{label}{time_str}");
    #[allow(clippy::cast_possible_truncation)]
    let gauge = Gauge::default()
        .ratio(ratio)
        .label(gauge_label)
        .gauge_style(theme.border_selected);

    frame.render_widget(gauge, rows[0]);

    let help = Line::from(vec![
        Span::styled("[Space]", theme.title),
        Span::raw(" Pause  "),
        Span::styled("[\u{2190}/\u{2192}]", theme.title),
        Span::raw(" Step  "),
        Span::styled("[+/-]", theme.title),
        Span::raw(" Speed  "),
        Span::styled("[Home/End]", theme.title),
        Span::raw(" Jump"),
    ]);
    frame.render_widget(Paragraph::new(help), rows[1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let rc = ReplayControls::new(100);
        assert!((rc.speed - 1.0).abs() < f64::EPSILON);
        assert!(!rc.paused);
        assert_eq!(rc.current_frame, 0);
        assert_eq!(rc.total_frames, 100);
    }

    #[test]
    fn toggle_pause() {
        let mut rc = ReplayControls::new(10);
        assert!(!rc.paused);
        rc.toggle_pause();
        assert!(rc.paused);
        rc.toggle_pause();
        assert!(!rc.paused);
    }

    #[test]
    fn speed_up_cycles() {
        let mut rc = ReplayControls::new(10);
        assert!((rc.speed - 1.0).abs() < f64::EPSILON);
        rc.speed_up();
        assert!((rc.speed - 2.0).abs() < f64::EPSILON);
        rc.speed_up();
        assert!((rc.speed - 4.0).abs() < f64::EPSILON);
        rc.speed_up();
        assert!((rc.speed - 8.0).abs() < f64::EPSILON);
        rc.speed_up();
        assert!((rc.speed - 16.0).abs() < f64::EPSILON);
        rc.speed_up();
        assert!((rc.speed - 16.0).abs() < f64::EPSILON);
    }

    #[test]
    fn speed_down_cycles() {
        let mut rc = ReplayControls::new(10);
        rc.speed_down();
        assert!((rc.speed - 0.5).abs() < f64::EPSILON);
        rc.speed_down();
        assert!((rc.speed - 0.25).abs() < f64::EPSILON);
        rc.speed_down();
        assert!((rc.speed - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn seek_forward_clamps() {
        let mut rc = ReplayControls::new(5);
        rc.current_frame = 3;
        rc.seek_forward();
        assert_eq!(rc.current_frame, 4);
        rc.seek_forward();
        assert_eq!(rc.current_frame, 4);
    }

    #[test]
    fn seek_backward_clamps() {
        let mut rc = ReplayControls::new(5);
        rc.current_frame = 1;
        rc.seek_backward();
        assert_eq!(rc.current_frame, 0);
        rc.seek_backward();
        assert_eq!(rc.current_frame, 0);
    }

    #[test]
    fn seek_start_and_end() {
        let mut rc = ReplayControls::new(10);
        rc.current_frame = 5;
        rc.seek_end();
        assert_eq!(rc.current_frame, 9);
        rc.seek_start();
        assert_eq!(rc.current_frame, 0);
    }

    #[test]
    fn progress_fraction_boundaries() {
        let mut rc = ReplayControls::new(100);
        assert!(rc.progress_fraction().abs() < f64::EPSILON);
        rc.current_frame = 99;
        assert!((rc.progress_fraction() - 1.0).abs() < f64::EPSILON);
        rc.current_frame = 49;
        let expected = 49.0 / 99.0;
        assert!((rc.progress_fraction() - expected).abs() < 0.001);
    }

    #[test]
    fn progress_fraction_single_frame() {
        let rc = ReplayControls::new(1);
        assert!(rc.progress_fraction().abs() < f64::EPSILON);
    }

    #[test]
    fn progress_fraction_zero_frames() {
        let rc = ReplayControls::new(0);
        assert!(rc.progress_fraction().abs() < f64::EPSILON);
    }

    #[test]
    fn update_position() {
        let mut rc = ReplayControls::new(100);
        rc.update_position(42);
        assert_eq!(rc.current_frame, 42);
    }

    #[test]
    fn format_time_basic() {
        assert_eq!(format_time(0, 1_000_000_000), "00:00");
        assert_eq!(format_time(60, 1_000_000_000), "01:00");
        assert_eq!(format_time(150, 1_000_000_000), "02:30");
    }

    #[test]
    fn seek_forward_zero_frames() {
        let mut rc = ReplayControls::new(0);
        rc.seek_forward();
        assert_eq!(rc.current_frame, 0);
    }

    #[test]
    fn seek_end_zero_frames() {
        let mut rc = ReplayControls::new(0);
        rc.seek_end();
        assert_eq!(rc.current_frame, 0);
    }
}
