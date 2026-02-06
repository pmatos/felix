// SPDX-License-Identifier: MIT
use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub load_normal: Style,
    pub load_medium: Style,
    pub load_high: Style,
    pub histo_jit_load: Style,
    pub histo_smc: Style,
    pub histo_sigbus: Style,
    pub histo_softfloat: Style,
    pub border_normal: Style,
    pub border_selected: Style,
    pub title: Style,
    pub status_bar: Style,
    #[allow(dead_code)]
    pub recording_indicator: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            load_normal: Style::default().fg(Color::Green),
            load_medium: Style::default().fg(Color::Yellow),
            load_high: Style::default().fg(Color::Red),
            histo_jit_load: Style::default().fg(Color::Magenta),
            histo_smc: Style::default().fg(Color::Blue),
            histo_sigbus: Style::default().fg(Color::Cyan),
            histo_softfloat: Style::default().fg(Color::Green),
            border_normal: Style::default().fg(Color::White),
            border_selected: Style::default().fg(Color::Cyan),
            title: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            status_bar: Style::default().fg(Color::Black).bg(Color::White),
            recording_indicator: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        }
    }
}

pub const BLOCK_CHARS: [char; 10] = [
    ' ', '\u{2581}', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}',
    '\u{2587}', '\u{2588}',
];
pub const BLOCK_FULL: char = '\u{2588}';
pub const SELECTED_MARKER: [char; 2] = ['\u{2610}', '\u{2611}'];
pub const COLLAPSED_MARKER: [char; 2] = ['\u{25BC}', '\u{25BA}'];
