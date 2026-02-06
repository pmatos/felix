// SPDX-License-Identifier: MIT
use crossterm::event::KeyCode;

pub enum Action {
    Quit,
    PanelUp,
    PanelDown,
    ToggleCollapse,
    TogglePause,
    SeekForward,
    SeekBackward,
    SpeedUp,
    SpeedDown,
    SeekStart,
    SeekEnd,
    IncreaseSamplePeriod,
    DecreaseSamplePeriod,
    None,
}

pub fn handle_key(key: KeyCode, is_replay: bool) -> Action {
    match key {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Up => Action::PanelUp,
        KeyCode::Down => Action::PanelDown,
        KeyCode::Right if is_replay => Action::SeekForward,
        KeyCode::Right => Action::ToggleCollapse,
        KeyCode::Char('+' | '=') => Action::IncreaseSamplePeriod,
        KeyCode::Char('-' | '_') => Action::DecreaseSamplePeriod,
        KeyCode::Char(' ') if is_replay => Action::TogglePause,
        KeyCode::Left if is_replay => Action::SeekBackward,
        KeyCode::Char(']') if is_replay => Action::SpeedUp,
        KeyCode::Char('[') if is_replay => Action::SpeedDown,
        KeyCode::Home if is_replay => Action::SeekStart,
        KeyCode::End if is_replay => Action::SeekEnd,
        _ => Action::None,
    }
}
