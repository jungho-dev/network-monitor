//! Shared screen chrome: size gate, app bar, tab bar, footer chips with the
//! hint row and toast, and the help overlay. Both TUIs build their own specs
//! and call these; hit areas are recorded while drawing.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use crate::tui::cards::{CardKind, UiAction};
use crate::tui::format::{fit_display, text_width};
use crate::tui::i18n::{self, HelpRow, Lang, Msg};
use crate::tui::interaction::{ToastLevel, Ui};
use crate::tui::theme;

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

// 1. size gate ----------------------------------------------------------------
// Paints the base surface; below the minimum size one centered warning line
// replaces the screen and the caller draws nothing else (returns true).
pub fn render_base_and_gate(frame: &mut Frame, lang: Lang) -> bool {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::BG_BASE).fg(theme::FG_TEXT)),
        area,
    );
    if area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT {
        return false;
    }
    let notice = Paragraph::new(Line::from(Span::styled(
        i18n::too_small(lang, MIN_WIDTH, MIN_HEIGHT, area.width, area.height),
        theme::STYLE_WARN,
    )))
    .alignment(Alignment::Center);
    frame.render_widget(
        notice,
        Rect::new(area.x, area.y + area.height / 2, area.width, 1),
    );
    true
}

// 2. app bar ------------------------------------------------------------------
// Badge, then `detail`; `right` is capped at half the width.
pub fn render_app_bar(
    frame: &mut Frame,
    area: Rect,
    detail: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) {
    let right_line = Line::from(right);
    let right_width = (right_line.width() as u16).min(area.width / 2);
    let columns =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);
    let mut left = vec![Span::styled(
        format!(" {} ", i18n::PRODUCT_NAME),
        Style::default()
            .fg(theme::FG_ON_ACCENT)
            .bg(theme::FG_ACCENT)
            .add_modifier(Modifier::BOLD),
    )];
    left.extend(detail);
    frame.render_widget(
        Paragraph::new(Line::from(left)).style(Style::default().bg(theme::BG_BAR)),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right_line).style(Style::default().bg(theme::BG_BAR)),
        columns[1],
    );
}

// 3. tab bar ------------------------------------------------------------------
// Compacts in steps: normal gaps, no gaps, then number-only padding.
pub fn render_tab_bar(frame: &mut Frame, ui: &mut Ui, area: Rect, labels: &[&str], active: usize) {
    let row_end = area.x.saturating_add(area.width);
    let mut cursor = area.x.saturating_add(1);
    let count = labels.len();
    let full_labels: usize = labels.iter().map(|label| text_width(label)).sum();
    let full_width = full_labels + 4 * count + count;
    let compact_width = full_labels + 4 * count + 1;
    let level = if full_width <= usize::from(area.width) {
        0
    } else if compact_width <= usize::from(area.width) {
        1
    } else {
        2
    };
    let gap = u16::from(level != 1);
    for (index, label) in labels.iter().enumerate() {
        let (key_style, name_style, background) = if index == active {
            (
                Style::default()
                    .fg(theme::FG_ACCENT)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(theme::FG_STRONG)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                theme::BG_PANEL,
            )
        } else {
            (
                theme::STYLE_DIM,
                Style::default().fg(theme::TAB_IDLE_NAME),
                theme::BG_BASE,
            )
        };
        let line = match level {
            2 => Line::from(vec![
                Span::styled(format!("{}", index + 1), key_style),
                Span::styled((*label).to_string(), name_style),
            ]),
            _ => Line::from(vec![
                Span::styled(format!(" {} ", index + 1), key_style),
                Span::styled((*label).to_string(), name_style),
                Span::raw(" "),
            ]),
        };
        let width = line.width() as u16;
        if cursor.saturating_add(width) > row_end {
            break;
        }
        let rect = Rect::new(cursor, area.y, width, area.height);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(background)),
            rect,
        );
        ui.hits.tab_rects.push((index, rect));
        cursor = cursor.saturating_add(width + gap);
    }
}

// 4. footer -------------------------------------------------------------------
// One footer chip: action, key text, and its label.
pub struct Chip {
    pub action: UiAction,
    pub key: &'static str,
    pub label: String,
    // Filled warn chip for an active state (for example paused).
    pub active: bool,
}

// Hints in priority: context hints first, tail hints always keep their room.
pub struct HintSet {
    pub context: Vec<&'static str>,
    pub tail: Vec<&'static str>,
}

// Two rows: chips (left group drops from its tail, language chip last, help
// and quit pinned right) and the hint row with the toast first.
pub fn render_footer(frame: &mut Frame, ui: &mut Ui, area: Rect, left: Vec<Chip>, hints: &HintSet) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::BG_BAR)),
        area,
    );
    render_chip_row(frame, ui, rows[0], left);
    render_hint_row(frame, ui, rows[1], hints);
}

fn render_chip_row(frame: &mut Frame, ui: &mut Ui, row: Rect, left: Vec<Chip>) {
    // 1. chip styles (only an active state is filled; quit is a red letter on a neutral chip)
    let lang = ui.lang;
    let chip = Style::default().bg(theme::BG_TONAL);
    let key_style = chip.fg(theme::FG_ACCENT).add_modifier(Modifier::BOLD);
    let name_style = chip.fg(theme::FG_TEXT);
    let quit_key_style = chip.fg(theme::COLOR_CRIT).add_modifier(Modifier::BOLD);
    let active_style = Style::default()
        .fg(theme::FG_ON_ACCENT)
        .bg(theme::COLOR_WARN)
        .add_modifier(Modifier::BOLD);
    let make_line = |key: &'static str, label: String, key_use: Style, name_use: Style| {
        Line::from(vec![
            Span::styled(format!(" [{key}]"), key_use),
            Span::styled(format!(" {label} "), name_use),
        ])
    };
    let left_chips: Vec<(UiAction, Line<'static>)> = left
        .into_iter()
        .map(|entry| {
            let (key_use, name_use) = if entry.active {
                (active_style, active_style)
            } else {
                (key_style, name_style)
            };
            (
                entry.action,
                make_line(entry.key, entry.label, key_use, name_use),
            )
        })
        .collect();
    let language_chip = (
        UiAction::ToggleLanguage,
        make_line(
            "L",
            i18n::text(lang, Msg::ChipLanguage).to_string(),
            key_style,
            name_style,
        ),
    );
    let right_chips: Vec<(UiAction, Line<'static>)> = vec![
        (
            UiAction::ShowHelp,
            make_line(
                "?",
                i18n::text(lang, Msg::ChipHelp).to_string(),
                key_style,
                name_style,
            ),
        ),
        (
            UiAction::Quit,
            make_line(
                "q",
                i18n::text(lang, Msg::ChipQuit).to_string(),
                quit_key_style,
                name_style,
            ),
        ),
    ];

    // 2. layout: right group 1 column before the end, left group up to the room
    //    that still leaves the language chip its place
    let row_end = row.x.saturating_add(row.width);
    let right_width: u16 = right_chips
        .iter()
        .map(|(_, line)| line.width() as u16 + 1)
        .sum::<u16>()
        .saturating_sub(1);
    let right_start = row_end.saturating_sub(1 + right_width).max(row.x);
    let mut placed: Vec<(UiAction, Rect, Line<'static>)> = Vec::new();
    let mut cursor = row.x.saturating_add(1);
    let language_width = language_chip.1.width() as u16;
    for (action, line) in left_chips {
        let width = line.width() as u16;
        if cursor.saturating_add(width + 1 + language_width) >= right_start {
            break;
        }
        placed.push((action, Rect::new(cursor, row.y, width, row.height), line));
        cursor = cursor.saturating_add(width + 1);
    }
    if cursor.saturating_add(language_width) < right_start {
        let (action, line) = language_chip;
        placed.push((
            action,
            Rect::new(cursor, row.y, language_width, row.height),
            line,
        ));
    }
    let mut cursor = right_start;
    for (action, line) in right_chips {
        let width = line.width() as u16;
        placed.push((action, Rect::new(cursor, row.y, width, row.height), line));
        cursor = cursor.saturating_add(width + 1);
    }

    // 3. draw and register hit areas
    for (action, rect, line) in placed {
        frame.render_widget(Paragraph::new(line).style(chip), rect);
        ui.hits.button_rects.push((action, rect));
    }
}

fn render_hint_row(frame: &mut Frame, ui: &Ui, row: Rect, hints: &HintSet) {
    // 1. toast chip (only an error is a red fill)
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    if let Some(toast) = &ui.toast {
        let chip = Style::default().bg(theme::BG_TONAL);
        let (dot_style, text_style) = match toast.level {
            ToastLevel::Info => (chip.fg(theme::FG_ACCENT), chip.fg(theme::FG_TEXT)),
            ToastLevel::Success => (chip.fg(theme::COLOR_OK), chip.fg(theme::FG_TEXT)),
            ToastLevel::Error => {
                let filled = Style::default()
                    .fg(theme::FG_STRONG)
                    .bg(theme::COLOR_CRIT_BG)
                    .add_modifier(Modifier::BOLD);
                (filled, filled)
            }
        };
        let text_budget = usize::from(row.width).saturating_sub(2 + 4);
        let shown = if text_width(&toast.text) > text_budget {
            fit_display(&toast.text, text_budget).trim_end().to_string()
        } else {
            toast.text.clone()
        };
        spans.push(Span::styled(" ● ", dot_style));
        spans.push(Span::styled(shown, text_style));
        spans.push(Span::styled(" ", text_style));
    }

    // 2. hints that fit: tail room is reserved first, a hint that does not fit is skipped whole
    let mut room = usize::from(row.width).saturating_sub(Line::from(spans.clone()).width());
    let tail_width: usize = hints.tail.iter().map(|hint| 2 + text_width(hint)).sum();
    let reserved = if tail_width <= room { tail_width } else { 0 };
    let mut shown_hints: Vec<&str> = Vec::new();
    for hint in &hints.context {
        let piece = 2 + text_width(hint);
        if piece > room.saturating_sub(reserved) {
            continue;
        }
        shown_hints.push(hint);
        room -= piece;
    }
    for hint in &hints.tail {
        let piece = 2 + text_width(hint);
        if piece > room {
            continue;
        }
        shown_hints.push(hint);
        room -= piece;
    }
    let mut leading = ui.toast.is_some();
    for hint in shown_hints {
        let (key, rest) = hint.split_once(' ').unwrap_or((hint, ""));
        if leading {
            spans.push(Span::raw("  "));
        }
        leading = true;
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(theme::HINT_KEY)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {rest}"), theme::STYLE_DIM));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BG_BAR)),
        row,
    );
}

// Context hints in priority: arranging, pointer targets, then the focused card.
pub fn context_hints(ui: &Ui, kind: CardKind, sortable: bool) -> Vec<&'static str> {
    let lang = ui.lang;
    let pick = |msgs: &[Msg]| -> Vec<&'static str> {
        msgs.iter().map(|msg| i18n::text(lang, *msg)).collect()
    };
    let mut hints: Vec<&'static str> = Vec::new();
    if ui.card_move.is_some() {
        hints.extend(pick(&[Msg::HintRelease, Msg::HintCancel]));
        return hints;
    }
    if ui.split_hover.is_some() || ui.split_drag.is_some() {
        hints.extend(pick(&[Msg::HintDragCardSize, Msg::HintResetLayout]));
    }
    if ui.column_hover.is_some() || ui.column_drag.is_some() {
        hints.extend(pick(&[
            Msg::HintDragColumnWidth,
            Msg::HintDoubleClickAuto,
            Msg::HintResetLayout,
        ]));
    }
    if ui.scroll_hover.is_some() || ui.scroll_drag.is_some() {
        hints.extend(pick(&[Msg::HintDragScroll, Msg::HintTrackPage]));
    }
    match kind {
        CardKind::Selectable => {
            hints.extend(pick(&[
                Msg::HintSelectRow,
                Msg::HintPage,
                Msg::HintFirstLast,
            ]));
        }
        CardKind::Scrollable => hints.extend(pick(&[Msg::HintScroll, Msg::HintPage])),
    }
    if sortable {
        hints.extend(pick(&[Msg::HintSortColumn, Msg::HintSortOrder]));
    }
    hints
}

// 5. help overlay -------------------------------------------------------------
// Dimmed backdrop, double-border box with keyboard and mouse sections, a close
// hint, and a truncation line when the box is shorter than its rows.
pub fn render_help_overlay(frame: &mut Frame, rows: &[HelpRow], lang: Lang) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(76);
    let height = (rows.len() as u16 + 4).min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.buffer_mut().set_style(
        area,
        Style::default()
            .fg(theme::BORDER_DIM)
            .remove_modifier(Modifier::BOLD),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme::FG_ACCENT))
        .title(Span::styled(
            format!(" {} ", i18n::text(lang, Msg::HelpTitle)),
            theme::STYLE_STRONG,
        ))
        .title_bottom(
            Line::from(Span::styled(
                format!(" {} ", i18n::text(lang, Msg::HelpClose)),
                theme::STYLE_DIM,
            ))
            .right_aligned(),
        )
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(theme::BG_PANEL));
    let inner = block.inner(rect);

    // Blank a wide glyph that straddles the box's left edge.
    if rect.x > area.x {
        let buf = frame.buffer_mut();
        for row_y in rect.y..rect.bottom() {
            let cell = &mut buf[(rect.x - 1, row_y)];
            if text_width(cell.symbol()) > 1 {
                cell.set_symbol(" ");
            }
        }
    }
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    // Key column = longest key label + 2, at most 24.
    let key_width = rows
        .iter()
        .filter_map(|row| match row {
            HelpRow::Key(keys, _) => Some(text_width(keys)),
            HelpRow::Section(_) => None,
        })
        .max()
        .map_or(0, |widest| (widest + 2).min(24));
    let mut lines: Vec<Line<'static>> = rows
        .iter()
        .map(|row| match row {
            HelpRow::Section(title) => Line::from(Span::styled(
                i18n::text(lang, *title).to_string(),
                theme::STYLE_ACCENT,
            )),
            HelpRow::Key(keys, meaning) => Line::from(vec![
                Span::styled(
                    fit_display(keys, key_width),
                    Style::default()
                        .fg(theme::HINT_KEY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(i18n::text(lang, *meaning).to_string(), theme::STYLE_TEXT),
            ]),
        })
        .collect();
    let room = usize::from(inner.height);
    if lines.len() > room {
        lines.truncate(room.saturating_sub(1));
        lines.push(Line::from(Span::styled(
            i18n::text(lang, Msg::HelpTruncated).to_string(),
            theme::STYLE_DIM,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
