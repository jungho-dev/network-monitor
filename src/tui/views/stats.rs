//! Statistics cards of the Flows, Stats, Alerts, and Stream tabs: top talkers,
//! protocol shares, flows, the connection timeline, bookmarks, and the
//! followed TCP stream.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::tui::cards::CardId;
use crate::tui::format::{fmt_bytes, fmt_count, stable_id};
use crate::tui::i18n::{self, Msg};
use crate::tui::sort::SortKey;
use crate::tui::state::App;
use crate::tui::stats::printable;
use crate::tui::theme;
use crate::tui::views::common::{
    CardTable, TableRow, TextCard, all_columns_shown, empty_pair, render_card_table,
    render_text_card, sort_rows,
};

const TIMELINE_WIDTH: usize = 40;

// Sorts the rows, keeps the selection on the same item, and draws the card.
fn render_stats_table(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    card: CardId,
    title: String,
    rows: Vec<TableRow>,
    empty: Option<(String, String)>,
) {
    let lang = app.ui.lang;
    let rows = sort_rows(&app.ui, card, rows);
    let total = rows.len();
    let ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
    app.ui.sync_selection(card, ids);
    let table = CardTable {
        card,
        title,
        hint: i18n::text(lang, Msg::HintBackToPackets).to_string(),
        rows,
        total,
        offset: 0,
        windowed: false,
        shown_columns: all_columns_shown(),
        empty,
    };
    render_card_table(frame, &mut app.ui, area, table);
}

fn plain(text: String) -> Vec<Span<'static>> {
    vec![Span::raw(text)]
}

pub fn render_talkers(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let rows: Vec<TableRow> = app
        .stats
        .talkers
        .iter()
        .map(|row| TableRow {
            id: stable_id(&row.endpoint),
            cells: vec![
                plain(row.endpoint.clone()),
                plain(fmt_count(row.packets)),
                plain(fmt_bytes(row.bytes)),
            ],
            keys: vec![
                SortKey::Text(row.endpoint.clone()),
                SortKey::Num(row.packets as i64),
                SortKey::Num(row.bytes as i64),
            ],
            style: Style::default(),
        })
        .collect();
    let title = i18n::text(lang, Msg::TitleTalkers).to_string();
    let empty = empty_pair(lang, Msg::EmptyNoRows, Msg::EmptyWaitingHint);
    render_stats_table(frame, app, area, CardId::StatTalkers, title, rows, empty);
}

pub fn render_protocols(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let total: u64 = app.stats.protocols.iter().map(|row| row.count).sum();
    let rows: Vec<TableRow> = app
        .stats
        .protocols
        .iter()
        .map(|row| {
            let pct = if total > 0 {
                row.count as f64 * 100.0 / total as f64
            } else {
                0.0
            };
            TableRow {
                id: stable_id(&row.name),
                cells: vec![
                    plain(row.name.clone()),
                    plain(fmt_count(row.count)),
                    plain(format!("{pct:.1}%")),
                    plain("█".repeat((pct / 2.0) as usize)),
                ],
                keys: vec![
                    SortKey::Text(row.name.clone()),
                    SortKey::Num(row.count as i64),
                    SortKey::Num((pct * 10.0) as i64),
                    SortKey::Num(row.count as i64),
                ],
                style: Style::default(),
            }
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleProtocols), total as usize);
    let empty = empty_pair(lang, Msg::EmptyNoRows, Msg::EmptyWaitingHint);
    render_stats_table(frame, app, area, CardId::StatProtocols, title, rows, empty);
}

pub fn render_flows(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let rows: Vec<TableRow> = app
        .stats
        .flows
        .iter()
        .map(|row| {
            let duration = (row.last_ms - row.first_ms) as f64 / 1000.0;
            TableRow {
                id: stable_id(&row.label),
                cells: vec![
                    plain(row.label.clone()),
                    plain(row.proto.to_string()),
                    plain(fmt_count(row.packets)),
                    plain(fmt_bytes(row.bytes)),
                    plain(format!("{duration:.1}s")),
                ],
                keys: vec![
                    SortKey::Text(row.label.clone()),
                    SortKey::Text(row.proto.to_string()),
                    SortKey::Num(row.packets as i64),
                    SortKey::Num(row.bytes as i64),
                    SortKey::Num(row.last_ms - row.first_ms),
                ],
                style: Style::default(),
            }
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleFlows), rows.len());
    let empty = empty_pair(lang, Msg::EmptyNoRows, Msg::EmptyWaitingHint);
    render_stats_table(frame, app, area, CardId::StatFlows, title, rows, empty);
}

pub fn render_bookmarks(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let rows: Vec<TableRow> = app
        .stats
        .bookmarks
        .iter()
        .map(|row| {
            let time = chrono::DateTime::from_timestamp_millis(row.time_ms)
                .map(|stamp| stamp.format("%H:%M:%S%.3f").to_string())
                .unwrap_or_default();
            TableRow {
                id: row.number,
                cells: vec![
                    vec![Span::styled(
                        format!("*{}", row.number),
                        Style::default().fg(theme::BOOKMARK),
                    )],
                    plain(time),
                    plain(row.endpoints.clone()),
                    plain(row.info.clone()),
                ],
                keys: vec![
                    SortKey::Num(row.number as i64),
                    SortKey::Num(row.time_ms),
                    SortKey::Text(row.endpoints.clone()),
                    SortKey::Text(row.info.clone()),
                ],
                style: Style::default(),
            }
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleBookmarks), app.bookmarks.len());
    let empty = empty_pair(lang, Msg::EmptyNoBookmarks, Msg::EmptyNoBookmarksHint);
    render_stats_table(frame, app, area, CardId::StatBookmarks, title, rows, empty);
}

// Horizontal bars of each flow between the earliest start and the latest end.
pub fn render_timeline(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let mut flows: Vec<(&str, i64, i64)> = app
        .stats
        .flows
        .iter()
        .map(|row| (row.label.as_str(), row.first_ms, row.last_ms))
        .collect();
    flows.sort_by_key(|row| row.1);
    let min = flows.iter().map(|row| row.1).min().unwrap_or(0);
    let max = flows.iter().map(|row| row.2).max().unwrap_or(1);
    let span = (max - min).max(1) as f64;
    let lines: Vec<Line<'static>> = flows
        .iter()
        .map(|(label, first, last)| {
            let start = (((first - min) as f64 / span) * TIMELINE_WIDTH as f64) as usize;
            let end = (((last - min) as f64 / span) * TIMELINE_WIDTH as f64).ceil() as usize;
            let length = end.saturating_sub(start).max(1);
            let bar = format!(
                "{}{}",
                " ".repeat(start.min(TIMELINE_WIDTH)),
                "▇".repeat(length.min(TIMELINE_WIDTH))
            );
            Line::from(format!("{bar:<width$} {label}", width = TIMELINE_WIDTH + 1))
        })
        .collect();
    let spec = TextCard {
        card: CardId::StatTimeline,
        title: i18n::text(lang, Msg::TitleTimeline).to_string(),
        hint: i18n::text(lang, Msg::HintBackToPackets).to_string(),
        lines,
        empty: empty_pair(lang, Msg::EmptyNoRows, Msg::EmptyWaitingHint),
    };
    render_text_card(frame, &mut app.ui, area, spec);
}

// Client lines first, then server lines, as printable text.
pub fn render_stream(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let (title, lines) = match &app.stream {
        Some((title, data)) => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            for chunk in printable(&data.client_to_server).split('\n') {
                lines.push(Line::from(Span::styled(
                    format!(">> {chunk}"),
                    Style::default().fg(theme::ACCENT_TCP),
                )));
            }
            for chunk in printable(&data.server_to_client).split('\n') {
                lines.push(Line::from(Span::styled(
                    format!("<< {chunk}"),
                    Style::default().fg(theme::ACCENT_UDP),
                )));
            }
            (
                format!(
                    "{}: {title} ({} >> {} <<)",
                    i18n::text(lang, Msg::TitleStream),
                    i18n::text(lang, Msg::StreamClient),
                    i18n::text(lang, Msg::StreamServer)
                ),
                lines,
            )
        }
        None => (i18n::text(lang, Msg::TitleStream).to_string(), Vec::new()),
    };
    let spec = TextCard {
        card: CardId::StatStream,
        title,
        hint: i18n::text(lang, Msg::HintBackToPackets).to_string(),
        lines,
        empty: empty_pair(lang, Msg::EmptyNoStream, Msg::EmptyNoStreamHint),
    };
    render_text_card(frame, &mut app.ui, area, spec);
}
