//! Default Style palette: surface ladder, text, state, packet accents,
//! scrollbar and grip colors, plus shared styles.
//!
//! Every color is a `Color::Indexed(n)` constant from this module; no named
//! 16-colors and no `Color::Rgb`. Border ladder: idle 240 rounded, hover 67
//! rounded, focus 39 thick with an inverted title, critical 203 first.
//! `display.color_scheme` in config.toml is accepted but not applied: the
//! palette below is the only scheme.

use ratatui::style::{Color, Modifier, Style};

use network_monitor_core::decode::ColorHint;

// 1. surfaces and borders -----------------------------------------------------
pub const BG_BASE: Color = Color::Indexed(233);
pub const BG_ZEBRA: Color = Color::Indexed(234);
pub const BG_PANEL: Color = Color::Indexed(235);
pub const BG_BAR: Color = Color::Indexed(236);
pub const BG_TONAL: Color = Color::Indexed(238);
pub const BG_SELECTED: Color = Color::Indexed(24);
pub const BG_SELECTED_IDLE: Color = Color::Indexed(237);
pub const BORDER_DIM: Color = Color::Indexed(240);
pub const BORDER_FOCUS: Color = Color::Indexed(39);
pub const BORDER_HOVER: Color = Color::Indexed(67);

// 2. text ---------------------------------------------------------------------
pub const FG_TEXT: Color = Color::Indexed(252);
pub const FG_STRONG: Color = Color::Indexed(231);
pub const FG_DIM: Color = Color::Indexed(247);
pub const FG_FAINT: Color = Color::Indexed(243);
pub const FG_ACCENT: Color = Color::Indexed(81);
pub const FG_ON_ACCENT: Color = Color::Indexed(16);
pub const TITLE_IDLE: Color = Color::Indexed(250);
pub const TITLE_HOVER: Color = FG_TEXT;
pub const HINT_KEY: Color = Color::Indexed(250);
pub const TAB_IDLE_NAME: Color = Color::Indexed(248);

// 3. state --------------------------------------------------------------------
pub const COLOR_OK: Color = Color::Indexed(78);
pub const COLOR_WARN: Color = Color::Indexed(220);
pub const COLOR_CRIT: Color = Color::Indexed(203);
// Error toast fill only (with white text).
pub const COLOR_CRIT_BG: Color = Color::Indexed(160);
pub const COLOR_ANOMALY: Color = Color::Indexed(208);

// 4. packet accents -----------------------------------------------------------
pub const DIR_IN: Color = Color::Indexed(78);
pub const DIR_OUT: Color = Color::Indexed(75);
pub const ACCENT_TCP: Color = Color::Indexed(74);
pub const ACCENT_UDP: Color = Color::Indexed(78);
pub const ACCENT_DNS_ARP: Color = Color::Indexed(179);
pub const ACCENT_ICMP: Color = Color::Indexed(170);
pub const ACCENT_TLS: Color = Color::Indexed(111);
pub const DETAIL_HEADER: Color = Color::Indexed(81);
pub const BOOKMARK: Color = Color::Indexed(220);
pub const SEARCH_BG: Color = Color::Indexed(178);
pub const FILTER_ACTIVE: Color = Color::Indexed(78);
pub const FILTER_EDIT: Color = Color::Indexed(221);

// 5. scrollbar and grips ------------------------------------------------------
// Gray family only; never the focus or accent color.
pub const SCROLL_TRACK: Color = Color::Indexed(238);
pub const SCROLL_TRACK_HOT: Color = Color::Indexed(241);
pub const SCROLL_THUMB: Color = Color::Indexed(245);
pub const SCROLL_THUMB_FOCUS: Color = Color::Indexed(250);
pub const SCROLL_THUMB_HOVER: Color = Color::Indexed(254);
pub const SCROLL_THUMB_DRAG: Color = Color::Indexed(231);
pub const SPLIT_GRIP_HOT: Color = Color::Indexed(250);
pub const SPLIT_GRIP_DRAG: Color = Color::Indexed(231);

// 6. shared styles ------------------------------------------------------------
pub const STYLE_TEXT: Style = Style::new().fg(FG_TEXT);
pub const STYLE_DIM: Style = Style::new().fg(FG_DIM);
pub const STYLE_FAINT: Style = Style::new().fg(FG_FAINT);
pub const STYLE_STRONG: Style = Style::new().fg(FG_STRONG).add_modifier(Modifier::BOLD);
pub const STYLE_ACCENT: Style = Style::new().fg(FG_ACCENT).add_modifier(Modifier::BOLD);
pub const STYLE_OK: Style = Style::new().fg(COLOR_OK);
pub const STYLE_WARN: Style = Style::new().fg(COLOR_WARN);
pub const STYLE_CRIT: Style = Style::new().fg(COLOR_CRIT);
pub const STYLE_TABLE_HEADER: Style = Style::new().fg(FG_DIM).add_modifier(Modifier::BOLD);
pub const STYLE_SELECTED_ROW: Style = Style::new().bg(BG_SELECTED).fg(FG_STRONG);
pub const STYLE_SELECTED_ROW_IDLE: Style = Style::new().bg(BG_SELECTED_IDLE);
pub const STYLE_SEARCH_ROW: Style = Style::new().fg(FG_ON_ACCENT).bg(SEARCH_BG);

// 7. protocol accent ----------------------------------------------------------
pub fn proto_color(hint: ColorHint) -> Color {
    match hint {
        ColorHint::Tcp => ACCENT_TCP,
        ColorHint::Udp => ACCENT_UDP,
        ColorHint::Arp | ColorHint::Dns => ACCENT_DNS_ARP,
        ColorHint::Icmp => ACCENT_ICMP,
        ColorHint::Tls => ACCENT_TLS,
        ColorHint::Retransmission => COLOR_CRIT,
        ColorHint::Other => FG_TEXT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_and_grip_colors_never_reuse_the_focus_or_accent_color() {
        for color in [
            SCROLL_TRACK,
            SCROLL_TRACK_HOT,
            SCROLL_THUMB,
            SCROLL_THUMB_FOCUS,
            SCROLL_THUMB_HOVER,
            SCROLL_THUMB_DRAG,
            SPLIT_GRIP_HOT,
            SPLIT_GRIP_DRAG,
        ] {
            assert_ne!(color, BORDER_FOCUS);
            assert_ne!(color, FG_ACCENT);
        }
    }

    #[test]
    fn packet_accents_never_reuse_the_focus_color() {
        for color in [
            DIR_IN,
            DIR_OUT,
            ACCENT_TCP,
            ACCENT_UDP,
            ACCENT_DNS_ARP,
            ACCENT_ICMP,
            ACCENT_TLS,
        ] {
            assert_ne!(color, BORDER_FOCUS);
        }
    }

    #[test]
    fn proto_color_maps_every_hint_to_an_indexed_color() {
        for hint in [
            ColorHint::Tcp,
            ColorHint::Udp,
            ColorHint::Arp,
            ColorHint::Icmp,
            ColorHint::Dns,
            ColorHint::Tls,
            ColorHint::Retransmission,
            ColorHint::Other,
        ] {
            assert!(matches!(proto_color(hint), Color::Indexed(_)));
        }
        assert_eq!(proto_color(ColorHint::Retransmission), COLOR_CRIT);
    }
}
