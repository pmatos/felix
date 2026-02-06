// SPDX-License-Identifier: MIT
use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::sampler::accumulator::HistogramEntry;
use crate::tui::theme::{BLOCK_CHARS, BLOCK_FULL, Theme};

struct HistogramWidget<'a> {
    entries: &'a VecDeque<HistogramEntry>,
    theme: &'a Theme,
}

impl Widget for HistogramWidget<'_> {
    #[allow(clippy::cast_possible_truncation)]
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 || area.width < 2 {
            return;
        }

        let legend_height: u16 = 1;
        let chart_height = area.height.saturating_sub(legend_height);
        if chart_height == 0 {
            return;
        }

        let chart_width = area.width as usize;
        let num_columns = chart_width.min(self.entries.len());

        for j in 0..num_columns {
            let entry_idx = self.entries.len() - 1 - j;
            let entry = &self.entries[entry_idx];
            let col_x = area.x + area.width - 1 - j as u16;

            let mut pip_stack: Vec<(char, Style)> = Vec::new();
            if entry.high_jit_load {
                pip_stack.push((BLOCK_FULL, self.theme.histo_jit_load));
            }
            if entry.high_invalidation_or_smc {
                pip_stack.push((BLOCK_FULL, self.theme.histo_smc));
            }
            if entry.high_sigbus {
                pip_stack.push((BLOCK_FULL, self.theme.histo_sigbus));
            }
            if entry.high_softfloat {
                pip_stack.push((BLOCK_FULL, self.theme.histo_softfloat));
            }

            let load = entry.load_percent.clamp(0.0, 100.0);
            let bar_style = if load >= 75.0 {
                self.theme.load_high
            } else if load >= 50.0 {
                self.theme.load_medium
            } else {
                self.theme.load_normal
            };

            let rounded_down = (load / 10.0).floor() * 10.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let tens_digit = (rounded_down / 10.0) as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let digit_percent = (load - rounded_down).floor() as usize;

            for i in 0..chart_height as usize {
                let cell_y = area.y + chart_height - 1 - i as u16;

                let pip_char = if tens_digit > i {
                    BLOCK_CHARS[BLOCK_CHARS.len() - 1]
                } else if tens_digit == i && digit_percent < BLOCK_CHARS.len() {
                    BLOCK_CHARS[digit_percent]
                } else {
                    ' '
                };

                let (final_char, final_style) = if i < pip_stack.len() {
                    let (pip_c, pip_s) = pip_stack[i];
                    if pip_char <= BLOCK_CHARS[i.min(BLOCK_CHARS.len() - 1)] {
                        (pip_c, pip_s)
                    } else {
                        (pip_char, pip_s)
                    }
                } else {
                    (pip_char, bar_style)
                };

                if final_char != ' ' && cell_y >= area.y && col_x >= area.x {
                    buf[(col_x, cell_y)]
                        .set_char(final_char)
                        .set_style(final_style);
                }
            }
        }

        let legend_y = area.y + chart_height;
        if legend_y < area.y + area.height {
            let legend_area = Rect::new(area.x, legend_y, area.width, 1);
            let legend = Line::from(vec![
                Span::styled("\u{25A0} High JIT", self.theme.histo_jit_load),
                Span::raw("  "),
                Span::styled("\u{25A0} SMC", self.theme.histo_smc),
                Span::raw("  "),
                Span::styled("\u{25A0} SIGBUS", self.theme.histo_sigbus),
                Span::raw("  "),
                Span::styled("\u{25A0} Softfloat", self.theme.histo_softfloat),
            ]);
            Paragraph::new(legend).render(legend_area, buf);
        }
    }
}

pub fn render(
    frame: &mut ratatui::Frame,
    area: Rect,
    histogram: &VecDeque<HistogramEntry>,
    theme: &Theme,
) {
    if area.height < 2 || area.width < 2 {
        return;
    }

    if histogram.is_empty() {
        let paragraph = Paragraph::new("Waiting for data...");
        frame.render_widget(paragraph, area);
        return;
    }

    let widget = HistogramWidget {
        entries: histogram,
        theme,
    };
    frame.render_widget(widget, area);
}
