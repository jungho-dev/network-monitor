//! Render checks for the inspector: every tab in en and ko at the size
//! matrix, exact cells for focus, scrollbar, selection, and chips, and hit
//! areas driven through real mouse events.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{CommandFactory, Parser};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::core::cli::{Cli, Command as CliCommand, MonitorAction};
use crate::inspect::state::{
    AlertItem, Command, ConnItem, DiskBuildWatch, DomainItem, HistoryItem, Inspect, InspectData,
    InspectTab, PREFLIGHT_RETRY, PreflightCache, PreflightOutcome, ProcItem, RestartBlock,
    SeverityRank, StatusItem, daemon_is_stale, daemon_restart_args, preflight_outcome,
    shell_quote_args,
};
use crate::inspect::views::render_frame;
use crate::tui::cards::{CardId, UiAction, scroll_thumb};
use crate::tui::i18n::Lang;
use crate::tui::interaction::ToastLevel;
use crate::tui::layout::SplitGroup;
use crate::tui::sort::SortState;
use crate::tui::test_support::{draw, dump, glyph_budget_violations, key};
use crate::tui::theme;

const SIZES: [(u16, u16); 3] = [(60, 16), (100, 30), (130, 40)];
const LANGS: [Lang; 2] = [Lang::En, Lang::Ko];

fn conn(id: u64, process: &str, dest: &str, up: u64) -> ConnItem {
    ConnItem {
        id,
        process: process.into(),
        pid: Some(100 + id as u32),
        dest_name: dest.into(),
        remote_ip: format!("93.184.216.{}", id % 250),
        remote_port: 443,
        local_port: 50_000 + id as u16,
        protocol: 6,
        country: Some("US".into()),
        asn: Some(15133),
        as_org: Some("Edgecast".into()),
        bytes_up: up,
        bytes_down: up / 2,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_060_000 + id as i64,
        outbound: true,
    }
}

fn sample_data(count: u64) -> InspectData {
    InspectData {
        conns: (0..count)
            .map(|id| {
                conn(
                    id,
                    &format!("proc{}", id % 7),
                    &format!("host{id}.example.com"),
                    1_000 + id * 37,
                )
            })
            .collect(),
        procs: (0..count.min(12))
            .map(|id| ProcItem {
                process: format!("proc{id}"),
                pid: Some(10 + id as u32),
                conns: 1 + id as usize,
                bytes_up: 5_000 * (id + 1),
                bytes_down: 2_000 * (id + 1),
            })
            .collect(),
        domains: (0..count.min(12))
            .map(|id| DomainItem {
                domain: format!("domain{id}.com"),
                conns: 2 + id as usize,
                bytes_up: 9_000 * (id + 1),
                bytes_down: 3_000 * (id + 1),
            })
            .collect(),
        alerts: [
            SeverityRank::Critical,
            SeverityRank::Warning,
            SeverityRank::Notice,
            SeverityRank::Info,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, severity)| AlertItem {
            kind: "New destination".into(),
            severity,
            title: format!("alert title {index}"),
            ts_ms: 1_700_000_000_000 + index as i64,
            key: format!("key-{index}"),
        })
        .collect(),
        history: (0..count.min(30))
            .map(|id| HistoryItem {
                name: format!("hist{id}.example.com"),
                proto: if id % 2 == 0 { 6 } else { 17 },
                local_port: 40_000,
                remote_port: 443,
                bytes_up: 100 * (id + 1),
                bytes_down: 50 * (id + 1),
                started_ms: 1_700_000_000_000 + id as i64 * 1_000,
                ended_ms: 1_700_000_005_000 + id as i64 * 1_000,
            })
            .collect(),
        status: Some(StatusItem {
            baseline: "learning".into(),
            alerts: 4,
            stale_build: false,
        }),
        connected: true,
        disconnect_detail: None,
    }
}

fn sample_inspect(count: u64) -> Inspect {
    let mut inspect = Inspect::new(Some(PathBuf::from("tui-test.toml")));
    inspect.set_data(sample_data(count), Instant::now());
    inspect
}

fn render(inspect: &mut Inspect, width: u16, height: u16) -> Buffer {
    draw(width, height, |frame| render_frame(frame, inspect))
}

fn text_of(inspect: &mut Inspect, width: u16, height: u16) -> String {
    dump(&render(inspect, width, height)).join("\n")
}

fn card_rect(inspect: &Inspect, card: CardId) -> Rect {
    inspect
        .ui
        .hits
        .card_rects
        .iter()
        .find(|(id, _)| *id == card)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("card {card:?} was not drawn"))
}

fn button_rect(inspect: &Inspect, action: UiAction) -> Option<Rect> {
    inspect
        .ui
        .hits
        .button_rects
        .iter()
        .find(|(candidate, _)| *candidate == action)
        .map(|(_, rect)| *rect)
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn click(inspect: &mut Inspect, column: u16, row: u16, now: Instant) -> Command {
    inspect.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        now,
    )
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

// A connected inspector whose daemon runs another build.
fn stale_inspect() -> Inspect {
    let mut inspect = sample_inspect(10);
    if let Some(status) = inspect.data.status.as_mut() {
        status.stale_build = true;
    }
    inspect
}

// 1. size and language matrix -------------------------------------------------
#[test]
fn every_tab_renders_in_both_languages_at_the_size_matrix_and_below_minimum() {
    for lang in LANGS {
        for (width, height) in SIZES.into_iter().chain([(59, 15), (30, 8), (1, 1)]) {
            for tab in InspectTab::ALL {
                let mut inspect = sample_inspect(40);
                inspect.ui.lang = lang;
                inspect.set_tab(tab);
                render(&mut inspect, width, height);
                inspect.mode = crate::inspect::state::Mode::Search;
                inspect.search = "host".into();
                render(&mut inspect, width, height);
                inspect.ui.show_help = true;
                render(&mut inspect, width, height);
            }
            let mut empty = Inspect::new(Some(PathBuf::from("tui-test.toml")));
            empty.ui.lang = lang;
            for tab in InspectTab::ALL {
                empty.set_tab(tab);
                render(&mut empty, width, height);
            }
        }
    }
}

#[test]
fn below_minimum_size_shows_one_centered_warning_in_both_languages() {
    for (lang, expected) in [
        (Lang::En, "Terminal too small (min 60x16, now 59x15)"),
        (Lang::Ko, "터미널 크기 부족 (최소 60x16, 현재 59x15)"),
    ] {
        let mut inspect = sample_inspect(10);
        inspect.ui.lang = lang;
        let rows = dump(&render(&mut inspect, 59, 15));
        assert!(rows[7].contains(expected), "{}", rows[7]);
        for (index, row) in rows.iter().enumerate() {
            if index != 7 {
                assert!(row.trim().is_empty(), "row {index}: {row}");
            }
        }
        assert!(inspect.ui.hits.tab_rects.is_empty());
    }
}

#[test]
fn language_toggle_translates_chrome_and_keeps_daemon_data_untouched() {
    let mut inspect = sample_inspect(10);
    let english = text_of(&mut inspect, 130, 40);
    for needle in [
        "Connections",
        "Live connections (10)",
        "[L] EN",
        "● Connected",
        "learning",
    ] {
        assert!(english.contains(needle), "{needle}");
    }
    inspect.handle_key(key(KeyCode::Char('L')));
    let korean = text_of(&mut inspect, 130, 40);
    for needle in ["연결", "실시간 연결 (10)", "[L] KO", "● 연결됨", "learning"] {
        assert!(korean.contains(needle), "{needle}");
    }
    for data in ["proc3", "host3.example.com", "TCP", "network-monitor"] {
        assert!(english.contains(data) && korean.contains(data), "{data}");
    }
}

// 2. tab bar ------------------------------------------------------------------
#[test]
fn tab_bar_registers_five_hit_areas_and_styles_the_active_tab_with_underline() {
    for lang in LANGS {
        for (width, height) in SIZES {
            let mut inspect = sample_inspect(10);
            inspect.ui.lang = lang;
            let buffer = render(&mut inspect, width, height);
            assert_eq!(
                inspect.ui.hits.tab_rects.len(),
                5,
                "{lang:?} {width}x{height}"
            );
            let (active_index, active) = inspect.ui.hits.tab_rects[0];
            assert_eq!(active_index, 0);
            let tab_row = active.y;
            let name = (active.x..active.right())
                .map(|x| buffer.cell((x, tab_row)).unwrap())
                .find(|cell| cell.modifier.contains(Modifier::UNDERLINED))
                .expect("active tab name is underlined");
            assert_eq!((name.fg, name.bg), (theme::FG_STRONG, theme::BG_PANEL));
            let idle = inspect.ui.hits.tab_rects[1].1;
            let idle_cell = buffer.cell((idle.x, tab_row)).unwrap();
            assert_eq!(idle_cell.bg, theme::BG_BASE);
        }
    }
}

#[test]
fn clicking_a_tab_switches_it_and_history_asks_for_its_rows() {
    let mut inspect = sample_inspect(10);
    render(&mut inspect, 130, 40);
    let alerts = inspect.ui.hits.tab_rects[3].1;
    let now = Instant::now();
    assert_eq!(
        click(&mut inspect, alerts.x + 2, alerts.y, now),
        Command::None
    );
    assert_eq!(inspect.tab, InspectTab::Alerts);
    let text = text_of(&mut inspect, 130, 40);
    assert!(text.contains("Alerts (4)"), "{text}");
    let history = inspect.ui.hits.tab_rects[4].1;
    assert_eq!(
        click(&mut inspect, history.x + 2, history.y, now),
        Command::LoadHistory
    );
    assert!(text_of(&mut inspect, 130, 40).contains("History (10)"));
}

// 3. chips at the minimum size ------------------------------------------------
#[test]
fn language_help_and_quit_chips_survive_the_minimum_size_in_both_languages() {
    for (lang, language, help, quit) in [
        (Lang::En, "[L] EN", "[?] Help", "[q] Quit"),
        (Lang::Ko, "[L] KO", "[?] 도움말", "[q] 종료"),
    ] {
        for tab in InspectTab::ALL {
            let mut inspect = sample_inspect(10);
            inspect.ui.lang = lang;
            inspect.set_tab(tab);
            let buffer = render(&mut inspect, 60, 16);
            let footer = dump(&buffer)[14].clone();
            for chip in [language, help, quit] {
                assert!(
                    footer.contains(chip),
                    "{lang:?} {tab:?}: {chip} in {footer}"
                );
            }
            let language_chip = button_rect(&inspect, UiAction::ToggleLanguage).unwrap();
            let help_chip = button_rect(&inspect, UiAction::ShowHelp).unwrap();
            let quit_chip = button_rect(&inspect, UiAction::Quit).unwrap();
            assert!(language_chip.right() <= help_chip.x && help_chip.right() < quit_chip.x);
            assert_eq!(quit_chip.right(), 59);
            assert!(
                button_rect(&inspect, UiAction::ResetLayout).is_none(),
                "left chips drop first"
            );
        }
    }
}

#[test]
fn wide_footer_lists_refresh_search_reset_language_help_and_quit() {
    let mut inspect = sample_inspect(10);
    render(&mut inspect, 130, 40);
    for action in [
        UiAction::Refresh,
        UiAction::OpenSearch,
        UiAction::ResetLayout,
        UiAction::ToggleLanguage,
        UiAction::ShowHelp,
        UiAction::Quit,
    ] {
        assert!(button_rect(&inspect, action).is_some(), "{action:?}");
    }
    assert_eq!(inspect.ui.hits.button_rects.len(), 6);
}

#[test]
fn footer_chip_clicks_run_the_action_of_their_key() {
    let mut inspect = sample_inspect(10);
    render(&mut inspect, 130, 40);
    let now = Instant::now();
    let refresh = button_rect(&inspect, UiAction::Refresh).unwrap();
    assert_eq!(
        click(&mut inspect, refresh.x + 2, refresh.y, now),
        Command::Refresh
    );
    let language = button_rect(&inspect, UiAction::ToggleLanguage).unwrap();
    assert_eq!(
        click(&mut inspect, language.x + 2, language.y, now),
        Command::SaveState
    );
    assert_eq!(inspect.ui.lang, Lang::Ko);
    render(&mut inspect, 130, 40);
    let search = button_rect(&inspect, UiAction::OpenSearch).unwrap();
    click(&mut inspect, search.x + 2, search.y, now);
    assert_eq!(inspect.mode, crate::inspect::state::Mode::Search);
    let quit = button_rect(&inspect, UiAction::Quit).unwrap();
    click(&mut inspect, quit.x + 2, quit.y, now);
    assert!(inspect.should_quit);
}

// 4. borders, scrollbar, selection --------------------------------------------
#[test]
fn table_card_is_focused_with_a_thick_corner_and_the_detail_card_is_rounded() {
    let mut inspect = sample_inspect(20);
    let buffer = render(&mut inspect, 130, 40);
    let table = card_rect(&inspect, CardId::ConnTable);
    let corner = buffer.cell((table.x, table.y)).unwrap();
    assert_eq!((corner.symbol(), corner.fg), ("┏", theme::BORDER_FOCUS));
    let title = buffer.cell((table.x + 2, table.y)).unwrap();
    assert_eq!(
        (title.fg, title.bg),
        (theme::FG_ON_ACCENT, theme::BORDER_FOCUS)
    );
    let detail = card_rect(&inspect, CardId::ConnDetail);
    let other = buffer.cell((detail.x, detail.y)).unwrap();
    assert_eq!((other.symbol(), other.fg), ("╭", theme::BORDER_DIM));
    inspect.handle_key(key(KeyCode::Enter));
    let buffer = render(&mut inspect, 130, 40);
    assert_eq!(buffer.cell((detail.x, detail.y)).unwrap().symbol(), "┏");
    assert_eq!(buffer.cell((table.x, table.y)).unwrap().symbol(), "╭");
}

#[test]
fn table_scrollbar_uses_gray_thumb_and_track_in_its_own_gutter_for_every_tab() {
    for (width, height) in SIZES {
        let mut inspect = sample_inspect(120);
        inspect.data.history = sample_data(120).history;
        inspect.data.history.extend(sample_data(30).history);
        inspect.data.history.extend(sample_data(30).history);
        for tab in [InspectTab::Connections, InspectTab::History] {
            inspect.set_tab(tab);
            let buffer = render(&mut inspect, width, height);
            let table = card_rect(&inspect, tab.table_card());
            let gutter_x = table.right() - 2;
            let mut thumb = 0;
            let mut track = 0;
            for y in table.y + 2..table.bottom() - 1 {
                let cell = buffer.cell((gutter_x, y)).unwrap();
                match cell.symbol() {
                    "█" => {
                        thumb += 1;
                        assert_eq!(
                            cell.fg,
                            theme::SCROLL_THUMB_FOCUS,
                            "{tab:?} {width}x{height}"
                        );
                    }
                    "░" => {
                        track += 1;
                        assert_eq!(cell.fg, theme::SCROLL_TRACK, "{tab:?} {width}x{height}");
                    }
                    other => panic!("{tab:?} {width}x{height}: gutter glyph {other:?}"),
                }
                assert_ne!(cell.fg, theme::BORDER_FOCUS);
                let border = buffer.cell((table.right() - 1, y)).unwrap();
                assert_eq!((border.symbol(), border.fg), ("┃", theme::BORDER_FOCUS));
            }
            assert!(thumb >= 1 && track >= 1, "{tab:?} {width}x{height}");
        }
    }
}

#[test]
fn the_selected_row_is_a_band_and_moves_with_the_arrow_keys() {
    let mut inspect = sample_inspect(20);
    let buffer = render(&mut inspect, 130, 40);
    let table = card_rect(&inspect, CardId::ConnTable);
    let first = table.y + 2;
    for x in table.x + 1..table.right() - 2 {
        assert_eq!(
            buffer.cell((x, first)).unwrap().bg,
            theme::BG_SELECTED,
            "x={x}"
        );
    }
    assert_eq!(
        buffer.cell((table.x + 3, first)).unwrap().fg,
        theme::FG_STRONG
    );
    inspect.handle_key(key(KeyCode::Down));
    inspect.handle_key(key(KeyCode::Down));
    let buffer = render(&mut inspect, 130, 40);
    assert_ne!(
        buffer.cell((table.x + 3, first)).unwrap().bg,
        theme::BG_SELECTED
    );
    assert_eq!(
        buffer.cell((table.x + 3, first + 2)).unwrap().bg,
        theme::BG_SELECTED
    );
}

#[test]
fn clicking_a_row_selects_it_and_the_detail_card_shows_that_connection() {
    let mut inspect = sample_inspect(20);
    render(&mut inspect, 130, 40);
    let row = inspect
        .ui
        .hits
        .row_hits
        .iter()
        .find(|hit| hit.card == CardId::ConnTable)
        .copied()
        .unwrap();
    click(&mut inspect, row.rect.x + 3, row.rect.y + 3, Instant::now());
    let text = text_of(&mut inspect, 130, 40);
    assert!(
        text.contains("Process") && text.contains("Address"),
        "{text}"
    );
    // Rows are in the daemon's order: row 3 is connection id 3.
    assert!(text.contains("93.184.216.3:443"), "{text}");
    assert!(
        text.contains("US AS15133") && text.contains("Edgecast"),
        "{text}"
    );
    inspect.ui.lang = Lang::Ko;
    let korean = text_of(&mut inspect, 130, 40);
    assert!(
        korean.contains("프로세스") && korean.contains("주소"),
        "{korean}"
    );
    assert!(korean.contains("93.184.216.3:443"));
}

// 5. sorting ------------------------------------------------------------------
fn header_of(inspect: &Inspect, card: CardId, column: usize) -> Rect {
    inspect
        .ui
        .hits
        .header_cells
        .iter()
        .find(|hit| hit.card == card && hit.column == column)
        .map(|hit| hit.rect)
        .unwrap_or_else(|| panic!("no header {card:?}/{column}"))
}

#[test]
fn header_click_cycles_numbers_descending_text_ascending_then_resets_on_every_table() {
    for tab in InspectTab::ALL {
        let mut inspect = sample_inspect(40);
        inspect.data.history = sample_data(40).history;
        inspect.set_tab(tab);
        let card = tab.table_card();
        render(&mut inspect, 130, 40);
        let kinds = crate::tui::columns::card_sort_kinds(card);
        for (column, kind) in kinds.iter().enumerate() {
            let numeric = *kind == crate::tui::sort::SortKind::Number;
            let now = Instant::now();
            inspect.ui.sorts[card as usize] = None;
            for step in 0..3 {
                render(&mut inspect, 130, 40);
                let header = header_of(&inspect, card, column);
                click(&mut inspect, header.x, header.y, now);
                let state = inspect.ui.sorts[card as usize];
                match step {
                    0 => assert_eq!(
                        state,
                        Some(SortState {
                            column,
                            descending: numeric
                        }),
                        "{tab:?} column {column}"
                    ),
                    1 => assert_eq!(
                        state.map(|s| s.descending),
                        Some(!numeric),
                        "{tab:?} {column}"
                    ),
                    _ => assert_eq!(state, None, "{tab:?} column {column} third click"),
                }
            }
        }
    }
}

#[test]
fn sorted_rows_follow_typed_keys_and_title_and_header_show_the_arrow_in_both_languages() {
    for (lang, word) in [(Lang::En, "Sort Up"), (Lang::Ko, "정렬 송신")] {
        let mut inspect = sample_inspect(30);
        inspect.ui.lang = lang;
        render(&mut inspect, 130, 40);
        let header = header_of(&inspect, CardId::ConnTable, 3);
        click(&mut inspect, header.x, header.y, Instant::now());
        let buffer = render(&mut inspect, 130, 40);
        let rows = dump(&buffer);
        assert!(rows.join("\n").contains(&format!("{word}↓")), "{lang:?}");
        let table = card_rect(&inspect, CardId::ConnTable);
        // Largest upload first: the last generated connection sends the most.
        assert!(
            rows[usize::from(table.y + 2)].contains("host29.example.com"),
            "{}",
            rows[usize::from(table.y + 2)]
        );
        assert!(
            rows[usize::from(header.y)].contains("↓") || rows[usize::from(header.y)].contains("Up")
        );
    }
}

#[test]
fn selection_follows_the_same_connection_after_a_sort() {
    let mut inspect = sample_inspect(30);
    render(&mut inspect, 130, 40);
    inspect.handle_key(key(KeyCode::Down));
    inspect.handle_key(key(KeyCode::Down));
    render(&mut inspect, 130, 40);
    let chosen = inspect.ui.selected_keys[CardId::ConnTable as usize];
    assert_eq!(chosen, Some(2));
    // Process text, descending: connection 2 (proc2) moves far from row 2.
    inspect.handle_key(key(KeyCode::Char('S')));
    inspect.handle_key(key(KeyCode::Char('S')));
    let buffer = render(&mut inspect, 130, 40);
    assert_eq!(inspect.ui.selected_keys[CardId::ConnTable as usize], chosen);
    assert_ne!(inspect.ui.views[CardId::ConnTable as usize].selected, 2);
    let rows = dump(&buffer);
    let table = card_rect(&inspect, CardId::ConnTable);
    let banded: Vec<&String> = (table.y + 2..table.bottom() - 1)
        .filter(|y| buffer.cell((table.x + 3, *y)).unwrap().bg == theme::BG_SELECTED)
        .map(|y| &rows[usize::from(y)])
        .collect();
    assert_eq!(banded.len(), 1, "exactly one selected band");
    assert!(banded[0].contains("host2.example.com"), "{}", banded[0]);
}

#[test]
fn s_and_upper_s_use_the_same_cycle_as_the_header_click_and_end_in_capture_order() {
    let mut inspect = sample_inspect(10);
    for _ in 0..6 {
        inspect.handle_key(key(KeyCode::Char('s')));
    }
    assert_eq!(
        inspect.ui.sorts[CardId::ConnTable as usize].map(|s| s.column),
        Some(5)
    );
    inspect.handle_key(key(KeyCode::Char('s')));
    assert_eq!(inspect.ui.sorts[CardId::ConnTable as usize], None);
    inspect.handle_key(key(KeyCode::Char('S')));
    inspect.handle_key(key(KeyCode::Char('S')));
    inspect.handle_key(key(KeyCode::Char('S')));
    assert_eq!(inspect.ui.sorts[CardId::ConnTable as usize], None);
}

// 6. columns ------------------------------------------------------------------
#[test]
fn dragging_a_column_edge_widens_it_saves_on_release_and_a_second_press_resets() {
    let mut inspect = sample_inspect(30);
    render(&mut inspect, 130, 40);
    let edge = inspect
        .ui
        .hits
        .column_hits
        .iter()
        .find(|hit| hit.card == CardId::ConnTable && hit.column == 0)
        .copied()
        .expect("process column edge");
    let now = Instant::now();
    click(&mut inspect, edge.grab.x, edge.grab.y, now);
    inspect.handle_mouse(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            edge.grab.x + 5,
            edge.grab.y,
        ),
        now,
    );
    assert_eq!(
        inspect.ui.column_widths[CardId::ConnTable as usize][0],
        Some(edge.width + 5)
    );
    let outcome = inspect.handle_mouse(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            edge.grab.x + 5,
            edge.grab.y,
        ),
        now,
    );
    assert_eq!(outcome, Command::SaveState);
    render(&mut inspect, 130, 40);
    let wider = inspect
        .ui
        .hits
        .column_hits
        .iter()
        .find(|hit| hit.column == 0)
        .copied()
        .unwrap();
    click(&mut inspect, wider.grab.x, wider.grab.y, now);
    inspect.handle_mouse(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            wider.grab.x,
            wider.grab.y,
        ),
        now,
    );
    let again = click(
        &mut inspect,
        wider.grab.x,
        wider.grab.y,
        now + Duration::from_millis(100),
    );
    assert_eq!(again, Command::SaveState);
    assert_eq!(
        inspect.ui.column_widths[CardId::ConnTable as usize][0],
        None
    );
}

#[test]
fn natural_widths_show_destinations_whole_when_the_card_is_wide_enough() {
    let mut inspect = sample_inspect(10);
    let rows = dump(&render(&mut inspect, 130, 40));
    let row = rows
        .iter()
        .find(|row| row.contains("host3.example.com"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(!row.contains('…'), "{row}");
    assert!(
        row.contains("US AS15133") && row.contains("↑") && row.contains("TCP"),
        "{row}"
    );
}

// 7. arrangement --------------------------------------------------------------
#[test]
fn shift_arrows_and_a_title_hold_reorder_the_table_and_detail_cards() {
    let mut inspect = sample_inspect(30);
    render(&mut inspect, 130, 40);
    let table = card_rect(&inspect, CardId::ConnTable);
    let detail = card_rect(&inspect, CardId::ConnDetail);
    assert!(table.y < detail.y);
    let shifted = crate::tui::test_support::shift_key(KeyCode::Down);
    assert_eq!(inspect.handle_key(shifted), Command::SaveState);
    render(&mut inspect, 130, 40);
    assert_eq!(card_rect(&inspect, CardId::ConnTable), detail);
    assert_eq!(card_rect(&inspect, CardId::ConnDetail), table);
    inspect.handle_key(key(KeyCode::Char('l')));
    render(&mut inspect, 130, 40);
    assert_eq!(
        card_rect(&inspect, CardId::ConnTable),
        table,
        "l restores the order"
    );

    let title = inspect
        .ui
        .hits
        .title_hits
        .iter()
        .find(|hit| hit.card == CardId::ConnTable)
        .copied()
        .unwrap();
    let start = Instant::now();
    click(&mut inspect, title.rect.x + 1, title.rect.y, start);
    assert!(inspect.on_tick(start + Duration::from_millis(400)));
    assert!(inspect.ui.card_move.is_some());
    inspect.handle_mouse(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            detail.x + 5,
            detail.y + 2,
        ),
        start + Duration::from_millis(450),
    );
    let outcome = inspect.handle_mouse(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            detail.x + 5,
            detail.y + 2,
        ),
        start + Duration::from_millis(500),
    );
    assert_eq!(outcome, Command::SaveState);
    assert_eq!(
        inspect.ui.layout.order_of(SplitGroup::InspectConn)[..2],
        [1, 0]
    );
}

#[test]
fn dragging_the_boundary_between_table_and_detail_stores_a_ratio_and_saves() {
    let mut inspect = sample_inspect(30);
    render(&mut inspect, 130, 40);
    let hit = inspect.ui.hits.split_hits[0];
    let before = card_rect(&inspect, CardId::ConnTable).height;
    let now = Instant::now();
    click(&mut inspect, hit.grab.x + 4, hit.grab.y, now);
    inspect.handle_mouse(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            hit.grab.x + 4,
            hit.grab.y + 3,
        ),
        now,
    );
    let outcome = inspect.handle_mouse(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            hit.grab.x + 4,
            hit.grab.y + 3,
        ),
        now,
    );
    assert_eq!(outcome, Command::SaveState);
    render(&mut inspect, 130, 40);
    assert_eq!(card_rect(&inspect, CardId::ConnTable).height, before + 3);
    assert!(
        inspect
            .ui
            .layout
            .custom_ratios(SplitGroup::InspectConn)
            .is_some()
    );
}

// 8. states -------------------------------------------------------------------
#[test]
fn alerts_show_the_severity_word_and_color_and_sort_by_rank_not_label() {
    let mut inspect = sample_inspect(10);
    inspect.set_tab(InspectTab::Alerts);
    // Move the selection off the first row so its own colors show.
    render(&mut inspect, 130, 40);
    inspect.handle_key(key(KeyCode::Down));
    let buffer = render(&mut inspect, 130, 40);
    let text = dump(&buffer).join("\n");
    for word in ["Critical", "Warning", "Notice", "Info"] {
        assert!(text.contains(word), "{word}");
    }
    let rows = dump(&buffer);
    let critical_row = rows
        .iter()
        .position(|row| row.contains("Critical"))
        .unwrap();
    let column = rows[critical_row].find("Critical").unwrap();
    let x = rows[critical_row][..column].chars().count() as u16;
    let cell = buffer.cell((x, critical_row as u16)).unwrap();
    assert_eq!(cell.fg, theme::COLOR_CRIT);
    assert!(cell.modifier.contains(Modifier::BOLD));
    // Sorting the Severity column ranks Critical first, in either language.
    for lang in LANGS {
        inspect.ui.lang = lang;
        inspect.ui.sorts[CardId::AlertTable as usize] = None;
        render(&mut inspect, 130, 40);
        let header = header_of(&inspect, CardId::AlertTable, 0);
        click(&mut inspect, header.x, header.y, Instant::now());
        let rows = dump(&render(&mut inspect, 130, 40));
        let table = card_rect(&inspect, CardId::AlertTable);
        let first = &rows[usize::from(table.y + 2)];
        assert!(first.contains("alert title 0"), "{lang:?}: {first}");
    }
}

#[test]
fn disconnected_daemon_marks_the_bar_and_explains_empty_tables() {
    let mut inspect = Inspect::new(Some(PathBuf::from("tui-test.toml")));
    inspect.data.connected = false;
    inspect.data.disconnect_detail = Some("daemon closed".into());
    let buffer = render(&mut inspect, 130, 40);
    let bar = dump(&buffer)[0].clone();
    assert!(bar.contains("○ Disconnected"), "{bar}");
    let rows = dump(&buffer).join("\n");
    assert!(
        rows.contains("Daemon disconnected: daemon closed"),
        "{rows}"
    );
    assert!(rows.contains("Press q to quit and run inspect again"));
    let x = bar
        .find("○ Disconnected")
        .map(|byte| bar[..byte].chars().count())
        .unwrap() as u16;
    assert_eq!(buffer.cell((x, 0)).unwrap().fg, theme::COLOR_CRIT);
    inspect.ui.lang = Lang::Ko;
    let korean = text_of(&mut inspect, 130, 40);
    assert!(korean.contains("데몬 연결 끊김: daemon closed"), "{korean}");
}

#[test]
fn empty_states_and_the_search_prompt_follow_the_active_language() {
    let mut inspect = Inspect::new(Some(PathBuf::from("tui-test.toml")));
    let text = text_of(&mut inspect, 130, 40);
    assert!(
        text.contains("Nothing to show yet") && text.contains("Press r to refresh"),
        "{text}"
    );
    inspect.ui.lang = Lang::Ko;
    assert!(text_of(&mut inspect, 130, 40).contains("표시할 항목 없음"));

    let mut searching = sample_inspect(10);
    searching.handle_key(key(KeyCode::Char('/')));
    for letter in "zzz".chars() {
        searching.handle_key(key(KeyCode::Char(letter)));
    }
    let text = text_of(&mut searching, 130, 40);
    assert!(text.contains("Search: zzz│"), "{text}");
    assert!(text.contains("No connection matches the search"), "{text}");
    searching.ui.lang = Lang::Ko;
    let korean = text_of(&mut searching, 130, 40);
    assert!(
        korean.contains("검색: zzz│") && korean.contains("검색에 맞는 연결 없음"),
        "{korean}"
    );
}

#[test]
fn app_bar_shows_counts_the_daemon_baseline_and_a_stale_marker() {
    let mut inspect = sample_inspect(10);
    let bar = dump(&render(&mut inspect, 130, 40))[0].clone();
    for needle in [
        "network-monitor",
        "● Connected",
        "learning",
        "10 live",
        "4 alerts",
    ] {
        assert!(bar.contains(needle), "{needle} in {bar}");
    }
    let start = Instant::now();
    inspect.set_data(sample_data(10), start);
    inspect.on_tick(start + Duration::from_secs(7));
    let stale = dump(&render(&mut inspect, 130, 40))[0].clone();
    assert!(stale.contains("Stale 7s"), "{stale}");
    inspect.ui.lang = Lang::Ko;
    assert!(dump(&render(&mut inspect, 130, 40))[0].contains("갱신 지연 7초"));
}

// 9. help overlay and hit areas -----------------------------------------------
#[test]
fn help_overlay_has_a_double_border_a_dim_backdrop_and_both_sections() {
    for (lang, keyboard, mouse_title, close) in [
        (Lang::En, "Keyboard", "Mouse", "Any key or click: close"),
        (Lang::Ko, "키보드", "마우스", "아무 키·클릭: 닫기"),
    ] {
        let mut inspect = sample_inspect(10);
        inspect.ui.lang = lang;
        inspect.ui.show_help = true;
        let buffer = render(&mut inspect, 100, 40);
        assert_eq!(buffer.cell((1, 0)).unwrap().fg, theme::BORDER_DIM);
        let mut corner = None;
        for y in 0..40 {
            for x in 0..100 {
                if buffer.cell((x, y)).unwrap().symbol() == "╔" {
                    corner = Some(buffer.cell((x, y)).unwrap().fg);
                }
            }
        }
        assert_eq!(corner, Some(theme::FG_ACCENT));
        let text = dump(&buffer).join("\n");
        for needle in [keyboard, mouse_title, close, "Shift+Arrows"] {
            assert!(text.contains(needle), "{lang:?}: {needle}");
        }
    }
    let mut short = sample_inspect(10);
    short.ui.show_help = true;
    assert!(text_of(&mut short, 60, 16).contains("… (enlarge window to show all)"));
}

#[test]
fn every_clickable_element_of_a_tab_registers_a_hit_area() {
    for tab in InspectTab::ALL {
        let mut inspect = sample_inspect(100);
        inspect.data.history = sample_data(30).history;
        inspect.set_tab(tab);
        render(&mut inspect, 130, 40);
        let hits = &inspect.ui.hits;
        let card = tab.table_card();
        let columns = crate::tui::columns::card_columns(card).len();
        assert_eq!(
            hits.header_cells
                .iter()
                .filter(|hit| hit.card == card)
                .count(),
            columns,
            "{tab:?}"
        );
        assert!(hits.row_hits.iter().any(|hit| hit.card == card), "{tab:?}");
        assert_eq!(hits.tab_rects.len(), 5);
        assert_eq!(hits.split_hits.len(), 1, "{tab:?}: one boundary");
        assert_eq!(hits.slot_groups.len(), 1);
        assert_eq!(hits.slot_groups[0].group, tab.group());
        let titled: Vec<CardId> = hits.title_hits.iter().map(|hit| hit.card).collect();
        assert!(
            titled.contains(&card) && titled.contains(&tab.detail_card()),
            "{tab:?}"
        );
        for header in hits.header_cells.iter().filter(|hit| hit.card == card) {
            let rect = card_rect(&inspect, card);
            assert!(
                header.rect.right() <= rect.right() - 2,
                "{tab:?} {header:?}"
            );
        }
    }
}

#[test]
fn track_click_pages_and_wheel_scrolls_the_card_under_the_pointer() {
    let mut inspect = sample_inspect(200);
    render(&mut inspect, 130, 40);
    let hit = *inspect
        .ui
        .hits
        .scroll_hits
        .iter()
        .find(|hit| hit.card == CardId::ConnTable)
        .expect("scroll hit");
    let thumb = scroll_thumb(hit.total, hit.visible, hit.scroll, hit.track.height).unwrap();
    assert_eq!(thumb.start, 0);
    let now = Instant::now();
    click(&mut inspect, hit.track.x, hit.track.bottom() - 1, now);
    assert_eq!(
        inspect.ui.views[CardId::ConnTable as usize].scroll,
        hit.visible
    );
    render(&mut inspect, 130, 40);
    let table = card_rect(&inspect, CardId::ConnTable);
    inspect.handle_mouse(
        mouse(MouseEventKind::ScrollDown, table.x + 5, table.y + 4),
        now,
    );
    assert_eq!(
        inspect.ui.views[CardId::ConnTable as usize].scroll,
        hit.visible + 3
    );
    let detail = card_rect(&inspect, CardId::ConnDetail);
    inspect.handle_mouse(
        mouse(MouseEventKind::ScrollDown, detail.x + 5, detail.y + 2),
        now,
    );
    assert_eq!(
        inspect.ui.views[CardId::ConnTable as usize].scroll,
        hit.visible + 3,
        "the wheel over the detail card leaves the table alone"
    );
}

#[test]
fn every_inspector_cell_stays_inside_the_glyph_budget_in_both_languages() {
    for lang in LANGS {
        for (width, height) in SIZES {
            for tab in InspectTab::ALL {
                let mut inspect = sample_inspect(40);
                inspect.ui.lang = lang;
                inspect.set_tab(tab);
                let found = glyph_budget_violations(&render(&mut inspect, width, height));
                assert!(
                    found.is_empty(),
                    "{lang:?} {width}x{height} {tab:?}: {found:?}"
                );
            }
            let mut inspect = sample_inspect(40);
            inspect.ui.lang = lang;
            inspect.ui.show_help = true;
            let found = glyph_budget_violations(&render(&mut inspect, width, height));
            assert!(
                found.is_empty(),
                "{lang:?} {width}x{height} help: {found:?}"
            );
        }
    }
}

// 10. force restart -----------------------------------------------------------
#[test]
fn daemon_is_stale_only_for_two_known_different_builds_of_a_monitor_run_daemon() {
    let run = args(&["monitor", "run", "-i", "eth0"]);
    assert!(!daemon_is_stale("5-100", "5-100", &run), "same build");
    assert!(!daemon_is_stale("", "5-100", &run), "older daemon");
    assert!(!daemon_is_stale("5-100", "", &run), "disk build unknown");
    assert!(
        !daemon_is_stale("5-100", "6-200", &args(&["inspect"])),
        "not monitor run"
    );
    assert!(!daemon_is_stale("5-100", "6-200", &[]), "no arguments");
    assert!(daemon_is_stale("5-100", "6-200", &run));
}

#[test]
fn daemon_restart_args_add_daemonize_and_state_dir_and_keep_the_rest_in_order() {
    assert_eq!(
        daemon_restart_args(&args(&["monitor", "run", "-i", "eth0"]), "/s"),
        Ok(args(&[
            "monitor",
            "run",
            "-i",
            "eth0",
            "--state-dir",
            "/s",
            "--daemonize"
        ]))
    );
    for kept in [
        args(&["monitor", "run", "--state-dir", "/kept", "--daemonize"]),
        args(&["monitor", "run", "--state-dir=/kept", "--daemonize"]),
        args(&[
            "monitor",
            "run",
            "-f",
            "tcp port 443",
            "--geoip-country-db",
            "/geo/country.mmdb",
            "--geoip-asn-db=/geo/asn.mmdb",
            "--state-dir=/kept",
            "--daemonize",
        ]),
    ] {
        assert_eq!(daemon_restart_args(&kept, "/other"), Ok(kept.clone()));
    }
}

#[test]
fn daemon_restart_args_refuse_a_command_other_than_monitor_run() {
    assert_eq!(
        daemon_restart_args(&args(&["monitor", "status"]), "/s"),
        Err(RestartBlock::NotMonitorRun)
    );
    assert_eq!(
        daemon_restart_args(&args(&["run"]), "/s"),
        Err(RestartBlock::NotMonitorRun)
    );
}

#[test]
fn daemon_restart_args_refuse_a_relative_or_missing_path() {
    let run = args(&["monitor", "run"]);
    for state_dir in ["", "s"] {
        assert_eq!(
            daemon_restart_args(&run, state_dir),
            Err(RestartBlock::RelativePath),
            "state dir {state_dir:?}"
        );
    }
    for relative in [
        args(&["monitor", "run", "--state-dir", "rel"]),
        args(&["monitor", "run", "--state-dir=rel"]),
        args(&["monitor", "run", "--geoip-country-db", "./c"]),
        args(&["monitor", "run", "--geoip-asn-db=a"]),
        args(&["monitor", "run", "-i", "eth0", "--geoip-asn-db"]),
    ] {
        assert_eq!(
            daemon_restart_args(&relative, "/s"),
            Err(RestartBlock::RelativePath),
            "{relative:?}"
        );
    }
}

#[test]
fn disk_build_watch_settles_a_build_only_after_two_equal_reads() {
    let mut watch = DiskBuildWatch::default();
    assert!(!watch.observe("A"), "one read");
    assert!(watch.observe("A"), "two reads");
    assert!(!watch.observe("A"), "already settled");
    assert!(!watch.observe("B"), "one read of B");
    assert_eq!(watch.settled(), "A");
    assert!(watch.observe("B"), "two reads of B");
    assert_eq!(watch.settled(), "B");
}

#[test]
fn disk_build_watch_settles_an_unreadable_exe_only_after_a_known_build() {
    let mut settled = DiskBuildWatch::default();
    settled.observe("A");
    settled.observe("A");
    assert!(!settled.observe(""));
    assert!(settled.observe(""));
    assert_eq!(settled.settled(), "");

    let mut fresh = DiskBuildWatch::default();
    assert!(!fresh.observe(""));
    assert!(!fresh.observe(""));
}

#[test]
fn bang_is_ignored_without_a_mismatch_and_restarts_once_the_builds_differ() {
    let mut inspect = sample_inspect(10);
    assert_eq!(inspect.handle_key(key(KeyCode::Char('!'))), Command::None);
    assert!(!inspect.should_quit && !inspect.restart_requested);

    let mut stale = stale_inspect();
    assert_eq!(stale.handle_key(key(KeyCode::Char('!'))), Command::None);
    assert!(stale.should_quit && stale.restart_requested);
}

#[test]
fn bang_without_capture_privilege_only_shows_an_error_toast() {
    let mut inspect = stale_inspect();
    inspect.restart_block = Some(RestartBlock::Privilege);
    inspect.handle_key(key(KeyCode::Char('!')));
    assert!(!inspect.should_quit && !inspect.restart_requested);
    let toast = inspect.ui.toast.as_ref().expect("error toast");
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.text.contains("sudo or setcap"), "{}", toast.text);
}

#[test]
fn bang_without_state_folder_access_only_shows_an_error_toast() {
    let mut inspect = stale_inspect();
    inspect.restart_block = Some(RestartBlock::StateAccess);
    inspect.handle_key(key(KeyCode::Char('!')));
    assert!(!inspect.should_quit && !inspect.restart_requested);
    let toast = inspect.ui.toast.as_ref().expect("error toast");
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.text.contains("daemon's user"), "{}", toast.text);
}

#[test]
fn bang_in_the_search_box_is_typed_as_a_letter() {
    let mut inspect = stale_inspect();
    inspect.handle_key(key(KeyCode::Char('/')));
    inspect.handle_key(key(KeyCode::Char('!')));
    assert_eq!(inspect.search, "!");
    assert!(!inspect.should_quit && !inspect.restart_requested);
}

#[test]
fn a_build_mismatch_puts_the_force_restart_chip_first_and_marks_the_app_bar() {
    let mut inspect = sample_inspect(10);
    let bar = dump(&render(&mut inspect, 130, 40))[0].clone();
    assert!(!bar.contains("Build mismatch"), "{bar}");
    assert!(button_rect(&inspect, UiAction::ForceRestart).is_none());

    let mut stale = stale_inspect();
    let rows = dump(&render(&mut stale, 130, 40));
    assert!(rows[0].contains("● Build mismatch"), "{}", rows[0]);
    let chip = button_rect(&stale, UiAction::ForceRestart).expect("force restart chip");
    for (action, rect) in &stale.ui.hits.button_rects {
        if *action != UiAction::ForceRestart {
            assert!(chip.right() <= rect.x, "{action:?} sits before the chip");
        }
    }
    assert!(rows[usize::from(chip.y)].contains("Force restart"));
    stale.ui.lang = Lang::Ko;
    let korean = dump(&render(&mut stale, 130, 40));
    assert!(korean[0].contains("빌드 불일치"), "{}", korean[0]);

    stale.data.connected = false;
    render(&mut stale, 130, 40);
    assert!(button_rect(&stale, UiAction::ForceRestart).is_none());
}

#[test]
fn preflight_cache_answers_only_for_the_stored_known_build() {
    let now = Instant::now();
    let mut cache = PreflightCache::default();
    assert_eq!(cache.lookup("5-100", now), None, "empty cache");
    cache.store("5-100", PreflightOutcome::Passed, now);
    assert_eq!(cache.lookup("5-100", now), Some(PreflightOutcome::Passed));
    assert_eq!(cache.lookup("6-200", now), None, "another build");
    cache.store("6-200", PreflightOutcome::NoPrivilege, now);
    assert_eq!(
        cache.lookup("6-200", now),
        Some(PreflightOutcome::NoPrivilege)
    );
    assert_eq!(cache.lookup("", now), None, "unknown build");

    let mut blank = PreflightCache::default();
    blank.store("", PreflightOutcome::Passed, now);
    assert_eq!(
        blank.lookup("", now),
        None,
        "an empty build is never cached"
    );
}

#[test]
fn preflight_cache_keeps_a_pass_and_expires_a_failure_after_the_retry_interval() {
    let now = Instant::now();
    let later = now + PREFLIGHT_RETRY;
    let mut cache = PreflightCache::default();
    cache.store("5-100", PreflightOutcome::Passed, now);
    assert_eq!(cache.lookup("5-100", later), Some(PreflightOutcome::Passed));

    cache.store("5-100", PreflightOutcome::NoPrivilege, now);
    assert_eq!(
        cache.lookup("5-100", now + Duration::from_secs(9)),
        Some(PreflightOutcome::NoPrivilege)
    );
    assert_eq!(cache.lookup("5-100", later), None, "expired failure");

    cache.store("5-100", PreflightOutcome::Unknown, now);
    assert_eq!(cache.lookup("5-100", later), None, "expired unknown");
}

#[test]
fn preflight_outcome_reads_only_exit_codes_zero_and_one_as_answers() {
    assert_eq!(preflight_outcome(Some(0)), PreflightOutcome::Passed);
    assert_eq!(preflight_outcome(Some(1)), PreflightOutcome::NoPrivilege);
    assert_eq!(
        preflight_outcome(Some(2)),
        PreflightOutcome::Unknown,
        "old build"
    );
    assert_eq!(
        preflight_outcome(None),
        PreflightOutcome::Unknown,
        "no exit code"
    );
}

#[test]
fn unknown_preflight_and_exe_name_blocks_map_to_their_toasts() {
    let unknown = crate::tui::i18n::text(Lang::En, RestartBlock::PreflightUnknown.toast());
    assert_eq!(
        unknown,
        "New exe could not check its capture privilege: restart the daemon by hand"
    );
    let name = crate::tui::i18n::text(Lang::Ko, RestartBlock::ExeName.toast());
    assert_eq!(name, "데몬 exe 이름이 inspect 와 다름: 수동으로 재시작");
}

#[test]
fn shell_quote_args_quotes_only_words_the_shell_would_split() {
    assert_eq!(
        shell_quote_args("/usr/bin/nm", &args(&["monitor", "run", "-i", "eth0"])),
        "/usr/bin/nm monitor run -i eth0"
    );
    assert_eq!(
        shell_quote_args("/nm", &args(&["-f", "tcp port 443"])),
        "/nm -f 'tcp port 443'"
    );
    assert_eq!(shell_quote_args("/nm", &args(&["it's"])), r"/nm 'it'\''s'");
    assert_eq!(
        shell_quote_args("/nm", &args(&["--state-dir", ""])),
        "/nm --state-dir ''"
    );
    assert_eq!(
        shell_quote_args("/opt/my tools/nm", &args(&["monitor"])),
        "'/opt/my tools/nm' monitor"
    );
}

#[test]
fn bang_when_the_new_build_lacks_capture_privilege_asks_for_setcap_again() {
    let mut inspect = stale_inspect();
    inspect.restart_block = Some(RestartBlock::NewExePrivilege);
    inspect.handle_key(key(KeyCode::Char('!')));
    assert!(!inspect.should_quit && !inspect.restart_requested);
    let toast = inspect.ui.toast.as_ref().expect("error toast");
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.text.contains("setcap"), "{}", toast.text);
}

#[test]
fn monitor_preflight_parses_and_stays_out_of_the_monitor_help() {
    let cli = Cli::try_parse_from(["network-monitor", "monitor", "preflight"])
        .expect("monitor preflight parses");
    assert!(matches!(
        cli.command,
        Some(CliCommand::Monitor {
            action: MonitorAction::Preflight
        })
    ));
    let mut command = Cli::command();
    let monitor = command
        .find_subcommand_mut("monitor")
        .expect("monitor subcommand");
    let help = monitor.render_help().to_string();
    assert!(
        help.contains("status") && !help.contains("preflight"),
        "{help}"
    );
}
