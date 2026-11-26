//! Alerts tab: the threat table at the current output level over the
//! bookmarks card. Threat details come from the engine and are not translated.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::cards::CardId;
use crate::tui::i18n::{self, Msg};
use crate::tui::layout::SplitGroup;
use crate::tui::level::{OutputLevel, level_of};
use crate::tui::sort::SortKey;
use crate::tui::state::App;
use crate::tui::theme;
use crate::tui::views::common::{
    CardTable, TableRow, all_columns_shown, empty_pair, render_card_table, sort_rows,
};
use crate::tui::views::stats;

// 1. tab ----------------------------------------------------------------------
// Threat table on top (the larger slot), bookmarks below; rects are indexed
// by card, so reordering never changes these call sites.
pub fn render_alerts_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let cards = app.ui.layout.split_cards(
        &mut app.ui.hits,
        SplitGroup::CaptureAlerts,
        area,
        &[area.height * 60 / 100],
        &[5, 4],
    );
    render_threat_table(frame, app, cards[0]);
    stats::render_bookmarks(frame, app, cards[1]);
}

// 2. threat table -------------------------------------------------------------
// Rows are sorted first so the zebra follows the drawn order; error kinds use
// the critical foreground and warn kinds the muted tone, never a fill.
fn render_threat_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;

    // 1. rows with typed sort keys and the level tone
    let rows: Vec<TableRow> = app
        .stats
        .threats
        .iter()
        .map(|row| {
            let label = row.kind.label();
            let tone = match level_of(Some(row.kind), None, false) {
                OutputLevel::Error => theme::COLOR_CRIT,
                OutputLevel::Warn => theme::TONE_WARN,
                OutputLevel::Info | OutputLevel::Debug => theme::FG_TEXT,
            };
            TableRow {
                id: row.number,
                cells: vec![
                    vec![Span::raw(row.time.format("%H:%M:%S%.3f").to_string())],
                    vec![Span::raw(label)],
                    vec![Span::raw(row.number.to_string())],
                    vec![Span::raw(row.detail.replace('→', "->"))],
                ],
                keys: vec![
                    SortKey::Num(row.time.timestamp_millis()),
                    SortKey::Text(label.to_string()),
                    SortKey::Num(row.number as i64),
                    SortKey::Text(row.detail.clone()),
                ],
                style: Style::default().fg(tone),
            }
        })
        .collect();

    // 2. sort, then the zebra per drawn row
    let mut rows = sort_rows(&app.ui, CardId::ThreatTable, rows);
    for (position, row) in rows.iter_mut().enumerate() {
        let zebra = if position.is_multiple_of(2) {
            theme::BG_PANEL
        } else {
            theme::BG_ZEBRA
        };
        row.style = row.style.bg(zebra);
    }

    // 3. selection and card
    let total = rows.len();
    let ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
    app.ui.sync_selection(CardId::ThreatTable, ids);
    let table = CardTable {
        card: CardId::ThreatTable,
        title: i18n::title_count(i18n::text(lang, Msg::TitleThreats), total),
        hint: i18n::text(lang, Msg::HintBackToPackets).to_string(),
        rows,
        total,
        offset: 0,
        windowed: false,
        shown_columns: all_columns_shown(),
        empty: empty_pair(lang, Msg::EmptyNoAlerts, Msg::EmptyNoAlertsHint),
    };
    render_card_table(frame, &mut app.ui, area, table);
}
