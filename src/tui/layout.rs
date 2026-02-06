// SPDX-License-Identifier: MIT
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct PanelState {
    pub name: &'static str,
    pub collapsed: bool,
    pub min_height: u16,
}

pub fn build_layout(panels: &[PanelState], area: Rect) -> Vec<Rect> {
    let constraints: Vec<Constraint> = panels
        .iter()
        .map(|p| {
            if p.collapsed {
                Constraint::Length(3)
            } else {
                Constraint::Min(p.min_height)
            }
        })
        .collect();

    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}
