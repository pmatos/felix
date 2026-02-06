// SPDX-License-Identifier: MIT
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::sampler::accumulator::ComputedFrame;
use crate::tui::theme::Theme;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / GIB as f64;
        format!("{val:.1} GiB")
    } else if bytes >= MIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / MIB as f64;
        format!("{val:.0} MiB")
    } else if bytes >= KIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / KIB as f64;
        format!("{val:.0} KiB")
    } else {
        format!("{bytes} B")
    }
}

pub fn render(frame: &mut ratatui::Frame, area: Rect, data: &ComputedFrame, _theme: &Theme) {
    if area.height < 2 || area.width < 10 {
        return;
    }

    if data.mem.total_anon == 0 {
        let paragraph = Paragraph::new("Waiting for memory data...");
        frame.render_widget(paragraph, area);
        return;
    }

    let mem = &data.mem;
    let lines = vec![
        Line::from(format!(
            "Total FEX Anon memory resident: {}",
            format_bytes(mem.total_anon)
        )),
        Line::from(format!(
            "    JIT resident:             {}",
            format_bytes(mem.jit_code)
        )),
        Line::from(format!(
            "    OpDispatcher resident:     {}",
            format_bytes(mem.op_dispatcher)
        )),
        Line::from(format!(
            "    Frontend resident:         {}",
            format_bytes(mem.frontend)
        )),
        Line::from(format!(
            "    CPUBackend resident:       {}",
            format_bytes(mem.cpu_backend)
        )),
        Line::from(format!(
            "    Lookup cache resident:     {}",
            format_bytes(mem.lookup)
        )),
        Line::from(format!(
            "    Lookup L1 cache resident:  {}",
            format_bytes(mem.lookup_l1)
        )),
        Line::from(format!(
            "    ThreadStates resident:     {}",
            format_bytes(mem.thread_states)
        )),
        Line::from(format!(
            "    BlockLinks resident:       {}",
            format_bytes(mem.block_links)
        )),
        Line::from(format!(
            "          Misc resident:       {}",
            format_bytes(mem.misc)
        )),
        Line::from(format!(
            "    JEMalloc resident:         {}",
            format_bytes(mem.jemalloc)
        )),
        Line::from(format!(
            "    Unaccounted resident:      {}",
            format_bytes(mem.unaccounted)
        )),
        Line::from(format!(
            "                 Largest:      {} [0x{:x}, 0x{:x})",
            format_bytes(mem.largest_anon.size),
            mem.largest_anon.begin,
            mem.largest_anon.end,
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1 KiB");
        assert_eq!(format_bytes(2048), "2 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1 MiB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }
}
