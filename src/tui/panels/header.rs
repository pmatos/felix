// SPDX-License-Identifier: MIT
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::datasource::SessionMetadata;
use crate::tui::theme::Theme;

pub fn render(
    frame: &mut ratatui::Frame,
    area: Rect,
    metadata: &SessionMetadata,
    is_replay: bool,
    sample_period_ns: Option<u64>,
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let version = env!("CARGO_PKG_VERSION");

    let text = if is_replay {
        format!(
            "WTF v{version} | REPLAY | FEX: {} | Type: {}",
            metadata.fex_version, metadata.app_type,
        )
    } else {
        let sample_part = sample_period_ns
            .map_or_else(String::new, |ns| format!(" | Sample: {}ms", ns / 1_000_000));
        format!(
            "WTF v{version} | PID: {} | FEX: {} | Type: {}{sample_part}",
            metadata.pid, metadata.fex_version, metadata.app_type,
        )
    };

    let line = Line::from(vec![Span::styled(
        format!("{text:<width$}", width = area.width as usize),
        theme.status_bar,
    )]);

    frame.render_widget(Paragraph::new(line), area);
}
