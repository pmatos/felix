// SPDX-License-Identifier: MIT
use std::collections::VecDeque;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::input::Action;
use super::layout::{PanelState, build_layout};
use super::panels::{header, histogram, jit_stats, mem_stats};
use super::replay_controls::{self, ReplayControls};
use super::theme::{COLLAPSED_MARKER, SELECTED_MARKER, Theme};
use crate::datasource::SessionMetadata;
use crate::sampler::accumulator::{ComputedFrame, HistogramEntry};

const HISTOGRAM_CAPACITY: usize = 200;
const REPLAY_BAR_HEIGHT: u16 = 4;

pub struct App {
    pub panels: Vec<PanelState>,
    pub selected_panel: usize,
    pub latest_frame: Option<ComputedFrame>,
    pub histogram: VecDeque<HistogramEntry>,
    pub metadata: SessionMetadata,
    pub is_replay: bool,
    pub should_quit: bool,
    pub theme: Theme,
    replay_controls: Option<ReplayControls>,
}

impl App {
    #[must_use]
    pub fn new(metadata: SessionMetadata, is_replay: bool) -> Self {
        let panels = vec![
            PanelState {
                name: "FEX JIT Stats",
                collapsed: false,
                min_height: 26,
            },
            PanelState {
                name: "FEX Memory Usage",
                collapsed: false,
                min_height: 15,
            },
            PanelState {
                name: "Total JIT usage",
                collapsed: false,
                min_height: 12,
            },
        ];

        let replay_controls = if is_replay {
            Some(ReplayControls::new(0))
        } else {
            None
        };

        Self {
            panels,
            selected_panel: 0,
            latest_frame: None,
            histogram: VecDeque::with_capacity(HISTOGRAM_CAPACITY),
            metadata,
            is_replay,
            should_quit: false,
            theme: Theme::default(),
            replay_controls,
        }
    }

    pub fn update_frame(&mut self, frame: ComputedFrame) {
        let entry = frame.histogram_entry.clone();
        self.latest_frame = Some(frame);

        if self.histogram.len() >= HISTOGRAM_CAPACITY {
            self.histogram.pop_front();
        }
        self.histogram.push_back(entry);
    }

    pub fn set_replay_total_frames(&mut self, total: usize) {
        if let Some(ref mut controls) = self.replay_controls {
            controls.total_frames = total;
        }
    }

    #[must_use]
    pub fn replay_controls(&self) -> Option<&ReplayControls> {
        self.replay_controls.as_ref()
    }

    pub fn replay_controls_mut(&mut self) -> Option<&mut ReplayControls> {
        self.replay_controls.as_mut()
    }

    pub fn handle_action(&mut self, action: &Action) {
        match *action {
            Action::Quit => self.should_quit = true,
            Action::PanelUp => {
                if self.selected_panel > 0 {
                    self.selected_panel -= 1;
                }
            }
            Action::PanelDown => {
                if self.selected_panel + 1 < self.panels.len() {
                    self.selected_panel += 1;
                }
            }
            Action::ToggleCollapse => {
                if let Some(panel) = self.panels.get_mut(self.selected_panel) {
                    panel.collapsed = !panel.collapsed;
                }
            }
            Action::TogglePause => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.toggle_pause();
                }
            }
            Action::SeekForward => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.seek_forward();
                    controls.paused = true;
                }
            }
            Action::SeekBackward => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.seek_backward();
                    controls.paused = true;
                }
            }
            Action::SpeedUp => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.speed_up();
                }
            }
            Action::SpeedDown => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.speed_down();
                }
            }
            Action::SeekStart => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.seek_start();
                }
            }
            Action::SeekEnd => {
                if let Some(ref mut controls) = self.replay_controls {
                    controls.seek_end();
                }
            }
            Action::IncreaseSamplePeriod | Action::DecreaseSamplePeriod | Action::None => {}
        }
    }

    pub fn render(&self, frame: &mut ratatui::Frame) {
        let outer = frame.area();
        if outer.height < 2 || outer.width < 5 {
            return;
        }

        let has_replay_bar = self.replay_controls.is_some();

        let vertical = if has_replay_bar {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(REPLAY_BAR_HEIGHT),
                ])
                .split(outer)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(outer)
        };

        let header_area = vertical[0];
        let body_area = vertical[1];

        let sample_period_ns = self.latest_frame.as_ref().map(|f| f.sample_period_ns);
        header::render(
            frame,
            header_area,
            &self.metadata,
            self.is_replay,
            sample_period_ns,
            &self.theme,
        );

        if has_replay_bar && let Some(ref controls) = self.replay_controls {
            let controls_area = vertical[2];
            let period = sample_period_ns.unwrap_or(1_000_000_000);
            replay_controls::render(frame, controls_area, controls, period, &self.theme);
        }

        let areas = build_layout(&self.panels, body_area);

        for (i, (panel, area)) in self.panels.iter().zip(areas.iter()).enumerate() {
            let is_selected = i == self.selected_panel;

            let sel_mark = if is_selected {
                SELECTED_MARKER[1]
            } else {
                SELECTED_MARKER[0]
            };
            let col_mark = if panel.collapsed {
                COLLAPSED_MARKER[1]
            } else {
                COLLAPSED_MARKER[0]
            };

            let title = format!("{sel_mark} {col_mark} {}", panel.name);

            let border_style = if is_selected {
                self.theme.border_selected
            } else {
                self.theme.border_normal
            };

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style)
                .title_style(self.theme.title);

            if panel.collapsed {
                frame.render_widget(block, *area);
            } else {
                let inner = block.inner(*area);
                frame.render_widget(block, *area);

                if inner.width < 2 || inner.height < 1 {
                    continue;
                }

                match (i, &self.latest_frame) {
                    (0, Some(data)) => {
                        jit_stats::render(frame, inner, data, &self.metadata, &self.theme);
                    }
                    (1, Some(data)) => {
                        mem_stats::render(frame, inner, data, &self.theme);
                    }
                    (2, _) => {
                        histogram::render(frame, inner, &self.histogram, &self.theme);
                    }
                    _ => {
                        frame.render_widget(Paragraph::new("Waiting for data..."), inner);
                    }
                }
            }
        }
    }
}
