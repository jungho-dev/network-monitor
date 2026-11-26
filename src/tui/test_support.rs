//! Test helpers: packet builders, sample apps, key events, and buffer dumps.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::text::Span;

use network_monitor_core::analysis::{ThreatAnnotation, ThreatKind};
use network_monitor_core::capture::{Linktype, RawPacket};
use network_monitor_core::decode::{DecodedPacket, decode_packet};

use crate::tui::state::{App, CaptureSource};

// 1. packets ------------------------------------------------------------------
// 64 zero bytes: a packet that decodes to an Ethernet frame only.
pub fn blank_packet(number: u64) -> DecodedPacket {
    decode_packet(RawPacket {
        number,
        timestamp: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        wire_len: 64,
        data: vec![0u8; 64],
        linktype: Linktype::Ethernet,
    })
}

// Ethernet + IPv4 + TCP (+ payload) from the given endpoints.
pub fn tcp_packet(
    number: u64,
    src: [u8; 4],
    src_port: u16,
    dst: [u8; 4],
    dst_port: u16,
    wire_len: u32,
    payload: &[u8],
) -> DecodedPacket {
    let mut data = Vec::new();
    data.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
    data.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x01]);
    data.extend_from_slice(&[0x08, 0x00]);
    let total = (20 + 20 + payload.len()) as u16;
    data.extend_from_slice(&[0x45, 0x00]);
    data.extend_from_slice(&total.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0x40, 0, 64, 6, 0, 0]);
    data.extend_from_slice(&src);
    data.extend_from_slice(&dst);
    data.extend_from_slice(&src_port.to_be_bytes());
    data.extend_from_slice(&dst_port.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    data.extend_from_slice(&[0x50, 0x18, 0xff, 0xff, 0, 0, 0, 0]);
    data.extend_from_slice(payload);
    decode_packet(RawPacket {
        number,
        timestamp: chrono::DateTime::from_timestamp(1_700_000_000 + number as i64, 0).unwrap(),
        wire_len,
        data,
        linktype: Linktype::Ethernet,
    })
}

// A TCP packet to an uncommon port carrying a threat annotation of `kind`.
pub fn threat_packet(number: u64, kind: ThreatKind) -> DecodedPacket {
    let mut packet = tcp_packet(
        number,
        [10, 0, 0, 1],
        40000,
        [93, 184, 216, 9],
        4444,
        60,
        b"",
    );
    packet.threat = Some(ThreatAnnotation {
        kind,
        detail: "test".to_string(),
    });
    packet
}

// 2. apps and keys ------------------------------------------------------------
pub fn capture_app() -> App {
    App::new(
        1024,
        Some(PathBuf::from("tui-test.toml")),
        CaptureSource::Interface("eth0".into()),
    )
}

// Adds `count` packets of mixed endpoints to the ring.
pub fn populate(app: &mut App, count: u64) {
    for number in 0..count {
        let last = (number % 200) as u8 + 2;
        let packet = tcp_packet(
            number,
            [10, 0, 0, 1],
            40000 + (number % 50) as u16,
            [93, 184, 216, last],
            443,
            60 + (number % 90) as u32,
            b"",
        );
        app.packets.push(packet);
    }
}

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn shift_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

// 3. buffer dumps -------------------------------------------------------------
pub fn draw<F: FnMut(&mut Frame)>(width: u16, height: u16, mut render: F) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame)).unwrap();
    terminal.backend().buffer().clone()
}

// Cells whose glyph is outside the budget: ASCII, Box Drawing and Block
// Elements (U+2500-259F), `·`, `…`, `●`, `○`, `↑`, `↓`, and Hangul syllables for the
// Korean text. Returns (x, y, symbol) per offending cell.
pub fn glyph_budget_violations(buffer: &Buffer) -> Vec<(u16, u16, String)> {
    let area = buffer.area;
    let mut found = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let allowed = cell.symbol().chars().all(|glyph| {
                glyph.is_ascii()
                    || ('\u{2500}'..='\u{259F}').contains(&glyph)
                    || ('\u{AC00}'..='\u{D7A3}').contains(&glyph)
                    || "·…●○↑↓".contains(glyph)
            });
            if !allowed {
                found.push((x, y, cell.symbol().to_string()));
            }
        }
    }
    found
}

// One string per row; the cell after a wide glyph is skipped so assertions
// match what the user reads.
pub fn dump(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    let mut rows = Vec::new();
    for y in 0..area.height {
        let mut row = String::new();
        let mut skip = 0usize;
        for x in 0..area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = buffer.cell((x, y)).map_or(" ", |cell| cell.symbol());
            skip = Span::raw(symbol).width().saturating_sub(1);
            row.push_str(symbol);
        }
        rows.push(row);
    }
    rows
}
