//! Inspector screen: app bar, tab bar, one table card over one detail card per
//! tab, footer chips, grips, and the help overlay. Daemon data (process names,
//! destinations, addresses, alert titles) is shown as received.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::inspect::state::{
    AlertItem, ConnItem, DomainItem, HistoryItem, Inspect, InspectTab, Mode, ProcItem, SeverityRank,
};
use crate::tui::cards::{CardId, HitAreas, MAX_TABLE_COLUMNS, UiAction};
use crate::tui::columns::card_columns;
use crate::tui::format::{fit_display, fmt_bytes, fmt_count, fmt_timestamp, stable_id};
use crate::tui::i18n::{self, INSPECT_HELP, Lang, Msg};
use crate::tui::sort::SortKey;
use crate::tui::theme;
use crate::tui::views::chrome::{
    Chip, HintSet, context_hints, render_app_bar, render_base_and_gate, render_footer,
    render_help_overlay, render_tab_bar,
};
use crate::tui::views::common::{
    CardTable, TableRow, TextCard, all_columns_shown, empty_pair, render_card_table,
    render_column_handles, render_split_handles, render_text_card, sort_rows,
};

// Width of the label column in detail cards.
const DETAIL_LABEL_WIDTH: usize = 16;

// 1. frame --------------------------------------------------------------------
pub fn render_frame(frame: &mut Frame, inspect: &mut Inspect) {
    inspect.ui.hits = HitAreas::default();
    if render_base_and_gate(frame, inspect.ui.lang) {
        return;
    }
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);
    render_inspect_bar(frame, rows[0], inspect);
    let lang = inspect.ui.lang;
    let labels: Vec<&str> = InspectTab::ALL
        .iter()
        .map(|tab| i18n::text(lang, tab.label()))
        .collect();
    render_tab_bar(
        frame,
        &mut inspect.ui,
        rows[1],
        &labels,
        inspect.tab.index(),
    );
    render_tab_body(frame, rows[2], inspect);
    render_inspect_footer(frame, rows[3], inspect);
    render_split_handles(frame, &inspect.ui);
    render_column_handles(frame, &inspect.ui);
    if inspect.ui.show_help {
        render_help_overlay(frame, INSPECT_HELP, inspect.ui.lang);
    }
}

// 2. app bar ------------------------------------------------------------------
// Connection state with the daemon baseline, then live and alert counts.
fn render_inspect_bar(frame: &mut Frame, area: Rect, inspect: &Inspect) {
    let lang = inspect.ui.lang;
    let connected = inspect.data.connected;
    let state = if connected {
        Span::styled(
            format!("● {}", i18n::text(lang, Msg::BarConnected)),
            theme::STYLE_OK,
        )
    } else {
        Span::styled(
            format!("○ {}", i18n::text(lang, Msg::BarDisconnected)),
            theme::STYLE_CRIT,
        )
    };
    let mut detail = vec![Span::raw("  "), state];
    if let Some(status) = &inspect.data.status {
        detail.push(Span::styled(
            format!("  {}", status.baseline),
            theme::STYLE_DIM,
        ));
    }
    let mut right = vec![
        Span::styled(
            fmt_count(inspect.data.conns.len() as u64),
            theme::STYLE_TEXT,
        ),
        Span::styled(
            format!(" {}", i18n::text(lang, Msg::BarLiveConns)),
            theme::STYLE_DIM,
        ),
    ];
    if let Some(status) = &inspect.data.status {
        right.push(Span::styled("  ·  ", theme::STYLE_FAINT));
        right.push(Span::styled(fmt_count(status.alerts), theme::STYLE_TEXT));
        right.push(Span::styled(
            format!(" {}", i18n::text(lang, Msg::BarAlerts)),
            theme::STYLE_DIM,
        ));
    }
    if inspect.is_stale() {
        let secs = inspect.last_update.map_or(0, |last| {
            inspect.clock.saturating_duration_since(last).as_secs()
        }) as i64;
        right.push(Span::raw("  "));
        right.push(Span::styled(
            format!("● {}", i18n::stale_for(lang, secs)),
            theme::STYLE_WARN,
        ));
    }
    right.push(Span::raw(" "));
    render_app_bar(frame, area, detail, right);
}

// 3. tab body -----------------------------------------------------------------
// The table card and its detail card share one reorderable group; rects come
// back indexed by card, so a reorder changes nothing here.
fn render_tab_body(frame: &mut Frame, area: Rect, inspect: &mut Inspect) {
    let tab = inspect.tab;
    let cards = inspect.ui.layout.split_cards(
        &mut inspect.ui.hits,
        tab.group(),
        area,
        &[area.height * 3 / 5],
        &[5, 4],
    );
    let (selected_id, count) = match tab {
        InspectTab::Connections => render_connections(frame, cards[0], inspect),
        InspectTab::Processes => render_processes(frame, cards[0], inspect),
        InspectTab::Domains => render_domains(frame, cards[0], inspect),
        InspectTab::Alerts => render_alerts(frame, cards[0], inspect),
        InspectTab::History => render_history(frame, cards[0], inspect),
    };
    let lang = inspect.ui.lang;
    let lines = detail_lines(inspect, tab, selected_id, count, lang);
    let spec = TextCard {
        card: tab.detail_card(),
        title: i18n::text(lang, Msg::TitleDetail).to_string(),
        hint: String::new(),
        lines,
        empty: Some((
            i18n::text(lang, Msg::EmptyNoSelection).to_string(),
            String::new(),
        )),
    };
    render_text_card(frame, &mut inspect.ui, cards[1], spec);
}

// One table card to draw: its id, texts, rows, and empty-state text.
struct TableDraw {
    card: CardId,
    title: String,
    hint: String,
    rows: Vec<TableRow>,
    empty: Option<(String, String)>,
}

// Sorts, keeps the selection on the same row, draws the table card, and
// returns the selected row's id with the row count.
fn draw_table(
    frame: &mut Frame,
    inspect: &mut Inspect,
    area: Rect,
    draw: TableDraw,
) -> (Option<u64>, usize) {
    let TableDraw {
        card,
        title,
        hint,
        rows,
        empty,
    } = draw;
    let rows = sort_rows(&inspect.ui, card, rows);
    let total = rows.len();
    let ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
    inspect.ui.sync_selection(card, ids.clone());
    let view = inspect.ui.views[card as usize];
    let selected_id = ids.get(view.selected.min(total.saturating_sub(1))).copied();
    let table = CardTable {
        card,
        title,
        hint,
        rows,
        total,
        offset: 0,
        windowed: false,
        shown_columns: all_columns_shown(),
        empty,
    };
    render_card_table(frame, &mut inspect.ui, area, table);
    debug_assert!(card_columns(card).len() <= MAX_TABLE_COLUMNS);
    (selected_id, total)
}

fn plain(text: String) -> Vec<Span<'static>> {
    vec![Span::raw(text)]
}

fn proto_name(proto: u8) -> &'static str {
    match proto {
        6 => "TCP",
        17 => "UDP",
        1 => "ICMP",
        _ => "?",
    }
}

// Empty-state text: disconnected, no search match, or nothing yet.
fn empty_for(inspect: &Inspect, searching: bool) -> Option<(String, String)> {
    let lang = inspect.ui.lang;
    if !inspect.data.connected {
        let headline = match &inspect.data.disconnect_detail {
            Some(detail) => i18n::disconnected_detail(lang, detail),
            None => i18n::text(lang, Msg::EmptyDisconnected).to_string(),
        };
        Some((
            headline,
            i18n::text(lang, Msg::EmptyDisconnectedHint).to_string(),
        ))
    } else if searching {
        empty_pair(lang, Msg::EmptyNoMatchRows, Msg::EmptyNoMatchRowsHint)
    } else {
        empty_pair(lang, Msg::EmptyNoRows, Msg::EmptyNoRowsHint)
    }
}

// 4. tables -------------------------------------------------------------------
fn render_connections(
    frame: &mut Frame,
    area: Rect,
    inspect: &mut Inspect,
) -> (Option<u64>, usize) {
    let lang = inspect.ui.lang;
    let rows: Vec<TableRow> = inspect
        .matching_conns()
        .into_iter()
        .map(|conn| {
            let geo = geo_text(conn);
            let destination = if conn.dest_name.is_empty() {
                conn.remote_ip.clone()
            } else {
                conn.dest_name.clone()
            };
            TableRow {
                id: conn.id,
                cells: vec![
                    plain(conn.process.clone()),
                    plain(destination.clone()),
                    plain(geo.clone()),
                    plain(format!("↑{}", fmt_bytes(conn.bytes_up))),
                    plain(format!("↓{}", fmt_bytes(conn.bytes_down))),
                    plain(proto_name(conn.protocol).to_string()),
                ],
                keys: vec![
                    SortKey::Text(conn.process.clone()),
                    SortKey::Text(destination),
                    SortKey::Text(geo),
                    SortKey::Num(conn.bytes_up as i64),
                    SortKey::Num(conn.bytes_down as i64),
                    SortKey::Text(proto_name(conn.protocol).to_string()),
                ],
                style: Style::default(),
            }
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleConnections), rows.len());
    let searching = !inspect.search.is_empty() || inspect.mode == Mode::Search;
    let hint = if searching {
        let cursor = if inspect.mode == Mode::Search {
            "│"
        } else {
            ""
        };
        format!(
            "{}: {}{cursor}",
            i18n::text(lang, Msg::SearchLabelRows),
            inspect.search
        )
    } else {
        String::new()
    };
    let empty = empty_for(inspect, searching);
    draw_table(
        frame,
        inspect,
        area,
        TableDraw {
            card: CardId::ConnTable,
            title,
            hint,
            rows,
            empty,
        },
    )
}

fn geo_text(conn: &ConnItem) -> String {
    match (&conn.country, conn.asn) {
        (Some(country), Some(asn)) => format!("{country} AS{asn}"),
        (Some(country), None) => country.clone(),
        _ => "-".to_string(),
    }
}

fn render_processes(frame: &mut Frame, area: Rect, inspect: &mut Inspect) -> (Option<u64>, usize) {
    let lang = inspect.ui.lang;
    let rows: Vec<TableRow> = inspect
        .data
        .procs
        .iter()
        .map(|item| TableRow {
            id: stable_id(&item.process),
            cells: vec![
                plain(item.process.clone()),
                plain(fmt_count(item.conns as u64)),
                plain(format!("↑{}", fmt_bytes(item.bytes_up))),
                plain(format!("↓{}", fmt_bytes(item.bytes_down))),
            ],
            keys: vec![
                SortKey::Text(item.process.clone()),
                SortKey::Num(item.conns as i64),
                SortKey::Num(item.bytes_up as i64),
                SortKey::Num(item.bytes_down as i64),
            ],
            style: Style::default(),
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleProcesses), rows.len());
    let empty = empty_for(inspect, false);
    draw_table(
        frame,
        inspect,
        area,
        TableDraw {
            card: CardId::ProcTable,
            title,
            hint: String::new(),
            rows,
            empty,
        },
    )
}

fn render_domains(frame: &mut Frame, area: Rect, inspect: &mut Inspect) -> (Option<u64>, usize) {
    let lang = inspect.ui.lang;
    let rows: Vec<TableRow> = inspect
        .data
        .domains
        .iter()
        .map(|item| TableRow {
            id: stable_id(&item.domain),
            cells: vec![
                plain(item.domain.clone()),
                plain(fmt_count(item.conns as u64)),
                plain(format!("↑{}", fmt_bytes(item.bytes_up))),
                plain(format!("↓{}", fmt_bytes(item.bytes_down))),
            ],
            keys: vec![
                SortKey::Text(item.domain.clone()),
                SortKey::Num(item.conns as i64),
                SortKey::Num(item.bytes_up as i64),
                SortKey::Num(item.bytes_down as i64),
            ],
            style: Style::default(),
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleDomains), rows.len());
    let empty = empty_for(inspect, false);
    draw_table(
        frame,
        inspect,
        area,
        TableDraw {
            card: CardId::DomainTable,
            title,
            hint: String::new(),
            rows,
            empty,
        },
    )
}

fn severity_msg(rank: SeverityRank) -> Msg {
    match rank {
        SeverityRank::Info => Msg::SeverityInfo,
        SeverityRank::Notice => Msg::SeverityNotice,
        SeverityRank::Warning => Msg::SeverityWarning,
        SeverityRank::Critical => Msg::SeverityCritical,
    }
}

fn severity_style(rank: SeverityRank) -> Style {
    match rank {
        SeverityRank::Critical => theme::STYLE_CRIT.add_modifier(Modifier::BOLD),
        SeverityRank::Warning => theme::STYLE_WARN,
        SeverityRank::Notice => theme::STYLE_TEXT,
        SeverityRank::Info => theme::STYLE_DIM,
    }
}

fn render_alerts(frame: &mut Frame, area: Rect, inspect: &mut Inspect) -> (Option<u64>, usize) {
    let lang = inspect.ui.lang;
    let rows: Vec<TableRow> = inspect
        .data
        .alerts
        .iter()
        .map(|item| TableRow {
            id: stable_id(&format!("{}|{}", item.key, item.ts_ms)),
            cells: vec![
                vec![Span::styled(
                    i18n::text(lang, severity_msg(item.severity)).to_string(),
                    severity_style(item.severity),
                )],
                plain(item.kind.clone()),
                plain(item.title.clone()),
            ],
            keys: vec![
                SortKey::Num(item.severity as i64),
                SortKey::Text(item.kind.clone()),
                SortKey::Text(item.title.clone()),
            ],
            style: Style::default(),
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleAlerts), rows.len());
    let empty = empty_for(inspect, false);
    draw_table(
        frame,
        inspect,
        area,
        TableDraw {
            card: CardId::AlertTable,
            title,
            hint: String::new(),
            rows,
            empty,
        },
    )
}

fn render_history(frame: &mut Frame, area: Rect, inspect: &mut Inspect) -> (Option<u64>, usize) {
    let lang = inspect.ui.lang;
    let rows: Vec<TableRow> = inspect
        .data
        .history
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let name = if item.name.is_empty() {
                "-".to_string()
            } else {
                item.name.clone()
            };
            TableRow {
                id: position as u64,
                cells: vec![
                    plain(name.clone()),
                    plain(format!("{}:{}", proto_name(item.proto), item.remote_port)),
                    plain(format!("↑{}", fmt_bytes(item.bytes_up))),
                    plain(format!("↓{}", fmt_bytes(item.bytes_down))),
                    plain(fmt_timestamp(item.started_ms)),
                ],
                keys: vec![
                    SortKey::Text(name),
                    SortKey::Text(format!(
                        "{}:{:05}",
                        proto_name(item.proto),
                        item.remote_port
                    )),
                    SortKey::Num(item.bytes_up as i64),
                    SortKey::Num(item.bytes_down as i64),
                    SortKey::Num(item.started_ms),
                ],
                style: Style::default(),
            }
        })
        .collect();
    let title = i18n::title_count(i18n::text(lang, Msg::TitleHistory), rows.len());
    let empty = empty_for(inspect, false);
    draw_table(
        frame,
        inspect,
        area,
        TableDraw {
            card: CardId::HistoryTable,
            title,
            hint: String::new(),
            rows,
            empty,
        },
    )
}

// 5. detail card --------------------------------------------------------------
// Label and value pairs of the selected row; values are data, labels are catalog text.
fn detail_lines(
    inspect: &Inspect,
    tab: InspectTab,
    selected_id: Option<u64>,
    count: usize,
    lang: Lang,
) -> Vec<Line<'static>> {
    let Some(id) = selected_id.filter(|_| count > 0) else {
        return Vec::new();
    };
    let pairs: Vec<(Msg, String)> = match tab {
        InspectTab::Connections => inspect
            .data
            .conns
            .iter()
            .find(|item| item.id == id)
            .map(|item| conn_pairs(item, lang))
            .unwrap_or_default(),
        InspectTab::Processes => inspect
            .data
            .procs
            .iter()
            .find(|item| stable_id(&item.process) == id)
            .map(proc_pairs)
            .unwrap_or_default(),
        InspectTab::Domains => inspect
            .data
            .domains
            .iter()
            .find(|item| stable_id(&item.domain) == id)
            .map(domain_pairs)
            .unwrap_or_default(),
        InspectTab::Alerts => inspect
            .data
            .alerts
            .iter()
            .find(|item| stable_id(&format!("{}|{}", item.key, item.ts_ms)) == id)
            .map(|item| alert_pairs(item, lang))
            .unwrap_or_default(),
        InspectTab::History => inspect
            .data
            .history
            .get(id as usize)
            .map(history_pairs)
            .unwrap_or_default(),
    };
    pairs
        .into_iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    fit_display(i18n::text(lang, label), DETAIL_LABEL_WIDTH),
                    theme::STYLE_DIM,
                ),
                Span::styled(value, theme::STYLE_TEXT),
            ])
        })
        .collect()
}

fn conn_pairs(item: &ConnItem, lang: Lang) -> Vec<(Msg, String)> {
    let direction = if item.outbound {
        Msg::DetailOutbound
    } else {
        Msg::DetailInbound
    };
    let mut pairs = vec![
        (Msg::DetailProcess, item.process.clone()),
        (
            Msg::DetailPid,
            item.pid.map_or("-".to_string(), |pid| pid.to_string()),
        ),
        (Msg::DetailDestination, item.dest_name.clone()),
        (
            Msg::DetailAddress,
            format!("{}:{}", item.remote_ip, item.remote_port),
        ),
        (Msg::DetailLocalPort, item.local_port.to_string()),
        (Msg::DetailProtocol, proto_name(item.protocol).to_string()),
        (Msg::DetailGeo, geo_text(item)),
    ];
    if let Some(org) = &item.as_org {
        pairs.push((Msg::DetailOrg, org.clone()));
    }
    pairs.push((Msg::DetailUp, fmt_bytes(item.bytes_up)));
    pairs.push((Msg::DetailDown, fmt_bytes(item.bytes_down)));
    pairs.push((Msg::DetailFirstSeen, fmt_timestamp(item.first_seen_ms)));
    pairs.push((Msg::DetailLastSeen, fmt_timestamp(item.last_seen_ms)));
    pairs.push((
        Msg::DetailDirection,
        i18n::text(lang, direction).to_string(),
    ));
    pairs
}

fn proc_pairs(item: &ProcItem) -> Vec<(Msg, String)> {
    vec![
        (Msg::DetailProcess, item.process.clone()),
        (
            Msg::DetailPid,
            item.pid.map_or("-".to_string(), |pid| pid.to_string()),
        ),
        (Msg::DetailConns, fmt_count(item.conns as u64)),
        (Msg::DetailUp, fmt_bytes(item.bytes_up)),
        (Msg::DetailDown, fmt_bytes(item.bytes_down)),
    ]
}

fn domain_pairs(item: &DomainItem) -> Vec<(Msg, String)> {
    vec![
        (Msg::DetailDomain, item.domain.clone()),
        (Msg::DetailConns, fmt_count(item.conns as u64)),
        (Msg::DetailUp, fmt_bytes(item.bytes_up)),
        (Msg::DetailDown, fmt_bytes(item.bytes_down)),
    ]
}

fn alert_pairs(item: &AlertItem, lang: Lang) -> Vec<(Msg, String)> {
    vec![
        (
            Msg::DetailSeverity,
            i18n::text(lang, severity_msg(item.severity)).to_string(),
        ),
        (Msg::DetailKind, item.kind.clone()),
        (Msg::DetailTitle, item.title.clone()),
        (Msg::DetailTime, fmt_timestamp(item.ts_ms)),
        (Msg::DetailDedup, item.key.clone()),
    ]
}

fn history_pairs(item: &HistoryItem) -> Vec<(Msg, String)> {
    vec![
        (Msg::DetailName, item.name.clone()),
        (Msg::DetailProtocol, proto_name(item.proto).to_string()),
        (Msg::DetailLocalPort, item.local_port.to_string()),
        (Msg::DetailPort, item.remote_port.to_string()),
        (Msg::DetailUp, fmt_bytes(item.bytes_up)),
        (Msg::DetailDown, fmt_bytes(item.bytes_down)),
        (Msg::DetailStarted, fmt_timestamp(item.started_ms)),
        (Msg::DetailEnded, fmt_timestamp(item.ended_ms)),
    ]
}

// 6. footer -------------------------------------------------------------------
fn render_inspect_footer(frame: &mut Frame, area: Rect, inspect: &mut Inspect) {
    let lang = inspect.ui.lang;
    let left = vec![
        Chip {
            action: UiAction::Refresh,
            key: "r",
            label: i18n::text(lang, Msg::ChipRefresh).to_string(),
            active: false,
        },
        Chip {
            action: UiAction::OpenSearch,
            key: "/",
            label: i18n::text(lang, Msg::ChipSearch).to_string(),
            active: false,
        },
        Chip {
            action: UiAction::ResetLayout,
            key: "l",
            label: i18n::text(lang, Msg::ChipResetLayout).to_string(),
            active: false,
        },
    ];
    let pick = |msgs: &[Msg]| -> Vec<&'static str> {
        msgs.iter().map(|msg| i18n::text(lang, *msg)).collect()
    };
    let focused = inspect.ui.focused_card();
    let context = if inspect.mode == Mode::Search {
        pick(&[Msg::HintApply, Msg::HintEditCancel])
    } else {
        let sortable = !card_columns(focused).is_empty();
        let mut hints = context_hints(&inspect.ui, focused.kind(), sortable);
        hints.extend(pick(&[Msg::HintToggleDetail]));
        hints
    };
    let hints = HintSet {
        context,
        tail: pick(&[Msg::HintNextTab, Msg::HintAllKeys]),
    };
    render_footer(frame, &mut inspect.ui, area, left, &hints);
}
