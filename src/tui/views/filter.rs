//! The one-row filter bar: idle hint, the filter being edited, the regex
//! search prompt, the applied filter, or the column toggles.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use network_monitor_core::decode::Direction;

use crate::tui::columns::PACKET_COLUMNS;
use crate::tui::i18n::{self, Msg};
use crate::tui::state::{App, InputMode};
use crate::tui::theme;

// The cursor glyph drawn inside the edited text.
const CURSOR: char = '│';

pub fn render_filter_row(frame: &mut Frame, area: Rect, app: &App) {
    let lang = app.ui.lang;
    let base = Style::default().bg(theme::BG_BASE);
    let label =
        |msg: Msg| Span::styled(format!(" {}: ", i18n::text(lang, msg)), theme::STYLE_ACCENT);
    let spans: Vec<Span<'static>> = match app.mode {
        InputMode::ColumnConfig => {
            let mut spans = vec![label(Msg::ColumnsLabel)];
            for (index, def) in PACKET_COLUMNS.iter().enumerate() {
                let on = app.col_visible[index];
                let state = if on {
                    String::new()
                } else {
                    format!(" ({})", i18n::text(lang, Msg::ColumnOff))
                };
                let style = if on {
                    Style::default().fg(theme::FILTER_ACTIVE)
                } else {
                    theme::STYLE_DIM
                };
                spans.push(Span::styled(
                    format!("[{}]{}{}  ", index + 1, i18n::text(lang, def.label), state),
                    style,
                ));
            }
            spans
        }
        InputMode::FilterInput => {
            let mut text = app.filter_input.clone();
            if text.is_char_boundary(app.filter_cursor) {
                text.insert(app.filter_cursor, CURSOR);
            }
            vec![
                label(Msg::FilterLabel),
                Span::styled(text, Style::default().fg(theme::FILTER_EDIT)),
            ]
        }
        InputMode::Search => vec![
            label(Msg::SearchLabel),
            Span::styled(
                format!("{}{CURSOR}", app.search_query),
                Style::default().fg(theme::FILTER_EDIT),
            ),
        ],
        InputMode::Normal => {
            let dir_tag = match app.dir_filter {
                Some(Direction::In) => Some(Msg::FilterInOnly),
                Some(Direction::Out) => Some(Msg::FilterOutOnly),
                _ => None,
            };
            if app.active_filter.is_some() {
                let mut spans = vec![
                    label(Msg::FilterLabel),
                    Span::styled(
                        app.filter_input.clone(),
                        Style::default().fg(theme::FILTER_ACTIVE),
                    ),
                    Span::styled(
                        format!("  [{}]", i18n::text(lang, Msg::FilterApplied)),
                        theme::STYLE_DIM,
                    ),
                ];
                if let Some(tag) = dir_tag {
                    spans.push(Span::styled(
                        format!("  [{}]", i18n::text(lang, tag)),
                        Style::default().fg(theme::FILTER_ACTIVE),
                    ));
                }
                spans
            } else if let Some(tag) = dir_tag {
                vec![
                    label(Msg::FilterLabel),
                    Span::styled(
                        i18n::text(lang, tag).to_string(),
                        Style::default().fg(theme::FILTER_ACTIVE),
                    ),
                ]
            } else {
                vec![
                    label(Msg::FilterLabel),
                    Span::styled(
                        i18n::text(lang, Msg::FilterIdle).to_string(),
                        theme::STYLE_DIM,
                    ),
                ]
            }
        }
    };
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}
