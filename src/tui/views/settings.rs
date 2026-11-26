//! Settings tab: one selectable card with the output level and the display
//! language. Enter on a row changes it (state.rs); level codes are config
//! values and stay untranslated.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::tui::cards::CardId;
use crate::tui::format::{fit_display, text_width};
use crate::tui::i18n::{self, Msg};
use crate::tui::level::OutputLevel;
use crate::tui::state::App;
use crate::tui::theme;
use crate::tui::views::common::{TextCard, render_text_card};

// 1. settings card ------------------------------------------------------------
// Row 0: output level, every code in cycle order with the current one
// accented (a dim note while the config default applies); row 1: language.
// Labels share one padded column.
pub fn render_settings_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let current = app.ui.min_level();
    let level_label = i18n::text(lang, Msg::SettingOutputLevel);
    let language_label = i18n::text(lang, Msg::SettingLanguage);
    let label_width = text_width(level_label).max(text_width(language_label)) + 2;
    let mut level_line = vec![Span::styled(
        fit_display(level_label, label_width),
        theme::STYLE_TEXT,
    )];
    for (position, level) in OutputLevel::ALL.iter().enumerate() {
        let style = if *level == current {
            theme::STYLE_ACCENT
        } else {
            theme::STYLE_FAINT
        };
        if position > 0 {
            level_line.push(Span::raw(" "));
        }
        level_line.push(Span::styled(level.code().to_string(), style));
    }
    if app.ui.level_choice.is_none() {
        level_line.push(Span::styled(
            format!("  ({})", i18n::text(lang, Msg::SettingConfigDefault)),
            theme::STYLE_DIM,
        ));
    }
    let lines = vec![
        Line::from(level_line),
        Line::from(vec![
            Span::styled(fit_display(language_label, label_width), theme::STYLE_TEXT),
            Span::styled(
                i18n::text(lang, Msg::ChipLanguage).to_string(),
                theme::STYLE_ACCENT,
            ),
        ]),
    ];
    let spec = TextCard {
        card: CardId::SettingsList,
        title: i18n::text(lang, Msg::TitleSettings).to_string(),
        hint: i18n::text(lang, Msg::HintChangeSetting).to_string(),
        lines,
        empty: None,
    };
    render_text_card(frame, &mut app.ui, area, spec);
}
