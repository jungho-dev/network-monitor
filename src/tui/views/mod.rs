//! Capture screen assembly: app bar, filter row, main cards, throughput strip,
//! footer chips with hints, grips, and the help overlay.

pub mod chrome;
pub mod common;
pub mod filter;
pub mod packets;
pub mod stats;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Sparkline};

use crate::tui::cards::{CardId, HitAreas, UiAction};
use crate::tui::format::{fmt_bytes, fmt_count, text_width};
use crate::tui::i18n::{self, CAPTURE_HELP, Msg};
use crate::tui::layout::SplitGroup;
use crate::tui::state::{App, CaptureSource, InputMode};
use crate::tui::theme;
use crate::tui::views::chrome::{
    Chip, HintSet, context_hints, render_app_bar, render_base_and_gate, render_footer,
    render_help_overlay,
};
use crate::tui::views::common::{render_column_handles, render_split_handles};

// Hex pane height: a thin border plus four content rows (64 bytes).
const HEX_HEIGHT: u16 = 6;
// The throughput strip is the first row to go when the screen is short.
const THROUGHPUT_MIN_HEIGHT: u16 = 24;

// 1. frame --------------------------------------------------------------------
pub fn render_frame(frame: &mut Frame, app: &mut App) {
    app.ui.hits = HitAreas::default();
    if render_base_and_gate(frame, app.ui.lang) {
        return;
    }
    let area = frame.area();
    let strip = area.height >= THROUGHPUT_MIN_HEIGHT;
    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
    ];
    if strip {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(2));
    let rows = Layout::vertical(constraints).split(area);
    render_capture_bar(frame, rows[0], app);
    filter::render_filter_row(frame, rows[1], app);
    render_body(frame, rows[2], app);
    if strip {
        render_throughput(frame, rows[3], app);
    }
    render_capture_footer(frame, rows[rows.len() - 1], app);
    render_split_handles(frame, &app.ui);
    render_column_handles(frame, &app.ui);
    if app.ui.show_help {
        render_help_overlay(frame, CAPTURE_HELP, app.ui.lang);
    }
}

// 2. body ---------------------------------------------------------------------
// A statistics view replaces the main area; otherwise list and detail side by
// side over the hex pane. Rects are indexed by card, so reordering never
// changes these call sites.
fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if let Some(kind) = app.overlay {
        stats::render_overlay(frame, app, area, kind);
        return;
    }
    let (main, hex) = if app.show_hex {
        let cards = app.ui.layout.split_cards(
            &mut app.ui.hits,
            SplitGroup::CaptureRows,
            area,
            &[area.height.saturating_sub(HEX_HEIGHT)],
            &[5, 3],
        );
        (cards[0], Some(cards[1]))
    } else {
        (area, None)
    };
    if app.show_detail {
        let width = main.width;
        let cards = app.ui.layout.split_cards(
            &mut app.ui.hits,
            SplitGroup::CaptureCols,
            main,
            &[width * 62 / 100],
            &[20, 16],
        );
        packets::render_packet_list(frame, app, cards[0]);
        packets::render_detail(frame, app, cards[1]);
    } else {
        packets::render_packet_list(frame, app, main);
    }
    if let Some(hex_area) = hex {
        packets::render_hex(frame, app, hex_area);
    }
}

// 3. app bar ------------------------------------------------------------------
// Left: source and capture state. Right: counters and the freshness dot.
fn render_capture_bar(frame: &mut Frame, area: Rect, app: &App) {
    let lang = app.ui.lang;
    let (source_label, source_name, live_source) = match &app.source {
        CaptureSource::Interface(name) => (Msg::BarInterface, name.as_str(), true),
        CaptureSource::File(name) => (Msg::BarFile, name.as_str(), false),
    };
    let (state_text, state_style) = if app.paused {
        (Msg::BarPaused, theme::STYLE_WARN)
    } else if app.follow {
        (Msg::BarLive, theme::STYLE_OK)
    } else {
        (Msg::BarBrowse, theme::STYLE_DIM)
    };
    let detail = vec![
        Span::styled(
            format!("  {} ", i18n::text(lang, source_label)),
            theme::STYLE_DIM,
        ),
        Span::styled(source_name.to_string(), theme::STYLE_ACCENT),
        Span::raw("  "),
        Span::styled(format!("● {}", i18n::text(lang, state_text)), state_style),
    ];
    let mut right = vec![
        Span::styled(fmt_count(app.total_received), theme::STYLE_TEXT),
        Span::styled(
            format!(" {}", i18n::text(lang, Msg::BarPackets)),
            theme::STYLE_DIM,
        ),
        Span::styled("  ·  ", theme::STYLE_FAINT),
        Span::styled(fmt_count(app.total_dropped), theme::STYLE_TEXT),
        Span::styled(
            format!(" {}", i18n::text(lang, Msg::BarDropped)),
            theme::STYLE_DIM,
        ),
    ];
    if live_source {
        right.push(Span::raw("  "));
        match app.last_packet_at {
            None => right.push(Span::styled(
                format!("○ {}", i18n::text(lang, Msg::BarWaiting)),
                theme::STYLE_DIM,
            )),
            Some(last) if app.is_idle() => {
                let secs = app.clock.saturating_duration_since(last).as_secs() as i64;
                right.push(Span::styled(
                    format!("● {}", i18n::idle_for(lang, secs)),
                    theme::STYLE_WARN,
                ));
            }
            Some(_) => {
                let rate = app.throughput_pps.back().copied().unwrap_or(0);
                right.push(Span::styled("● ", theme::STYLE_OK));
                right.push(Span::styled(
                    format!("{rate} {}", i18n::text(lang, Msg::UnitPacketsPerSec)),
                    theme::STYLE_DIM,
                ));
            }
        }
    }
    right.push(Span::raw(" "));
    render_app_bar(frame, area, detail, right);
}

// 4. throughput strip ---------------------------------------------------------
fn render_throughput(frame: &mut Frame, area: Rect, app: &App) {
    let lang = app.ui.lang;
    let data: Vec<u64> = app.throughput_pps.iter().copied().collect();
    let rate = data.last().copied().unwrap_or(0);
    let bytes = app.throughput_bps.back().copied().unwrap_or(0);
    let label = format!(
        " {} {rate} {} {}/s ",
        i18n::text(lang, Msg::ThroughputLabel),
        i18n::text(lang, Msg::UnitPacketsPerSec),
        fmt_bytes(bytes)
    );
    let label_width = (text_width(&label) as u16).min(area.width);
    let columns =
        Layout::horizontal([Constraint::Length(label_width), Constraint::Min(0)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label, theme::STYLE_DIM)))
            .style(Style::default().bg(theme::BG_BASE)),
        columns[0],
    );
    frame.render_widget(
        Sparkline::default()
            .data(&data)
            .style(Style::default().fg(theme::ACCENT_TCP).bg(theme::BG_BASE)),
        columns[1],
    );
}

// 5. footer -------------------------------------------------------------------
// Left chips in drop order (language, help, and quit are added by the chrome).
fn render_capture_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let lang = app.ui.lang;
    let pause_label = if app.paused {
        Msg::ChipResume
    } else {
        Msg::ChipPause
    };
    let left = vec![
        Chip {
            action: UiAction::TogglePause,
            key: "Space",
            label: i18n::text(lang, pause_label).to_string(),
            active: app.paused,
        },
        Chip {
            action: UiAction::OpenFilter,
            key: "/",
            label: i18n::text(lang, Msg::ChipFilter).to_string(),
            active: false,
        },
        Chip {
            action: UiAction::OpenColumns,
            key: "c",
            label: i18n::text(lang, Msg::ChipColumns).to_string(),
            active: false,
        },
        Chip {
            action: UiAction::SaveLog,
            key: "w",
            label: i18n::text(lang, Msg::ChipSave).to_string(),
            active: false,
        },
        Chip {
            action: UiAction::ResetLayout,
            key: "l",
            label: i18n::text(lang, Msg::ChipResetLayout).to_string(),
            active: false,
        },
    ];
    let focused = app.ui.focused_card();
    let pick = |msgs: &[Msg]| -> Vec<&'static str> {
        msgs.iter().map(|msg| i18n::text(lang, *msg)).collect()
    };
    let context = match app.mode {
        InputMode::FilterInput | InputMode::Search => pick(&[Msg::HintApply, Msg::HintEditCancel]),
        InputMode::ColumnConfig => pick(&[Msg::HintToggleColumn, Msg::HintColumnsDone]),
        InputMode::Normal => {
            let sortable = !crate::tui::columns::card_columns(focused).is_empty();
            let mut hints = context_hints(&app.ui, focused.kind(), sortable);
            if app.overlay.is_some() {
                hints.extend(pick(&[Msg::HintCloseView]));
            } else if focused == CardId::PacketList {
                hints.extend(pick(&[Msg::HintFilter, Msg::HintPause]));
            }
            hints
        }
    };
    let tail = pick(&[Msg::HintNextCard, Msg::HintAllKeys]);
    let hints = HintSet { context, tail };
    render_footer(frame, &mut app.ui, area, left, &hints);
}

#[cfg(test)]
mod tests;
