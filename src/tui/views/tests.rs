//! Render checks for the capture screen: every view in en and ko at the size
//! matrix, exact cells for focus, scrollbar, selection, toast, and chips, and
//! hit areas driven through real mouse events.

use super::*;
use crate::tui::cards::{CardId, UiAction};
use crate::tui::i18n::Lang;
use crate::tui::interaction::{Toast, ToastLevel};
use crate::tui::layout::IDENTITY_ORDER;
use crate::tui::level::OutputLevel;
use crate::tui::sort::{SortChange, SortState};
use crate::tui::state::{CaptureTab, Command};
use crate::tui::test_support::{
    capture_app, draw, dump, glyph_budget_violations, key, populate, shift_key, tcp_packet,
    threat_packet,
};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use network_monitor_core::analysis::ThreatKind;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::time::{Duration, Instant};

const SIZES: [(u16, u16); 3] = [(60, 16), (100, 30), (130, 40)];
const LANGS: [Lang; 2] = [Lang::En, Lang::Ko];

fn sample_app(count: u64) -> App {
    let mut app = capture_app();
    populate(&mut app, count);
    app.selected = (count as usize).saturating_sub(1);
    app
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    draw(width, height, |frame| render_frame(frame, app))
}

fn text_of(app: &mut App, width: u16, height: u16) -> String {
    dump(&render(app, width, height)).join("\n")
}

fn card_rect(app: &App, card: CardId) -> Rect {
    app.ui
        .hits
        .card_rects
        .iter()
        .find(|(id, _)| *id == card)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("card {card:?} was not drawn"))
}

fn button_rect(app: &App, action: UiAction) -> Option<Rect> {
    app.ui
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

fn click(app: &mut App, column: u16, row: u16, now: Instant) -> Command {
    app.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        now,
    )
}

fn release(app: &mut App, column: u16, row: u16, now: Instant) -> Command {
    app.handle_mouse(
        mouse(MouseEventKind::Up(MouseButton::Left), column, row),
        now,
    )
}

fn drag_to(app: &mut App, column: u16, row: u16, now: Instant) -> Command {
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), column, row),
        now,
    )
}

// 1. size and language matrix -------------------------------------------------
#[test]
fn every_view_renders_in_both_languages_at_the_size_matrix_and_below_minimum() {
    for lang in LANGS {
        for (width, height) in SIZES.into_iter().chain([(59, 15), (40, 10), (1, 1)]) {
            let mut app = sample_app(60);
            app.ui.lang = lang;
            render(&mut app, width, height);
            // Every tab, the pane toggles, and the input modes must not panic either.
            for tab in CaptureTab::ALL {
                app.switch_tab(tab);
                app.refresh_stats(Instant::now());
                render(&mut app, width, height);
            }
            app.switch_tab(CaptureTab::Packets);
            for (detail, hex) in [(true, false), (false, true), (false, false), (true, true)] {
                app.show_detail = detail;
                app.show_hex = hex;
                render(&mut app, width, height);
            }
            for mode in [
                InputMode::FilterInput,
                InputMode::Search,
                InputMode::ColumnConfig,
            ] {
                app.mode = mode;
                render(&mut app, width, height);
            }
            app.ui.show_help = true;
            render(&mut app, width, height);
        }
        let mut empty = capture_app();
        empty.ui.lang = lang;
        for (width, height) in SIZES {
            render(&mut empty, width, height);
        }
    }
}

#[test]
fn below_minimum_size_shows_only_one_centered_warning_in_both_languages() {
    for (lang, expected) in [
        (Lang::En, "Terminal too small (min 60x16, now 59x15)"),
        (Lang::Ko, "터미널 크기 부족 (최소 60x16, 현재 59x15)"),
    ] {
        let mut app = sample_app(10);
        app.ui.lang = lang;
        let rows = dump(&render(&mut app, 59, 15));
        let middle = 15 / 2;
        assert!(
            rows[middle].contains(expected),
            "{lang:?}: {}",
            rows[middle]
        );
        for (index, row) in rows.iter().enumerate() {
            if index != middle {
                assert!(row.trim().is_empty(), "{lang:?} row {index}: {row}");
            }
        }
        assert!(app.ui.hits.button_rects.is_empty() && app.ui.hits.card_rects.is_empty());
    }
}

#[test]
fn language_toggle_changes_every_chrome_text_without_a_restart_and_keeps_data_untouched() {
    let mut app = sample_app(20);
    let english = text_of(&mut app, 130, 40);
    assert!(english.contains("Interface"), "{english}");
    assert!(english.contains("Packets (20)"));
    assert!(english.contains("[L] EN"));
    app.handle_key(key(KeyCode::Char('L')));
    let korean = text_of(&mut app, 130, 40);
    assert!(korean.contains("인터페이스"), "{korean}");
    assert!(korean.contains("패킷 (20)"));
    assert!(korean.contains("[L] KO"));
    // Addresses and protocol names are never translated.
    for data in ["eth0", "93.184.216", "TCP", "network-monitor"] {
        assert!(english.contains(data) && korean.contains(data), "{data}");
    }
    app.handle_key(key(KeyCode::Char('L')));
    assert!(text_of(&mut app, 130, 40).contains("[L] EN"));
}

// 2. chips at the minimum size ------------------------------------------------
#[test]
fn language_help_and_quit_chips_survive_the_minimum_size_in_both_languages() {
    for (lang, language, help, quit) in [
        (Lang::En, "[L] EN", "[?] Help", "[q] Quit"),
        (Lang::Ko, "[L] KO", "[?] 도움말", "[q] 종료"),
    ] {
        let mut app = sample_app(10);
        app.ui.lang = lang;
        let buffer = render(&mut app, 60, 16);
        let footer = dump(&buffer)[14].clone();
        for chip in [language, help, quit] {
            assert!(footer.contains(chip), "{lang:?}: {chip} in {footer}");
        }
        let language_chip = button_rect(&app, UiAction::ToggleLanguage).expect("language chip");
        let help_chip = button_rect(&app, UiAction::ShowHelp).expect("help chip");
        let quit_chip = button_rect(&app, UiAction::Quit).expect("quit chip");
        assert!(
            language_chip.right() <= help_chip.x,
            "language is left of help"
        );
        assert!(help_chip.right() < quit_chip.x);
        assert_eq!(
            quit_chip.right(),
            59,
            "quit is pinned one column before the edge"
        );
        // The language chip is the last left chip: layout reset drops first.
        assert!(button_rect(&app, UiAction::ResetLayout).is_none());
        assert!(button_rect(&app, UiAction::TogglePause).is_some());
        for (_, rect) in &app.ui.hits.button_rects {
            assert!(rect.right() < 60);
        }
    }
}

#[test]
fn wide_footer_registers_every_chip_as_a_clickable_area() {
    let mut app = sample_app(10);
    render(&mut app, 130, 40);
    let actions: Vec<UiAction> = app
        .ui
        .hits
        .button_rects
        .iter()
        .map(|(action, _)| *action)
        .collect();
    for action in [
        UiAction::TogglePause,
        UiAction::OpenFilter,
        UiAction::OpenColumns,
        UiAction::SaveLog,
        UiAction::ResetLayout,
        UiAction::CycleLevel,
        UiAction::ToggleLanguage,
        UiAction::ShowHelp,
        UiAction::Quit,
    ] {
        assert!(actions.contains(&action), "{action:?}");
    }
    assert_eq!(actions.len(), 9);
    assert_eq!(app.ui.hits.tab_rects.len(), 6, "one hit area per tab");
    let level = button_rect(&app, UiAction::CycleLevel).unwrap();
    let language = button_rect(&app, UiAction::ToggleLanguage).unwrap();
    assert!(
        level.right() < language.x,
        "the level chip sits before the language chip"
    );
    assert!(dump(&render(&mut app, 130, 40))[38].contains("[v] debug"));
}

#[test]
fn footer_chips_share_one_neutral_surface_and_only_the_quit_key_is_red() {
    let mut app = sample_app(10);
    let buffer = render(&mut app, 130, 40);
    let filter = button_rect(&app, UiAction::OpenFilter).unwrap();
    let key_cell = buffer.cell((filter.x + 1, filter.y)).unwrap();
    assert_eq!(key_cell.symbol(), "[");
    assert_eq!(
        (key_cell.fg, key_cell.bg),
        (theme::FG_ACCENT, theme::BG_TONAL)
    );
    let quit = button_rect(&app, UiAction::Quit).unwrap();
    let quit_key = buffer.cell((quit.x + 1, quit.y)).unwrap();
    assert_eq!(
        (quit_key.fg, quit_key.bg),
        (theme::COLOR_CRIT, theme::BG_TONAL)
    );
    for y in 0..40 {
        for x in 0..130 {
            assert_ne!(
                buffer.cell((x, y)).unwrap().bg,
                theme::COLOR_CRIT_BG,
                "({x},{y})"
            );
        }
    }
}

#[test]
fn paused_state_fills_the_pause_chip_with_the_warn_color_and_names_resume() {
    let mut app = sample_app(10);
    app.paused = true;
    let buffer = render(&mut app, 130, 40);
    let chip = button_rect(&app, UiAction::TogglePause).unwrap();
    assert_eq!(
        buffer.cell((chip.x + 2, chip.y)).unwrap().bg,
        theme::COLOR_WARN
    );
    let text = dump(&buffer).join("\n");
    assert!(text.contains("Resume") && text.contains("PAUSED"), "{text}");
}

// 3. card borders -------------------------------------------------------------
#[test]
fn focused_card_has_a_thick_corner_inverted_title_and_others_are_rounded() {
    let mut app = sample_app(30);
    let buffer = render(&mut app, 130, 40);
    let list = card_rect(&app, CardId::PacketList);
    let corner = buffer.cell((list.x, list.y)).unwrap();
    assert_eq!(corner.symbol(), "┏");
    assert_eq!(corner.fg, theme::BORDER_FOCUS);
    let title = buffer.cell((list.x + 2, list.y)).unwrap();
    assert_eq!(
        (title.fg, title.bg),
        (theme::FG_ON_ACCENT, theme::BORDER_FOCUS)
    );
    assert!(title.modifier.contains(Modifier::BOLD));
    let detail = card_rect(&app, CardId::PacketDetail);
    let other = buffer.cell((detail.x, detail.y)).unwrap();
    assert_eq!(other.symbol(), "╭");
    assert_eq!(other.fg, theme::BORDER_DIM);
    assert_ne!(
        buffer.cell((detail.x + 2, detail.y)).unwrap().bg,
        theme::BORDER_FOCUS
    );
}

#[test]
fn hovered_card_uses_the_hover_border_unless_it_is_focused() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let detail = card_rect(&app, CardId::PacketDetail);
    app.handle_mouse(
        mouse(MouseEventKind::Moved, detail.x + 5, detail.y + 3),
        Instant::now(),
    );
    let buffer = render(&mut app, 130, 40);
    let corner = buffer.cell((detail.x, detail.y)).unwrap();
    assert_eq!((corner.symbol(), corner.fg), ("╭", theme::BORDER_HOVER));
    let list = card_rect(&app, CardId::PacketList);
    assert_eq!(
        buffer.cell((list.x, list.y)).unwrap().fg,
        theme::BORDER_FOCUS
    );
}

#[test]
fn clicking_a_card_moves_focus_to_it_and_the_old_card_goes_back_to_idle() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let detail = card_rect(&app, CardId::PacketDetail);
    click(&mut app, detail.x + 4, detail.y + 3, Instant::now());
    release(&mut app, detail.x + 4, detail.y + 3, Instant::now());
    let buffer = render(&mut app, 130, 40);
    assert_eq!(buffer.cell((detail.x, detail.y)).unwrap().symbol(), "┏");
    let list = card_rect(&app, CardId::PacketList);
    assert_eq!(buffer.cell((list.x, list.y)).unwrap().symbol(), "╭");
}

// 4. scrollbar ----------------------------------------------------------------
#[test]
fn scrollbar_thumb_track_and_gutter_use_gray_colors_and_leave_the_border_alone() {
    for (width, height) in SIZES {
        let mut app = sample_app(100);
        let buffer = render(&mut app, width, height);
        let list = card_rect(&app, CardId::PacketList);
        let gutter_x = list.right() - 2;
        let border_x = list.right() - 1;
        let body_top = list.y + 2;
        let body_bottom = list.bottom() - 1;
        let mut thumb_cells = 0;
        let mut track_cells = 0;
        for y in body_top..body_bottom {
            let cell = buffer.cell((gutter_x, y)).unwrap();
            match cell.symbol() {
                "█" => {
                    thumb_cells += 1;
                    assert_eq!(cell.fg, theme::SCROLL_THUMB_FOCUS, "{width}x{height} y={y}");
                }
                "░" => {
                    track_cells += 1;
                    assert_eq!(cell.fg, theme::SCROLL_TRACK, "{width}x{height} y={y}");
                }
                other => panic!("{width}x{height}: unexpected gutter glyph {other:?} at y={y}"),
            }
            assert_ne!(cell.fg, theme::BORDER_FOCUS);
            let border = buffer.cell((border_x, y)).unwrap();
            assert_eq!(border.symbol(), "┃", "{width}x{height}: border untouched");
            assert_eq!(border.fg, theme::BORDER_FOCUS);
        }
        assert!(thumb_cells >= 1 && track_cells >= 1, "{width}x{height}");
        // LIVE follow pins the thumb to the bottom of the track.
        assert_eq!(
            buffer.cell((gutter_x, body_bottom - 1)).unwrap().symbol(),
            "█",
            "{width}x{height}"
        );
    }
}

#[test]
fn an_unfocused_card_shows_a_dimmer_thumb_and_a_short_list_shows_no_scrollbar() {
    let mut app = sample_app(100);
    render(&mut app, 100, 30);
    app.ui.set_focus(CardId::PacketDetail);
    let buffer = render(&mut app, 100, 30);
    let list = card_rect(&app, CardId::PacketList);
    let thumb = (list.y + 2..list.bottom() - 1)
        .map(|y| buffer.cell((list.right() - 2, y)).unwrap())
        .find(|cell| cell.symbol() == "█")
        .expect("thumb");
    assert_eq!(thumb.fg, theme::SCROLL_THUMB);

    let mut short = sample_app(3);
    let buffer = render(&mut short, 100, 30);
    let list = card_rect(&short, CardId::PacketList);
    for y in list.y + 2..list.bottom() - 1 {
        let symbol = buffer.cell((list.right() - 2, y)).unwrap().symbol();
        assert!(
            symbol != "█" && symbol != "░",
            "no scrollbar for a short list: {symbol}"
        );
    }
    assert!(
        short
            .ui
            .hits
            .scroll_hits
            .iter()
            .all(|hit| hit.card != CardId::PacketList)
    );
}

#[test]
fn position_label_sits_on_the_bottom_border_when_the_list_overflows() {
    let mut app = sample_app(100);
    app.follow = false;
    app.selected = 0;
    let rows = dump(&render(&mut app, 100, 30));
    let list = card_rect(&app, CardId::PacketList);
    let bottom = &rows[usize::from(list.bottom() - 1)];
    assert!(bottom.contains("1-15 / 100"), "{bottom}");
}

// 5. selection band -----------------------------------------------------------
#[test]
fn selected_row_is_a_band_in_the_selection_color_inside_the_gutter() {
    let mut app = sample_app(10);
    app.follow = false;
    app.selected = 4;
    let buffer = render(&mut app, 100, 30);
    let list = card_rect(&app, CardId::PacketList);
    let row_y = list.y + 2 + 4;
    for x in list.x + 1..list.right() - 2 {
        let cell = buffer.cell((x, row_y)).unwrap();
        assert_eq!(cell.bg, theme::BG_SELECTED, "x={x}");
    }
    assert_eq!(
        buffer.cell((list.x + 3, row_y)).unwrap().fg,
        theme::FG_STRONG
    );
    assert_ne!(
        buffer.cell((list.x + 3, row_y + 1)).unwrap().bg,
        theme::BG_SELECTED
    );
    assert_eq!(
        buffer.cell((list.x, row_y)).unwrap().symbol(),
        "┃",
        "the border is not painted"
    );
}

#[test]
fn an_unfocused_list_keeps_a_quiet_band_that_leaves_text_colors() {
    let mut app = sample_app(10);
    app.follow = false;
    app.selected = 2;
    render(&mut app, 100, 30);
    app.ui.set_focus(CardId::PacketDetail);
    let buffer = render(&mut app, 100, 30);
    let list = card_rect(&app, CardId::PacketList);
    let cell = buffer.cell((list.x + 3, list.y + 2 + 2)).unwrap();
    assert_eq!(cell.bg, theme::BG_SELECTED_IDLE);
    assert_ne!(cell.fg, theme::FG_STRONG);
}

// 6. toast --------------------------------------------------------------------
#[test]
fn only_an_error_toast_is_a_red_fill_and_it_clears_after_four_seconds() {
    let mut app = sample_app(10);
    let now = Instant::now();
    for level in [ToastLevel::Info, ToastLevel::Success] {
        app.ui.toast = Some(Toast {
            text: "note".into(),
            level,
            expires_at: now + Duration::from_secs(4),
        });
        let buffer = render(&mut app, 130, 40);
        for y in 0..40 {
            for x in 0..130 {
                assert_ne!(buffer.cell((x, y)).unwrap().bg, theme::COLOR_CRIT_BG);
            }
        }
    }
    app.ui.toast = Some(Toast {
        text: "failed".into(),
        level: ToastLevel::Error,
        expires_at: now + Duration::from_secs(4),
    });
    let buffer = render(&mut app, 130, 40);
    let cell = buffer.cell((3, 39)).unwrap();
    assert_eq!((cell.bg, cell.fg), (theme::COLOR_CRIT_BG, theme::FG_STRONG));
    assert!(dump(&buffer)[39].contains("failed"));
    // The hint row lists keys after the toast.
    assert!(dump(&buffer)[39].contains("Tab Next tab"));
    assert!(!app.ui.expire_toast(now + Duration::from_millis(3999)));
    assert!(app.ui.expire_toast(now + Duration::from_secs(4)));
}

#[test]
fn hint_row_follows_the_pointer_then_the_focused_card() {
    let mut app = sample_app(10);
    let rows = dump(&render(&mut app, 130, 40));
    assert!(rows[39].contains("↑↓ Select row"), "{}", rows[39]);
    assert!(rows[39].contains("s Sort column"));
    assert!(rows[39].contains("? All keys"));
    app.ui.scroll_hover = Some(CardId::PacketList);
    let rows = dump(&render(&mut app, 130, 40));
    assert!(rows[39].contains("Track-click page"), "{}", rows[39]);
    app.ui.scroll_hover = None;
    app.ui.set_focus(CardId::PacketDetail);
    let rows = dump(&render(&mut app, 130, 40));
    assert!(rows[39].contains("↑↓ Scroll") && !rows[39].contains("s Sort column"));
}

// True when any cell holds a double-line box glyph (overlay borders only).
fn has_double_line(buffer: &Buffer) -> bool {
    let area = buffer.area;
    (0..area.height).any(|y| {
        (0..area.width).any(|x| {
            buffer.cell((x, y)).is_some_and(|cell| {
                !cell.symbol().is_empty() && "═║╔╗╚╝╠╣╦╩╬".contains(cell.symbol())
            })
        })
    })
}

#[test]
fn double_lines_appear_only_in_overlays_never_on_cards_in_any_state() {
    let mut app = sample_app(10);
    let plain = render(&mut app, 100, 40);
    assert!(!has_double_line(&plain), "idle and focused cards");

    app.ui.hover_card = Some(CardId::PacketDetail);
    assert!(!has_double_line(&render(&mut app, 100, 40)), "hovered card");

    app.ui.show_help = true;
    let overlay = render(&mut app, 100, 40);
    assert!(
        has_double_line(&overlay),
        "the help overlay keeps its double border"
    );
}

#[test]
fn every_rendered_cell_stays_inside_the_glyph_budget_in_both_languages() {
    for lang in LANGS {
        for (width, height) in SIZES {
            let mut app = sample_app(40);
            app.ui.lang = lang;
            app.bookmarks.insert(1);
            app.packets
                .push(threat_packet(40, ThreatKind::RareExternal));
            for tab in CaptureTab::ALL {
                app.switch_tab(tab);
                app.refresh_stats(Instant::now());
                let found = glyph_budget_violations(&render(&mut app, width, height));
                assert!(
                    found.is_empty(),
                    "{lang:?} {width}x{height} {tab:?}: {found:?}"
                );
            }
            app.switch_tab(CaptureTab::Packets);
            app.ui.show_help = true;
            let found = glyph_budget_violations(&render(&mut app, width, height));
            assert!(
                found.is_empty(),
                "{lang:?} {width}x{height} help: {found:?}"
            );
        }
    }
}

#[test]
fn the_glyph_budget_check_passes_the_listed_glyphs_and_flags_the_dropped_ones() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 1));
    buffer.set_string(0, 0, "a┌╌╎·…●○↑↓░█한", ratatui::style::Style::default());
    assert!(glyph_budget_violations(&buffer).is_empty());
    buffer.set_string(0, 0, "▸→»«←", ratatui::style::Style::default());
    assert_eq!(glyph_budget_violations(&buffer).len(), 5);
}

// 7. help overlay -------------------------------------------------------------
#[test]
fn help_overlay_dims_the_backdrop_draws_a_double_border_and_lists_both_sections() {
    for (lang, keyboard, mouse_title, close) in [
        (Lang::En, "Keyboard", "Mouse", "Any key or click: close"),
        (Lang::Ko, "키보드", "마우스", "아무 키·클릭: 닫기"),
    ] {
        let mut app = sample_app(10);
        app.ui.lang = lang;
        app.ui.show_help = true;
        let buffer = render(&mut app, 100, 40);
        assert_eq!(buffer.cell((1, 0)).unwrap().fg, theme::BORDER_DIM);
        let mut corner = None;
        for y in 0..40 {
            for x in 0..100 {
                if buffer.cell((x, y)).unwrap().symbol() == "╔" {
                    corner = Some(buffer.cell((x, y)).unwrap().fg);
                }
            }
        }
        assert_eq!(corner, Some(theme::FG_ACCENT), "{lang:?}");
        let text = dump(&buffer).join("\n");
        for needle in [keyboard, mouse_title, close] {
            assert!(text.contains(needle), "{lang:?}: {needle}");
        }
        assert!(
            text.contains("Shift+Arrows"),
            "key labels stay untranslated"
        );
    }
}

#[test]
fn help_overlay_truncates_with_a_notice_line_when_the_window_is_short() {
    let mut app = sample_app(10);
    app.ui.show_help = true;
    let text = text_of(&mut app, 60, 16);
    assert!(text.contains("… (enlarge window to show all)"), "{text}");
    app.ui.lang = Lang::Ko;
    assert!(text_of(&mut app, 60, 16).contains("… (창을 키우면 전체 표시)"));
}

// 8. app bar ------------------------------------------------------------------
#[test]
fn app_bar_shows_the_badge_source_state_counters_and_a_freshness_dot() {
    let mut app = sample_app(5);
    app.total_received = 1_234;
    app.total_dropped = 7;
    let buffer = render(&mut app, 130, 40);
    let bar = dump(&buffer)[0].clone();
    for needle in [
        "network-monitor",
        "Interface",
        "eth0",
        "LIVE",
        "1,234 packets",
        "7 dropped",
        "○ Waiting",
    ] {
        assert!(bar.contains(needle), "{needle} in {bar}");
    }
    let badge = buffer.cell((2, 0)).unwrap();
    assert_eq!(
        (badge.fg, badge.bg),
        (theme::FG_ON_ACCENT, theme::FG_ACCENT)
    );

    let now = Instant::now();
    app.clock = now;
    app.last_packet_at = Some(now);
    assert!(dump(&render(&mut app, 130, 40))[0].contains("● 0 pkt/s"));
    app.clock = now + Duration::from_secs(5);
    let idle = dump(&render(&mut app, 130, 40))[0].clone();
    assert!(idle.contains("Idle 5s"), "{idle}");
    app.paused = true;
    assert!(dump(&render(&mut app, 130, 40))[0].contains("PAUSED"));
    app.paused = false;
    app.follow = false;
    assert!(dump(&render(&mut app, 130, 40))[0].contains("BROWSE"));
    app.ui.lang = Lang::Ko;
    let korean = dump(&render(&mut app, 130, 40))[0].clone();
    assert!(
        korean.contains("탐색") && korean.contains("패킷") && korean.contains("5초째 수신 없음"),
        "{korean}"
    );
}

#[test]
fn a_file_source_names_the_file_and_has_no_freshness_dot() {
    let mut app = sample_app(5);
    app.source = CaptureSource::File("trace.pcap".into());
    let bar = dump(&render(&mut app, 130, 40))[0].clone();
    assert!(bar.contains("File") && bar.contains("trace.pcap"), "{bar}");
    assert!(!bar.contains("Waiting"));
    app.ui.lang = Lang::Ko;
    assert!(dump(&render(&mut app, 130, 40))[0].contains("파일"));
}

// 9. layout and cards ---------------------------------------------------------
#[test]
fn pane_toggles_hide_cards_and_the_throughput_strip_needs_height() {
    let mut app = sample_app(30);
    let full = text_of(&mut app, 100, 30);
    for needle in ["Packets (30)", "Packet Details", "Hex Dump", "Throughput"] {
        assert!(full.contains(needle), "{needle}");
    }
    let small = text_of(&mut app, 60, 16);
    assert!(
        !small.contains("Throughput"),
        "the strip is the first row to go"
    );
    app.handle_key(key(KeyCode::Char('d')));
    assert!(!text_of(&mut app, 100, 30).contains("Packet Details"));
    app.handle_key(key(KeyCode::Char('x')));
    assert!(!text_of(&mut app, 100, 30).contains("Hex Dump"));
    app.handle_key(key(KeyCode::Char('z')));
    let restored = text_of(&mut app, 100, 30);
    assert!(restored.contains("Packet Details") && restored.contains("Hex Dump"));
    app.handle_key(key(KeyCode::Char('z')));
    let hidden = text_of(&mut app, 100, 30);
    assert!(!hidden.contains("Packet Details") && !hidden.contains("Hex Dump"));
}

#[test]
fn detail_and_hex_cards_show_the_selected_packet_and_reset_scroll_on_a_new_selection() {
    let mut app = sample_app(10);
    app.follow = false;
    app.selected = 3;
    let text = text_of(&mut app, 130, 40);
    for needle in [
        "Internet Protocol Version 4",
        "Src Port: 40003",
        "0000   02 00 00",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    app.ui.views[CardId::PacketDetail as usize].scroll = 5;
    app.selected = 4;
    render(&mut app, 130, 40);
    assert_eq!(app.ui.views[CardId::PacketDetail as usize].scroll, 0);
    let empty = text_of(&mut capture_app(), 130, 40);
    assert!(empty.contains("No packet selected"));
}

#[test]
fn reordering_swaps_the_list_and_detail_rects_but_keeps_slot_sizes() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let list = card_rect(&app, CardId::PacketList);
    let detail = card_rect(&app, CardId::PacketDetail);
    assert!(list.x < detail.x);
    app.ui
        .layout
        .move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
    render(&mut app, 130, 40);
    assert_eq!(card_rect(&app, CardId::PacketList), detail);
    assert_eq!(card_rect(&app, CardId::PacketDetail), list);
}

#[test]
fn every_clickable_element_of_the_list_registers_a_hit_area() {
    let mut app = sample_app(100);
    render(&mut app, 130, 40);
    let hits = &app.ui.hits;
    assert_eq!(
        hits.header_cells.len(),
        8,
        "one header cell per shown column"
    );
    assert!(hits.column_hits.len() >= 3, "column edges");
    assert_eq!(
        hits.row_hits
            .iter()
            .filter(|hit| hit.card == CardId::PacketList)
            .count(),
        1
    );
    assert!(
        hits.scroll_hits
            .iter()
            .any(|hit| hit.card == CardId::PacketList)
    );
    let titled: Vec<CardId> = hits.title_hits.iter().map(|hit| hit.card).collect();
    for card in [CardId::PacketList, CardId::PacketDetail, CardId::HexDump] {
        assert!(titled.contains(&card), "{card:?} title hit");
    }
    assert_eq!(
        hits.split_hits.len(),
        2,
        "rows boundary and columns boundary"
    );
    assert_eq!(
        hits.slot_groups.len(),
        1,
        "only the list/detail group reorders"
    );
    assert_eq!(hits.slot_groups[0].group, SplitGroup::CaptureCols);
    for header in &hits.header_cells {
        let card = card_rect(&app, header.card);
        assert!(header.rect.right() <= card.right() - 2, "{header:?}");
    }
}

// 10. tables: sort, columns ---------------------------------------------------
fn header_of(app: &App, column: usize) -> Rect {
    app.ui
        .hits
        .header_cells
        .iter()
        .find(|hit| hit.card == CardId::PacketList && hit.column == column)
        .map(|hit| hit.rect)
        .unwrap_or_else(|| panic!("no header for column {column}"))
}

#[test]
fn header_click_cycles_numbers_descending_then_ascending_then_capture_order() {
    let mut app = sample_app(40);
    for (lang, word) in [(Lang::En, "Sort Len"), (Lang::Ko, "정렬 길이")] {
        app.ui.lang = lang;
        app.ui.sorts[CardId::PacketList as usize] = None;
        render(&mut app, 130, 40);
        let header = header_of(&app, 6);
        let now = Instant::now();
        click(&mut app, header.x, header.y, now);
        release(&mut app, header.x, header.y, now);
        let first = text_of(&mut app, 130, 40);
        assert!(
            first.contains(&format!("{word}↓")),
            "{lang:?}: numbers start descending"
        );
        let header = header_of(&app, 6);
        click(&mut app, header.x, header.y, now);
        let second = text_of(&mut app, 130, 40);
        assert!(second.contains(&format!("{word}↑")), "{lang:?}");
        let header = header_of(&app, 6);
        click(&mut app, header.x, header.y, now);
        let third = text_of(&mut app, 130, 40);
        assert!(!third.contains(word), "{lang:?}: the third click resets");
        assert!(app.ui.sorts[CardId::PacketList as usize].is_none());
    }
}

#[test]
fn a_text_column_starts_ascending_and_the_active_header_carries_the_arrow() {
    let mut app = sample_app(40);
    render(&mut app, 130, 40);
    let header = header_of(&app, 5);
    click(&mut app, header.x, header.y, Instant::now());
    assert_eq!(
        app.ui.sorts[CardId::PacketList as usize],
        Some(SortState {
            column: 5,
            descending: false
        })
    );
    let buffer = render(&mut app, 130, 40);
    let header = header_of(&app, 5);
    let row = dump(&buffer)[usize::from(header.y)].clone();
    assert!(row.contains("Proto↑"), "{row}");
    assert_eq!(
        buffer.cell((header.x, header.y)).unwrap().fg,
        theme::FG_ACCENT
    );
}

#[test]
fn sorted_rows_follow_typed_keys_not_labels_so_switching_language_keeps_the_order() {
    let mut app = sample_app(12);
    app.ui
        .change_sort(CardId::PacketList, SortChange::SelectColumn(6));
    app.follow = false;
    app.order_dirty = true;
    app.refresh_order(Instant::now());
    let before = app.order.clone();
    app.handle_key(key(KeyCode::Char('L')));
    render(&mut app, 100, 30);
    assert_eq!(app.order, before);
    let lengths: Vec<u32> = app
        .order
        .iter()
        .map(|index| app.visible_packet(*index).unwrap().summary.length)
        .collect();
    assert!(
        lengths.windows(2).all(|pair| pair[0] >= pair[1]),
        "{lengths:?}"
    );
}

#[test]
fn dragging_a_column_edge_resizes_it_saves_on_release_and_double_click_resets() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let edge = app
        .ui
        .hits
        .column_hits
        .iter()
        .find(|hit| hit.column == 2)
        .copied()
        .expect("process column edge");
    let now = Instant::now();
    click(&mut app, edge.grab.x, edge.grab.y, now);
    drag_to(&mut app, edge.grab.x + 4, edge.grab.y, now);
    assert_eq!(
        app.ui.column_widths[CardId::PacketList as usize][2],
        Some(edge.width + 4)
    );
    assert_eq!(
        release(&mut app, edge.grab.x + 4, edge.grab.y, now),
        Command::SaveState
    );
    render(&mut app, 130, 40);
    let wider = app
        .ui
        .hits
        .column_hits
        .iter()
        .find(|hit| hit.column == 2)
        .copied()
        .unwrap();
    assert_eq!(wider.width, edge.width + 4);

    // A click without dragging, then a second press inside the window, resets to auto.
    click(&mut app, wider.grab.x, wider.grab.y, now);
    release(&mut app, wider.grab.x, wider.grab.y, now);
    let outcome = click(
        &mut app,
        wider.grab.x,
        wider.grab.y,
        now + Duration::from_millis(200),
    );
    assert_eq!(outcome, Command::SaveState);
    assert_eq!(app.ui.column_widths[CardId::PacketList as usize][2], None);
}

#[test]
fn dragged_widths_cap_at_512_and_fit_without_overflow_at_every_width() {
    for width in 60..=140u16 {
        let mut app = sample_app(30);
        app.ui.column_widths[CardId::PacketList as usize][7] = Some(u16::MAX);
        render(&mut app, width, 24);
        let list = card_rect(&app, CardId::PacketList);
        for hit in &app.ui.hits.header_cells {
            if hit.card == CardId::PacketList {
                assert!(hit.rect.right() <= list.right() - 2, "{width}: {hit:?}");
            }
        }
    }
}

#[test]
fn natural_widths_show_values_without_ellipsis_when_room_exists() {
    let mut app = sample_app(10);
    let rows = dump(&render(&mut app, 130, 40));
    let row = rows
        .iter()
        .find(|row| row.contains("93.184.216.11:443"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    // Only the flexible Info column may end in an ellipsis; every cell before it is whole.
    let before_info = &row[..row.find("TCP").unwrap()];
    assert!(!before_info.contains('…'), "{row}");
    assert!(row.contains("TCP") && row.contains("↑ OUT"), "{row}");
}

#[test]
fn narrow_lists_hide_right_columns_but_keep_info_and_hidden_columns_free_room() {
    let mut app = sample_app(10);
    render(&mut app, 60, 16);
    let shown: Vec<usize> = app
        .ui
        .hits
        .header_cells
        .iter()
        .filter(|hit| hit.card == CardId::PacketList)
        .map(|hit| hit.column)
        .collect();
    assert!(
        shown.contains(&7),
        "the Info column is hidden last: {shown:?}"
    );
    assert!(shown.contains(&0), "{shown:?}");
    assert!(
        shown.len() < 8,
        "not every column fits at 60 columns: {shown:?}"
    );
    // Hidden columns free their room.
    app.handle_key(key(KeyCode::Char('c')));
    for digit in ['1', '2', '3', '4'] {
        app.handle_key(key(KeyCode::Char(digit)));
    }
    app.handle_key(key(KeyCode::Esc));
    render(&mut app, 130, 40);
    let shown: Vec<usize> = app
        .ui
        .hits
        .header_cells
        .iter()
        .map(|hit| hit.column)
        .collect();
    assert_eq!(shown, vec![4, 5, 6, 7]);
}

#[test]
fn column_editor_row_names_columns_in_the_active_language() {
    let mut app = sample_app(5);
    app.handle_key(key(KeyCode::Char('c')));
    app.handle_key(key(KeyCode::Char('2')));
    let english = dump(&render(&mut app, 130, 40))[2].clone();
    assert!(
        english.contains("Columns:") && english.contains("[2]Dir (off)"),
        "{english}"
    );
    app.ui.lang = Lang::Ko;
    let korean = dump(&render(&mut app, 130, 40))[2].clone();
    assert!(
        korean.contains("열:") && korean.contains("[2]방향 (끔)"),
        "{korean}"
    );
}

// 11. scrollbar interactions through real events ------------------------------
#[test]
fn thumb_drag_moves_the_view_track_click_pages_and_the_wheel_scrolls_the_card_under_it() {
    let mut app = sample_app(200);
    render(&mut app, 130, 40);
    let hit = *app
        .ui
        .hits
        .scroll_hits
        .iter()
        .find(|hit| hit.card == CardId::PacketList)
        .expect("scroll hit");
    let now = Instant::now();
    // The thumb sits at the bottom in LIVE; grab it and drag to the top.
    let thumb =
        crate::tui::cards::scroll_thumb(hit.total, hit.visible, hit.scroll, hit.track.height)
            .unwrap();
    let grab_y = hit.track.y + thumb.start;
    click(&mut app, hit.track.x, grab_y, now);
    assert!(app.ui.scroll_drag.is_some());
    drag_to(&mut app, hit.track.x, hit.track.y, now);
    assert_eq!(app.ui.views[CardId::PacketList as usize].scroll, 0);
    assert!(!app.follow, "dragging away from the end leaves LIVE");
    release(&mut app, hit.track.x, hit.track.y, now);
    assert!(app.ui.scroll_drag.is_none());

    // Track click below the thumb pages down by the visible rows.
    render(&mut app, 130, 40);
    let hit = *app
        .ui
        .hits
        .scroll_hits
        .iter()
        .find(|hit| hit.card == CardId::PacketList)
        .unwrap();
    click(&mut app, hit.track.x, hit.track.bottom() - 1, now);
    assert_eq!(
        app.ui.views[CardId::PacketList as usize].scroll,
        hit.visible
    );

    // The wheel scrolls only the card under the pointer.
    render(&mut app, 130, 40);
    let detail = card_rect(&app, CardId::PacketDetail);
    let list_scroll = app.ui.views[CardId::PacketList as usize].scroll;
    app.ui.views[CardId::PacketDetail as usize].total = 50;
    app.ui.views[CardId::PacketDetail as usize].visible = 10;
    app.handle_mouse(
        mouse(MouseEventKind::ScrollDown, detail.x + 3, detail.y + 3),
        now,
    );
    assert_eq!(app.ui.views[CardId::PacketDetail as usize].scroll, 3);
    assert_eq!(
        app.ui.views[CardId::PacketList as usize].scroll,
        list_scroll
    );
}

#[test]
fn clicking_a_row_selects_it_and_leaves_live_with_the_detail_pane_shown() {
    let mut app = sample_app(30);
    app.show_detail = false;
    render(&mut app, 100, 30);
    let row = app
        .ui
        .hits
        .row_hits
        .iter()
        .find(|hit| hit.card == CardId::PacketList)
        .copied()
        .unwrap();
    click(&mut app, row.rect.x + 3, row.rect.y + 2, Instant::now());
    assert_eq!(app.selected, row.offset + 2);
    assert!(app.show_detail && !app.follow);
}

// 12. grips and title press ---------------------------------------------------
#[test]
fn split_grips_are_drawn_only_under_the_pointer_or_while_dragging() {
    let mut app = sample_app(30);
    let idle = render(&mut app, 130, 40);
    let rows_hit = app
        .ui
        .hits
        .split_hits
        .iter()
        .find(|hit| hit.group == SplitGroup::CaptureRows)
        .copied()
        .unwrap();
    let probe = (rows_hit.line.x + rows_hit.line.width / 2, rows_hit.line.y);
    assert_ne!(idle.cell(probe).unwrap().fg, theme::SPLIT_GRIP_HOT);
    app.handle_mouse(
        mouse(MouseEventKind::Moved, probe.0, probe.1),
        Instant::now(),
    );
    let hovered = render(&mut app, 130, 40);
    assert_eq!(hovered.cell(probe).unwrap().symbol(), "━");
    assert_eq!(hovered.cell(probe).unwrap().fg, theme::SPLIT_GRIP_HOT);
    app.handle_mouse(mouse(MouseEventKind::Moved, 0, 0), Instant::now());
    let left = render(&mut app, 130, 40);
    assert_ne!(
        left.cell(probe).unwrap().fg,
        theme::SPLIT_GRIP_HOT,
        "leaving clears the grip"
    );
}

#[test]
fn dragging_the_row_boundary_resizes_the_hex_pane_and_release_saves() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let hit = app
        .ui
        .hits
        .split_hits
        .iter()
        .find(|hit| hit.group == SplitGroup::CaptureRows)
        .copied()
        .unwrap();
    let before = card_rect(&app, CardId::HexDump).height;
    let now = Instant::now();
    click(&mut app, hit.grab.x + 5, hit.grab.y, now);
    drag_to(&mut app, hit.grab.x + 5, hit.grab.y - 3, now);
    assert_eq!(
        release(&mut app, hit.grab.x + 5, hit.grab.y - 3, now),
        Command::SaveState
    );
    render(&mut app, 130, 40);
    assert_eq!(card_rect(&app, CardId::HexDump).height, before + 3);
}

#[test]
fn a_title_hold_reorders_the_cards_and_a_title_tap_only_focuses() {
    let mut app = sample_app(30);
    render(&mut app, 130, 40);
    let title = app
        .ui
        .hits
        .title_hits
        .iter()
        .find(|hit| hit.card == CardId::PacketList)
        .copied()
        .unwrap();
    let detail = card_rect(&app, CardId::PacketDetail);
    let start = Instant::now();
    click(&mut app, title.rect.x + 1, title.rect.y, start);
    assert!(app.on_tick(start + Duration::from_millis(400)));
    assert!(app.ui.card_move.is_some());
    // The moved card's title is flagged and its border is light double-dashed.
    let buffer = render(&mut app, 130, 40);
    let list = card_rect(&app, CardId::PacketList);
    assert_eq!(buffer.cell((list.x, list.y)).unwrap().symbol(), "┌");
    let left_edge = buffer.cell((list.x, list.y + 1)).unwrap();
    assert_eq!(left_edge.symbol(), "╎");
    assert_eq!(left_edge.fg, theme::FG_ACCENT);
    let bottom_edge = buffer.cell((list.x + 2, list.y + list.height - 1)).unwrap();
    assert_eq!(bottom_edge.symbol(), "╌");
    assert!(dump(&buffer)[usize::from(list.y)].contains("moving"));
    assert!(
        !has_double_line(&buffer),
        "double lines are reserved for overlays"
    );
    drag_to(
        &mut app,
        detail.x + 5,
        detail.y + 3,
        start + Duration::from_millis(450),
    );
    let outcome = release(
        &mut app,
        detail.x + 5,
        detail.y + 3,
        start + Duration::from_millis(500),
    );
    assert_eq!(outcome, Command::SaveState);
    render(&mut app, 130, 40);
    assert!(card_rect(&app, CardId::PacketList).x > card_rect(&app, CardId::PacketDetail).x);
}

// 13. statistics views --------------------------------------------------------
fn app_with_traffic() -> App {
    let mut app = capture_app();
    for number in 0..12u64 {
        let remote = [93, 184, 216, (number % 4) as u8 + 10];
        let (src, dst, sport, dport) = if number % 2 == 0 {
            ([10, 0, 0, 1], remote, 40000, 443)
        } else {
            (remote, [10, 0, 0, 1], 443, 40000)
        };
        app.packets.push(tcp_packet(
            number,
            src,
            sport,
            dst,
            dport,
            100 + number as u32,
            b"",
        ));
        *app.proto_counts.entry("TCP".into()).or_insert(0) += 1;
    }
    app.bookmarks.insert(3);
    app.bookmarks.insert(5);
    app
}

#[test]
fn statistics_tabs_render_both_cards_in_both_languages_with_the_main_card_focused() {
    for lang in LANGS {
        for (tab, english, korean) in [
            (
                CaptureTab::Stats,
                ["Top talkers", "Protocol distribution"],
                ["상위 통신", "프로토콜 분포"],
            ),
            (
                CaptureTab::Flows,
                ["Flows (", "Connection timeline"],
                ["플로우 (", "연결 타임라인"],
            ),
            (
                CaptureTab::Alerts,
                ["Threat alerts (0)", "Bookmarks (2)"],
                ["위협 경고 (0)", "북마크 (2)"],
            ),
        ] {
            let mut app = app_with_traffic();
            app.ui.lang = lang;
            app.switch_tab(tab);
            app.refresh_stats(Instant::now());
            render(&mut app, 60, 16);
            for (width, height) in [(100, 30), (130, 40)] {
                let text = text_of(&mut app, width, height);
                let titles = if lang == Lang::En { english } else { korean };
                for title in titles {
                    assert!(
                        text.contains(title),
                        "{lang:?} {tab:?} {width}x{height}: {title}"
                    );
                }
            }
            assert_eq!(app.ui.hits.card_rects.len(), 2, "{tab:?} has two cards");
            assert_eq!(app.ui.focused_card(), tab.main_card());
        }
    }
}

#[test]
fn a_statistics_table_sorts_by_header_click_and_its_selection_follows_the_row() {
    let mut app = app_with_traffic();
    app.switch_tab(CaptureTab::Flows);
    app.refresh_stats(Instant::now());
    render(&mut app, 130, 40);
    app.handle_key(key(KeyCode::Down));
    render(&mut app, 130, 40);
    let chosen = app.ui.selected_keys[CardId::StatFlows as usize];
    let header = app
        .ui
        .hits
        .header_cells
        .iter()
        .find(|hit| hit.card == CardId::StatFlows && hit.column == 3)
        .copied()
        .unwrap();
    click(&mut app, header.rect.x, header.rect.y, Instant::now());
    let text = text_of(&mut app, 130, 40);
    assert!(text.contains("Sort Bytes↓"), "{text}");
    assert_eq!(app.ui.selected_keys[CardId::StatFlows as usize], chosen);
}

#[test]
fn the_stream_view_shows_both_directions_and_an_empty_stream_says_so() {
    let mut app = capture_app();
    app.packets.push(tcp_packet(
        1,
        [10, 0, 0, 1],
        40000,
        [10, 0, 0, 2],
        9000,
        80,
        b"GET /",
    ));
    app.packets.push(tcp_packet(
        2,
        [10, 0, 0, 2],
        9000,
        [10, 0, 0, 1],
        40000,
        80,
        b"200 OK",
    ));
    app.selected = 0;
    app.follow = false;
    app.handle_key(key(KeyCode::Char('f')));
    assert_eq!(app.tab, CaptureTab::Stream);
    let text = text_of(&mut app, 100, 30);
    assert!(
        text.contains(">> GET /") && text.contains("<< 200 OK"),
        "{text}"
    );
    assert!(text.contains("Follow TCP"));

    for (lang, headline, hint) in [
        (
            Lang::En,
            "No stream open",
            "Select a TCP packet and press f",
        ),
        (Lang::Ko, "열린 스트림 없음", "TCP 패킷 선택 후 f"),
    ] {
        let mut none = capture_app();
        none.ui.lang = lang;
        none.handle_key(key(KeyCode::Char('5')));
        let text = text_of(&mut none, 100, 30);
        assert!(text.contains(headline) && text.contains(hint), "{lang:?}");
    }
}

// 15. tabs, output level, alerts, settings ------------------------------------
const TAB_LABELS_EN: [&str; 6] = ["Packets", "Flows", "Stats", "Alerts", "Stream", "Settings"];
const TAB_LABELS_KO: [&str; 6] = ["패킷", "흐름", "통계", "경고", "스트림", "설정"];

#[test]
fn every_tab_shows_all_tab_labels_and_the_pinned_chips_at_the_size_matrix() {
    for (lang, labels, chips) in [
        (Lang::En, TAB_LABELS_EN, ["[L] EN", "[?] Help", "[q] Quit"]),
        (
            Lang::Ko,
            TAB_LABELS_KO,
            ["[L] KO", "[?] 도움말", "[q] 종료"],
        ),
    ] {
        for tab in CaptureTab::ALL {
            let mut app = app_with_traffic();
            app.ui.lang = lang;
            app.switch_tab(tab);
            app.refresh_stats(Instant::now());
            for (width, height) in SIZES {
                let rows = dump(&render(&mut app, width, height));
                for label in labels {
                    assert!(
                        rows[1].contains(label),
                        "{lang:?} {tab:?} {width}x{height}: {label} in {}",
                        rows[1]
                    );
                }
                assert_eq!(app.ui.hits.tab_rects.len(), 6);
                if (width, height) == (60, 16) {
                    for chip in chips {
                        assert!(rows[14].contains(chip), "{lang:?} {tab:?}: {chip}");
                    }
                }
            }
            let rows = dump(&render(&mut app, 59, 15));
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    !row.trim().is_empty(),
                    index == 15 / 2,
                    "{lang:?} {tab:?} row {index}: {row}"
                );
            }
            assert!(app.ui.hits.tab_rects.is_empty() && app.ui.hits.card_rects.is_empty());
        }
    }
}

#[test]
fn number_keys_tab_keys_arrows_and_a_tab_bar_click_switch_tabs_and_esc_goes_back() {
    let mut app = sample_app(10);
    for (digit, tab) in ('1'..='6').zip(CaptureTab::ALL) {
        app.handle_key(key(KeyCode::Char(digit)));
        assert_eq!(app.tab, tab, "{digit}");
    }
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.tab, CaptureTab::Packets, "Tab wraps to the first tab");
    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(app.tab, CaptureTab::Settings);
    app.handle_key(shift_key(KeyCode::Tab));
    assert_eq!(app.tab, CaptureTab::Stream);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.tab, CaptureTab::Settings);
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.tab, CaptureTab::Stream);

    render(&mut app, 130, 40);
    let (_, alerts) = app.ui.hits.tab_rects[CaptureTab::Alerts.index()];
    let now = Instant::now();
    click(&mut app, alerts.x + 1, alerts.y, now);
    release(&mut app, alerts.x + 1, alerts.y, now);
    assert_eq!(app.tab, CaptureTab::Alerts);
    assert_eq!(app.ui.focused_card(), CardId::ThreatTable);
    let text = text_of(&mut app, 130, 40);
    assert!(text.contains("Threat alerts"), "{text}");

    app.handle_key(key(KeyCode::Esc));
    assert!(app.tab == CaptureTab::Packets && !app.should_quit);
    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit, "Esc on Packets quits");
}

#[test]
fn the_threat_table_lists_only_threats_at_the_level_toned_on_zebra_without_fill() {
    let mut app = capture_app();
    let mut retransmitted = tcp_packet(0, [10, 0, 0, 1], 40000, [93, 184, 216, 2], 443, 60, b"");
    retransmitted.retransmission = true;
    app.packets.push(retransmitted);
    app.packets.push(threat_packet(1, ThreatKind::RareExternal));
    app.packets.push(threat_packet(2, ThreatKind::PortScan));
    app.packets.push(threat_packet(3, ThreatKind::HostSweep));
    app.ui.level_choice = Some(OutputLevel::Warn);
    app.switch_tab(CaptureTab::Alerts);
    app.refresh_stats(Instant::now());
    let buffer = render(&mut app, 130, 40);
    assert_eq!(app.ui.views[CardId::ThreatTable as usize].total, 3);
    let rows = dump(&buffer);
    for tag in ["RARE-DEST", "PORT-SCAN", "HOST-SWEEP"] {
        assert!(rows.iter().any(|row| row.contains(tag)), "{tag}");
    }

    // Below the title border, every cell is panel or zebra outside the
    // selected band, and no row is filled.
    let card = card_rect(&app, CardId::ThreatTable);
    let band_y = card.y + 2;
    for y in card.y + 1..card.bottom() {
        for x in card.x..card.right() {
            let bg = buffer.cell((x, y)).unwrap().bg;
            for filled in [208u8, 203, 160] {
                assert_ne!(bg, Color::Indexed(filled), "({x},{y})");
            }
            if y != band_y {
                assert!(
                    bg == theme::BG_PANEL || bg == theme::BG_ZEBRA,
                    "({x},{y}) {bg:?}"
                );
            }
        }
    }
    for (tag, tone) in [
        ("PORT-SCAN", theme::COLOR_CRIT),
        ("HOST-SWEEP", theme::COLOR_CRIT),
    ] {
        let y = rows.iter().position(|row| row.contains(tag)).unwrap();
        let x = rows[y][..rows[y].find(tag).unwrap()].chars().count() as u16;
        let cell = buffer.cell((x, y as u16)).unwrap();
        assert_eq!(cell.fg, tone, "{tag}");
        assert!(!cell.modifier.contains(Modifier::BOLD), "{tag}");
    }

    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.ui.min_level(), OutputLevel::Error);
    app.refresh_stats(Instant::now());
    let text = text_of(&mut app, 130, 40);
    assert_eq!(app.ui.views[CardId::ThreatTable as usize].total, 2);
    assert!(!text.contains("RARE-DEST"), "{text}");
}

#[test]
fn settings_enter_cycles_the_level_on_row_zero_and_toggles_the_language_on_row_one() {
    let mut app = sample_app(5);
    app.handle_key(key(KeyCode::Char('6')));
    assert_eq!(app.tab, CaptureTab::Settings);
    let text = text_of(&mut app, 100, 30);
    for needle in [
        "Output level",
        "debug info warn error",
        "config default",
        "Language",
        "EN",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Command::SaveState);
    assert_eq!(app.ui.level_choice, Some(OutputLevel::Info));
    assert!(!text_of(&mut app, 100, 30).contains("config default"));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.ui.views[CardId::SettingsList as usize].selected, 1);
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Command::SaveState);
    assert_eq!(app.ui.lang, Lang::Ko);
    assert_eq!(app.ui.level_choice, Some(OutputLevel::Info));
    let korean = text_of(&mut app, 100, 30);
    assert!(korean.contains("언어") && korean.contains("KO"), "{korean}");
}

// 14. empty and idle states ---------------------------------------------------
#[test]
fn empty_list_states_name_the_cause_and_the_next_step_in_both_languages() {
    for (lang, waiting, hint) in [
        (
            Lang::En,
            "Waiting for packets",
            "Traffic appears here as it is captured",
        ),
        (Lang::Ko, "패킷 대기 중", "캡처되는 대로 여기에 표시"),
    ] {
        let mut app = capture_app();
        app.ui.lang = lang;
        let text = text_of(&mut app, 100, 30);
        assert!(text.contains(waiting) && text.contains(hint), "{lang:?}");
        app.paused = true;
        let text = text_of(&mut app, 100, 30);
        assert!(text.contains(if lang == Lang::En {
            "Capture paused"
        } else {
            "캡처 일시정지"
        }));
    }
    let mut filtered = sample_app(5);
    filtered.mode = InputMode::FilterInput;
    filtered.filter_input = "tcp.port == 1".into();
    filtered.handle_key(key(KeyCode::Enter));
    let text = text_of(&mut filtered, 100, 30);
    assert!(text.contains("No packet matches the filter"), "{text}");
    assert!(text.contains("Press / to edit the filter"));
}

#[test]
fn filter_row_shows_idle_editing_applied_and_direction_states() {
    let mut app = sample_app(5);
    assert!(dump(&render(&mut app, 130, 40))[2].contains("press / to filter"));
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('t')));
    app.handle_key(key(KeyCode::Char('c')));
    app.handle_key(key(KeyCode::Char('p')));
    let editing = dump(&render(&mut app, 130, 40))[2].clone();
    assert!(editing.contains("Filter: tcp│"), "{editing}");
    app.handle_key(key(KeyCode::Enter));
    let applied = dump(&render(&mut app, 130, 40))[2].clone();
    assert!(
        applied.contains("tcp") && applied.contains("[Applied]"),
        "{applied}"
    );
    app.handle_key(key(KeyCode::Char('i')));
    assert!(dump(&render(&mut app, 130, 40))[2].contains("[IN only]"));
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Char('l')));
    app.handle_key(key(KeyCode::Char('/')));
    app.mode = InputMode::Search;
    let search = dump(&render(&mut app, 130, 40))[2].clone();
    assert!(search.contains("Regex search:"), "{search}");
    app.ui.lang = Lang::Ko;
    assert!(dump(&render(&mut app, 130, 40))[2].contains("정규식 검색:"));
}

#[test]
fn throughput_strip_names_the_rate_in_the_active_language() {
    let mut app = sample_app(5);
    app.throughput_pps.push_back(12);
    app.throughput_bps.push_back(3_400);
    let english = dump(&render(&mut app, 100, 30));
    assert!(
        english[27].contains("Throughput 12 pkt/s 3.3K/s"),
        "{}",
        english[27]
    );
    app.ui.lang = Lang::Ko;
    let korean = dump(&render(&mut app, 100, 30));
    assert!(korean[27].contains("처리량 12 패킷/초"), "{}", korean[27]);
}

// 16. tab reordering ----------------------------------------------------------
fn tab_rect(app: &App, tab: CaptureTab) -> Rect {
    app.ui
        .hits
        .capture_tabs
        .iter()
        .find(|(drawn, _)| *drawn == tab)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("tab {tab:?} was not drawn"))
}

fn drawn_tabs(app: &App) -> Vec<CaptureTab> {
    app.ui
        .hits
        .capture_tabs
        .iter()
        .map(|(tab, _)| *tab)
        .collect()
}

#[test]
fn greater_than_on_packets_draws_the_next_tab_first_and_digit_one_selects_it() {
    let mut app = sample_app(10);
    assert_eq!(app.handle_key(key(KeyCode::Char('>'))), Command::SaveState);
    let rows = dump(&render(&mut app, 100, 30));
    assert_eq!(
        drawn_tabs(&app)[..2],
        [CaptureTab::Flows, CaptureTab::Packets]
    );
    assert!(
        rows[1].contains("1 Flows") && rows[1].contains("2 Packets"),
        "{}",
        rows[1]
    );
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.tab, CaptureTab::Flows);
}

#[test]
fn dragging_the_first_tab_onto_the_third_previews_the_order_and_release_commits_it() {
    for lang in LANGS {
        for (width, height) in SIZES {
            let case = format!("{lang:?} {width}x{height}");
            let mut app = sample_app(10);
            app.ui.lang = lang;
            render(&mut app, width, height);
            let packets = tab_rect(&app, CaptureTab::Packets);
            let stats = tab_rect(&app, CaptureTab::Stats);
            let now = Instant::now();

            // 1. press opens the drag, dragging over Stats previews
            assert_eq!(
                click(&mut app, packets.x + 1, packets.y, now),
                Command::None
            );
            assert!(app.tab_drag.is_some(), "{case}");
            let pointer = stats.x + stats.width / 2;
            assert_eq!(drag_to(&mut app, pointer, stats.y, now), Command::None);
            let preview = [CaptureTab::Flows, CaptureTab::Stats, CaptureTab::Packets];
            assert_eq!(app.ui.tab_order[..3], preview, "{case}");

            // 2. redraw: new order, slot numbers, active dragged tab, drag hints
            let buffer = render(&mut app, width, height);
            assert_eq!(drawn_tabs(&app)[..3], preview, "{case}");
            let rows = dump(&buffer);
            assert!(rows[1].trim_start().starts_with('1'), "{case} {}", rows[1]);
            let moved = tab_rect(&app, CaptureTab::Packets);
            let cell = buffer.cell((moved.x, moved.y)).unwrap();
            assert_eq!(cell.bg, theme::BG_PANEL, "{case}");
            let hint_row: String = rows[usize::from(height) - 1].split_whitespace().collect();
            let cancel: String = i18n::text(lang, Msg::HintCancel)
                .split_whitespace()
                .collect();
            if width >= 100 {
                assert!(hint_row.contains(&cancel), "{case} {hint_row}");
            }

            // 3. release commits, saves, and shows the toast
            assert_eq!(
                release(&mut app, pointer, stats.y, now),
                Command::SaveState,
                "{case}"
            );
            assert!(app.tab_drag.is_none(), "{case}");
            assert_eq!(
                app.ui.toast.as_ref().map(|toast| toast.text.as_str()),
                Some(i18n::text(lang, Msg::ToastTabMoved)),
                "{case}"
            );

            // 4. digits follow the drawn order
            app.handle_key(key(KeyCode::Char('1')));
            assert_eq!(app.tab, CaptureTab::Flows, "{case}");
        }
    }
}

#[test]
fn esc_during_a_tab_drag_restores_the_order_and_a_lost_release_commits_it() {
    // 1. Esc: baseline order, no quit
    let mut app = sample_app(10);
    render(&mut app, 100, 30);
    let packets = tab_rect(&app, CaptureTab::Packets);
    let stats = tab_rect(&app, CaptureTab::Stats);
    let now = Instant::now();
    click(&mut app, packets.x + 1, packets.y, now);
    drag_to(&mut app, stats.x + 1, stats.y, now);
    assert_ne!(app.ui.tab_order, CaptureTab::ALL);
    assert_eq!(app.handle_key(key(KeyCode::Char('2'))), Command::None);
    assert_eq!(app.tab, CaptureTab::Packets);
    assert_eq!(app.handle_key(key(KeyCode::Esc)), Command::None);
    assert!(!app.should_quit && app.tab_drag.is_none());
    assert_eq!(app.ui.tab_order, CaptureTab::ALL);

    // 2. buttonless move after a lost release keeps the shown order
    render(&mut app, 100, 30);
    click(&mut app, packets.x + 1, packets.y, now);
    drag_to(&mut app, stats.x + 1, stats.y, now);
    let moved = app.ui.tab_order;
    assert_eq!(
        app.handle_mouse(mouse(MouseEventKind::Moved, 0, 10), now),
        Command::SaveState
    );
    assert!(app.tab_drag.is_none());
    assert_eq!(app.ui.tab_order, moved);
    assert_eq!(moved[2], CaptureTab::Packets);
}

#[test]
fn a_plain_tab_click_switches_tabs_and_saves_nothing() {
    let mut app = sample_app(10);
    render(&mut app, 100, 30);
    let stats = tab_rect(&app, CaptureTab::Stats);
    let now = Instant::now();
    assert_eq!(click(&mut app, stats.x + 1, stats.y, now), Command::None);
    assert_eq!(release(&mut app, stats.x + 1, stats.y, now), Command::None);
    assert_eq!(app.tab, CaptureTab::Stats);
    assert_eq!(app.ui.tab_order, CaptureTab::ALL);
    assert!(app.tab_drag.is_none() && app.ui.toast.is_none());
}

#[test]
fn a_resize_during_a_moved_tab_drag_commits_and_saves() {
    let mut app = sample_app(10);
    render(&mut app, 100, 30);
    let packets = tab_rect(&app, CaptureTab::Packets);
    let stats = tab_rect(&app, CaptureTab::Stats);
    let now = Instant::now();
    click(&mut app, packets.x + 1, packets.y, now);
    drag_to(&mut app, stats.x + 1, stats.y, now);
    let moved = app.ui.tab_order;
    assert_ne!(moved, CaptureTab::ALL);
    assert_eq!(app.handle_resize(), Command::SaveState);
    assert!(app.tab_drag.is_none());
    assert_eq!(app.ui.tab_order, moved);
}

#[test]
fn a_reordered_tab_bar_draws_the_labels_in_the_new_order_in_both_languages() {
    let order = [
        CaptureTab::Settings,
        CaptureTab::Stream,
        CaptureTab::Alerts,
        CaptureTab::Stats,
        CaptureTab::Flows,
        CaptureTab::Packets,
    ];
    for (lang, labels) in [(Lang::En, TAB_LABELS_EN), (Lang::Ko, TAB_LABELS_KO)] {
        for (width, height) in [(60, 16), (130, 40)] {
            let case = format!("{lang:?} {width}x{height}");
            let mut app = sample_app(10);
            app.ui.lang = lang;
            app.ui.tab_order = order;
            let buffer = render(&mut app, width, height);
            let rows = dump(&buffer);
            let positions: Vec<usize> = order
                .iter()
                .map(|tab| {
                    rows[1]
                        .find(labels[tab.index()])
                        .unwrap_or_else(|| panic!("{case}: {tab:?} in {}", rows[1]))
                })
                .collect();
            assert!(
                positions.windows(2).all(|pair| pair[0] < pair[1]),
                "{case}: {}",
                rows[1]
            );
            assert_eq!(drawn_tabs(&app), order, "{case}");
            assert!(rows[1].trim_start().starts_with('1'), "{case} {}", rows[1]);
            let active = tab_rect(&app, CaptureTab::Packets);
            let cell = buffer.cell((active.x, active.y)).unwrap();
            assert_eq!(cell.bg, theme::BG_PANEL, "{case}");
        }
    }
}
