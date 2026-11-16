//! Shared card drawing: bordered cards with focus and hover states, table
//! cards with sortable headers and draggable columns, text cards, the gutter
//! scrollbar, split and column grips, and empty states.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Padding, Paragraph, Row, Table};

use crate::tui::cards::{
    CardId, CardKind, ColumnHit, HeaderHit, MAX_COLUMN_WIDTH, MAX_TABLE_COLUMNS, RowHit, ScrollHit,
    TitleHit, scroll_thumb,
};
use crate::tui::columns::{ColumnFit, card_columns};
use crate::tui::format::{clip_spans_to_width, fit_display, text_width};
use crate::tui::i18n::{self, Lang, Msg};
use crate::tui::interaction::Ui;
use crate::tui::layout::SplitAxis;
use crate::tui::sort::{SortKey, sorted_positions};
use crate::tui::theme;

// Text columns shrink down to this width; one blank column separates cells.
const MIN_COLUMN_WIDTH: u16 = 3;
const TABLE_COLUMN_GAP: u16 = 1;

// 1. input types --------------------------------------------------------------
pub type TableCell = Vec<Span<'static>>;

// id = stable key of the row (selection follows it across sorts), cells and
// keys cover every column of the card, style = whole-row style.
pub struct TableRow {
    pub id: u64,
    pub cells: Vec<TableCell>,
    pub keys: Vec<SortKey>,
    pub style: Style,
}

// rows hold `rows.len()` entries starting at table row `offset` of `total`
// (windowed tables draw only the rows in view); a full table has offset 0.
pub struct CardTable {
    pub card: CardId,
    pub title: String,
    pub hint: String,
    pub rows: Vec<TableRow>,
    pub total: usize,
    pub offset: usize,
    pub windowed: bool,
    pub shown_columns: [bool; MAX_TABLE_COLUMNS],
    pub empty: Option<(String, String)>,
}

pub struct TextCard {
    pub card: CardId,
    pub title: String,
    pub hint: String,
    pub lines: Vec<Line<'static>>,
    pub empty: Option<(String, String)>,
}

// natural = width the content needs, floor = narrowest allowed, user = dragged
// width, keep = hidden only after every other column is gone.
#[derive(Debug, Clone, Copy)]
pub struct ColumnPlan {
    pub natural: u16,
    pub floor: u16,
    pub fit: ColumnFit,
    pub user: Option<u16>,
    pub keep: bool,
}

pub fn all_columns_shown() -> [bool; MAX_TABLE_COLUMNS] {
    [true; MAX_TABLE_COLUMNS]
}

// Rows a table card shows: card height minus borders and the header row.
pub fn table_visible_rows(area: Rect) -> usize {
    usize::from(area.height).saturating_sub(3)
}

// Orders full-table rows by the card's sort (capture order when none).
pub fn sort_rows(ui: &Ui, card: CardId, rows: Vec<TableRow>) -> Vec<TableRow> {
    let Some(sort) = ui.sorts[card as usize] else {
        return rows;
    };
    let keys: Vec<SortKey> = rows
        .iter()
        .map(|row| {
            row.keys
                .get(sort.column)
                .cloned()
                .unwrap_or(SortKey::Num(0))
        })
        .collect();
    let order = sorted_positions(&keys, sort.descending);
    let mut slots: Vec<Option<TableRow>> = rows.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

// 2. interactive card ---------------------------------------------------------
// Registers the card rect and its title hit area, then returns the block:
// focus = thick border and inverted title, hover = brighter rounded border,
// move mode = light double-dashed border in the accent color (double lines are
// reserved for overlays), tone = idle border color (critical tone wins over
// focus and hover), hint = dim right-aligned text dropped when it would touch
// the title.
pub fn interactive_card(
    ui: &mut Ui,
    id: CardId,
    area: Rect,
    title: &str,
    hint: &str,
    tone: Color,
) -> Block<'static> {
    ui.hits.card_rects.push((id, area));

    // 1. title text and its long-press area (inside the left and right borders)
    let lifted = ui.card_move.is_some_and(|moving| moving.card == id);
    let shown_title = if lifted {
        format!(" {} ", i18n::card_moving(ui.lang, title))
    } else {
        format!(" {title} ")
    };
    let title_width = text_width(&shown_title) as u16;
    let title_rect = Rect::new(
        area.x.saturating_add(1),
        area.y,
        title_width.min(area.width.saturating_sub(2)),
        1.min(area.height),
    );
    if !title_rect.is_empty() {
        ui.hits.title_hits.push(TitleHit {
            card: id,
            rect: title_rect,
        });
    }

    // 2. state: lifted, critical tone, focus, hover, idle
    let focused = ui.focused_card() == id;
    let hovered = ui.hover_card == Some(id);
    let border = if lifted {
        theme::FG_ACCENT
    } else if tone == theme::COLOR_CRIT {
        tone
    } else if focused {
        theme::BORDER_FOCUS
    } else if hovered {
        theme::BORDER_HOVER
    } else {
        tone
    };
    let (border_type, title_style) = if lifted {
        (
            BorderType::LightDoubleDashed,
            Style::default()
                .fg(theme::FG_ON_ACCENT)
                .bg(theme::FG_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if focused {
        (
            BorderType::Thick,
            Style::default()
                .fg(theme::FG_ON_ACCENT)
                .bg(border)
                .add_modifier(Modifier::BOLD),
        )
    } else if hovered {
        (
            BorderType::Rounded,
            Style::default()
                .fg(theme::TITLE_HOVER)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            BorderType::Rounded,
            Style::default()
                .fg(theme::TITLE_IDLE)
                .add_modifier(Modifier::BOLD),
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border))
        .title(Span::styled(shown_title, title_style))
        .style(Style::default().bg(theme::BG_PANEL));

    // 3. hint
    let needed = title_width as usize + text_width(&format!(" {hint} ")) + 4;
    if hint.is_empty() || needed > usize::from(area.width) {
        return block;
    }
    block.title_top(Line::from(Span::styled(format!(" {hint} "), theme::STYLE_DIM)).right_aligned())
}

fn position_title(
    block: Block<'static>,
    scroll: usize,
    visible: usize,
    total: usize,
) -> Block<'static> {
    if total <= visible || visible == 0 {
        return block;
    }
    let label = i18n::position_label(scroll + 1, (scroll + visible).min(total), total);
    block.title_bottom(Line::from(Span::styled(label, theme::STYLE_DIM)).right_aligned())
}

// 3. empty state --------------------------------------------------------------
// Centered headline with one dim next step (headline only in a one-row area).
pub fn render_empty_state(frame: &mut Frame, rect: Rect, headline: Span<'static>, hint: &str) {
    if rect.is_empty() {
        return;
    }
    let mut lines = vec![Line::from(headline)];
    if !hint.is_empty() && rect.height >= 2 {
        lines.push(Line::from(Span::styled(hint.to_string(), theme::STYLE_DIM)));
    }
    let used = (lines.len() as u16).min(rect.height);
    let body = Rect::new(rect.x, rect.y + (rect.height - used) / 2, rect.width, used);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), body);
}

// 4. table card ---------------------------------------------------------------
// Resolves widths from natural content, registers header, column-edge, and row
// hit areas, paints the selected band and the gutter scrollbar.
pub fn render_card_table(frame: &mut Frame, ui: &mut Ui, area: Rect, table: CardTable) {
    let CardTable {
        card,
        title,
        hint,
        rows,
        total,
        offset,
        windowed,
        shown_columns,
        empty,
    } = table;
    let defs = card_columns(card);
    let sort = ui.sorts[card as usize];
    let lang = ui.lang;
    let selectable = card.kind() == CardKind::Selectable;
    let labels: Vec<&'static str> = defs.iter().map(|def| i18n::text(lang, def.label)).collect();

    // 1. title with the active sort
    let title =
        match sort.and_then(|current| labels.get(current.column).map(|label| (*label, current))) {
            Some((label, current)) => i18n::sort_title(lang, &title, label, current.sort_arrow()),
            None => title,
        };
    let block = interactive_card(ui, card, area, &title, &hint, theme::BORDER_DIM)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let visible = usize::from(inner.height).saturating_sub(1);

    // 2. view state
    ui.clamp_view(card, total, visible);
    let view = ui.views[card as usize];
    let (scroll, selected) = (view.scroll, view.selected);

    // 3. width plan (windowed tables keep the widest width seen)
    let column_count = defs.len().min(MAX_TABLE_COLUMNS);
    let mut plans: Vec<ColumnPlan> = Vec::with_capacity(column_count);
    for index in 0..column_count {
        let def = defs[index];
        let label_width = text_width(labels[index]) + usize::from(sort.is_some());
        let content_width = rows
            .iter()
            .map(|row| {
                row.cells
                    .get(index)
                    .map_or(0, |cell| cell.iter().map(Span::width).sum::<usize>())
            })
            .max()
            .unwrap_or(0);
        let mut widest = u16::try_from(label_width.max(content_width)).unwrap_or(u16::MAX);
        if windowed {
            let sticky = &mut ui.sticky_widths[card as usize][index];
            *sticky = (*sticky).max(widest).min(MAX_COLUMN_WIDTH);
            widest = *sticky;
        }
        let fixed = def.fit == ColumnFit::Fixed;
        let natural = if fixed { def.min.max(widest) } else { widest };
        plans.push(ColumnPlan {
            natural,
            floor: if fixed {
                natural
            } else {
                MIN_COLUMN_WIDTH.min(natural)
            },
            fit: def.fit,
            user: ui.column_widths[card as usize][index],
            keep: def.keep,
        });
    }
    let mut widths = resolve_column_widths(&plans, inner.width, &shown_columns[..column_count]);
    widths.resize(defs.len(), 0);
    let shown: Vec<usize> = (0..column_count)
        .filter(|index| widths[*index] > 0)
        .collect();
    let shown_floors = shown
        .iter()
        .fold(0u16, |sum, index| sum.saturating_add(plans[*index].floor));
    let shown_gaps = (shown.len().saturating_sub(1)) as u16 * TABLE_COLUMN_GAP;

    // 4. header cells and their hit areas
    let mut header_cells: Vec<Cell<'static>> = Vec::new();
    let mut cursor = inner.x;
    for (position, index) in shown.iter().enumerate() {
        let def = defs[*index];
        let width = widths[*index];
        let active = sort.is_some_and(|current| current.column == *index);
        let name = labels[*index];
        let label = match sort {
            Some(current) if active && text_width(name) + 1 > usize::from(width) => {
                let head = fit_display(name, usize::from(width).saturating_sub(1));
                format!("{}{}", head.trim_end(), current.sort_arrow())
            }
            Some(current) if active => format!("{name}{}", current.sort_arrow()),
            _ if text_width(name) > usize::from(width) => {
                fit_display(name, usize::from(width)).trim_end().to_string()
            }
            _ => name.to_string(),
        };
        let cell = Cell::from(Line::from(label).alignment(def.align));
        header_cells.push(if active {
            cell.style(theme::STYLE_ACCENT)
        } else {
            cell
        });
        if inner.height > 0 {
            ui.hits.header_cells.push(HeaderHit {
                card,
                column: *index,
                rect: Rect::new(cursor, inner.y, width, 1),
            });
        }
        if position + 1 < shown.len() && *index < MAX_TABLE_COLUMNS && inner.height > 0 {
            let others_floor = (shown_floors - plans[*index].floor).saturating_add(shown_gaps);
            ui.hits.column_hits.push(ColumnHit {
                card,
                column: *index,
                grab: Rect::new(cursor + width, inner.y, TABLE_COLUMN_GAP, 1),
                line: Rect::new(cursor + width, inner.y, TABLE_COLUMN_GAP, inner.height),
                width,
                min: plans[*index].floor,
                max: inner
                    .width
                    .saturating_sub(others_floor)
                    .max(plans[*index].floor),
            });
        }
        cursor += width + TABLE_COLUMN_GAP;
    }

    // 5. body row hit area (header row excluded)
    if selectable && visible > 0 {
        ui.hits.row_hits.push(RowHit {
            card,
            rect: Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1),
            offset: scroll,
            len: total,
        });
    }

    // 6. rows clipped to their column widths
    let block = position_title(block, scroll, visible, total);
    let skip = scroll.saturating_sub(offset);
    let body_rows: Vec<Row<'static>> = rows
        .into_iter()
        .skip(skip)
        .take(visible)
        .map(|row| {
            let row_style = row.style;
            let cells: Vec<Cell<'static>> = row
                .cells
                .into_iter()
                .enumerate()
                .filter(|(index, _)| widths.get(*index).is_some_and(|width| *width > 0))
                .map(|(index, spans)| {
                    let clipped = clip_spans_to_width(spans, usize::from(widths[index]));
                    Cell::from(Line::from(clipped).alignment(defs[index].align))
                })
                .collect();
            Row::new(cells).style(row_style)
        })
        .collect();
    let constraints: Vec<Constraint> = shown
        .iter()
        .map(|index| Constraint::Length(widths[*index]))
        .collect();
    let drawn = Table::new(body_rows, constraints)
        .header(Row::new(header_cells).style(theme::STYLE_TABLE_HEADER))
        .block(block)
        .column_spacing(TABLE_COLUMN_GAP);
    frame.render_widget(drawn, area);

    // 7. empty body, selected band, scrollbar
    let body = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );
    if total == 0
        && let Some((headline, next_step)) = empty
    {
        render_empty_state(
            frame,
            body,
            Span::styled(headline, theme::STYLE_TEXT),
            &next_step,
        );
    }
    if selectable && total > 0 && (scroll..scroll + visible).contains(&selected) {
        let focused = ui.focused_card() == card;
        let row_y = body.y + (selected - scroll) as u16;
        paint_selection_band(frame.buffer_mut(), area, row_y, focused);
    }
    render_card_scrollbar(frame, ui, card, area, body);
}

// 5. text card ----------------------------------------------------------------
// Line-scrolled body; the selected row of a selectable card gets the band.
pub fn render_text_card(frame: &mut Frame, ui: &mut Ui, area: Rect, spec: TextCard) {
    let TextCard {
        card,
        title,
        hint,
        lines,
        empty,
    } = spec;
    let block = interactive_card(ui, card, area, &title, &hint, theme::BORDER_DIM)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let selectable = card.kind() == CardKind::Selectable;
    let total = lines.len();
    let visible = usize::from(inner.height);
    ui.clamp_view(card, total, visible);
    let view = ui.views[card as usize];
    let (scroll, selected) = (view.scroll, view.selected);
    if selectable && visible > 0 {
        ui.hits.row_hits.push(RowHit {
            card,
            rect: inner,
            offset: scroll,
            len: total,
        });
    }
    let block = position_title(block, scroll, visible, total);
    let width = usize::from(inner.width);
    let shown: Vec<Line<'static>> = lines
        .into_iter()
        .skip(scroll)
        .take(visible)
        .map(|line| {
            let Line {
                spans,
                style,
                alignment,
            } = line;
            Line {
                spans: clip_spans_to_width(spans, width),
                style,
                alignment,
            }
        })
        .collect();
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(shown), inner);
    if total == 0
        && let Some((headline, next_step)) = empty
    {
        render_empty_state(
            frame,
            inner,
            Span::styled(headline, theme::STYLE_DIM),
            &next_step,
        );
    }
    if selectable && total > 0 && (scroll..scroll + visible).contains(&selected) {
        let focused = ui.focused_card() == card;
        paint_selection_band(
            frame.buffer_mut(),
            area,
            inner.y + (selected - scroll) as u16,
            focused,
        );
    }
    render_card_scrollbar(frame, ui, card, area, inner);
}

// 6. scrollbar ----------------------------------------------------------------
// The gutter (column just inside the right border) holds a dim track and a
// gray thumb; the border column is never touched.
pub fn render_card_scrollbar(frame: &mut Frame, ui: &mut Ui, card: CardId, area: Rect, body: Rect) {
    if body.is_empty() || area.width < 4 {
        return;
    }
    let view = ui.views[card as usize];
    let Some(thumb) = scroll_thumb(view.total, view.visible, view.scroll, body.height) else {
        return;
    };
    let dragging = ui.scroll_drag.is_some_and(|drag| drag.card == card);
    let hovered = ui.scroll_hover == Some(card);
    let track_color = if dragging || hovered {
        theme::SCROLL_TRACK_HOT
    } else {
        theme::SCROLL_TRACK
    };
    let thumb_color = if dragging {
        theme::SCROLL_THUMB_DRAG
    } else if hovered {
        theme::SCROLL_THUMB_HOVER
    } else if ui.focused_card() == card {
        theme::SCROLL_THUMB_FOCUS
    } else {
        theme::SCROLL_THUMB
    };
    let gutter_x = area.right() - 2;
    let buf = frame.buffer_mut();
    for y in body.y..body.bottom() {
        buf[(gutter_x, y)].set_symbol("░").set_fg(track_color);
    }
    let thumb_top = body.y + thumb.start;
    for y in thumb_top..thumb_top + thumb.len {
        buf[(gutter_x, y)].set_symbol("█").set_fg(thumb_color);
    }
    ui.hits.scroll_hits.push(ScrollHit {
        card,
        track: Rect::new(gutter_x, body.y, 2, body.height),
        total: view.total,
        visible: view.visible,
        scroll: view.scroll,
    });
}

// Selected band: from the left padding column to just before the gutter; a
// focused card gets the strong band, others a quiet one that keeps text colors.
fn paint_selection_band(buf: &mut Buffer, area: Rect, row_y: u16, focused: bool) {
    let band = Rect::new(area.x + 1, row_y, area.width.saturating_sub(3), 1);
    let style = if focused {
        theme::STYLE_SELECTED_ROW
    } else {
        theme::STYLE_SELECTED_ROW_IDLE
    };
    buf.set_style(band, style);
}

// 7. grips --------------------------------------------------------------------
// Drawn only under the pointer or while dragging.
pub fn render_split_handles(frame: &mut Frame, ui: &Ui) {
    let dragging = ui.split_drag.map(|drag| (drag.group, drag.index));
    let buf = frame.buffer_mut();
    for hit in &ui.hits.split_hits {
        let key = Some((hit.group, hit.index));
        let dragged = dragging == key;
        let active = dragged || (dragging.is_none() && ui.split_hover == key);
        let line = hit.line;
        if !active || line.is_empty() {
            continue;
        }
        let color = if dragged {
            theme::SPLIT_GRIP_DRAG
        } else {
            theme::SPLIT_GRIP_HOT
        };
        match hit.axis {
            SplitAxis::Columns => {
                for y in line.y..line.bottom() {
                    buf[(line.x, y)].set_symbol("┃").set_fg(color);
                }
            }
            SplitAxis::Rows => {
                buf.set_style(line, Style::default().fg(color));
                let grip_width = line.width.min(6);
                let grip_x = line.x + (line.width - grip_width) / 2;
                for x in grip_x..grip_x + grip_width {
                    buf[(x, line.y)]
                        .set_symbol("━")
                        .set_fg(color)
                        .set_style(Style::default().add_modifier(Modifier::BOLD));
                }
            }
        }
    }
}

pub fn render_column_handles(frame: &mut Frame, ui: &Ui) {
    let dragging = ui.column_drag.map(|drag| (drag.card, drag.column));
    let buf = frame.buffer_mut();
    for hit in &ui.hits.column_hits {
        let key = Some((hit.card, hit.column));
        let dragged = dragging == key;
        let hovered = dragging.is_none() && ui.column_hover == key;
        if !dragged && !hovered {
            continue;
        }
        let (color, line) = if dragged {
            (theme::SPLIT_GRIP_DRAG, hit.line)
        } else {
            (theme::SPLIT_GRIP_HOT, hit.grab)
        };
        for y in line.y..line.bottom() {
            buf[(line.x, y)].set_symbol("┃").set_fg(color);
        }
    }
}

// 8. column widths ------------------------------------------------------------
// Start from natural (or dragged) widths. When short, shrink undragged text
// columns widest-first to their floor, then dragged ones; if still short, hide
// the right-most column that is not marked `keep`, then retry. Slack goes to
// the right-most undragged Grow column. `shown` marks columns the user left
// enabled (hidden ones get width 0).
pub fn resolve_column_widths(plans: &[ColumnPlan], total: u16, shown: &[bool]) -> Vec<u16> {
    let mut active: Vec<usize> = (0..plans.len())
        .filter(|index| shown.get(*index).copied().unwrap_or(true))
        .collect();
    let total = u32::from(total);
    while !active.is_empty() {
        let count = active.len();
        let mut widths: Vec<u16> = active
            .iter()
            .map(|index| {
                let plan = plans[*index];
                plan.user
                    .unwrap_or(plan.natural)
                    .min(MAX_COLUMN_WIDTH)
                    .max(plan.floor)
            })
            .collect();
        let gaps = (count as u32 - 1) * u32::from(TABLE_COLUMN_GAP);
        let used = widths.iter().map(|width| u32::from(*width)).sum::<u32>() + gaps;
        let mut deficit = used.saturating_sub(total);
        for dragged in [false, true] {
            while deficit > 0 {
                let widest = (0..count)
                    .filter(|slot| {
                        let plan = plans[active[*slot]];
                        plan.user.is_some() == dragged && widths[*slot] > plan.floor
                    })
                    .max_by_key(|slot| widths[*slot]);
                let Some(slot) = widest else {
                    break;
                };
                widths[slot] -= 1;
                deficit -= 1;
            }
        }
        if deficit > 0 {
            let drop_slot = (0..count)
                .rev()
                .find(|slot| !plans[active[*slot]].keep)
                .unwrap_or(count - 1);
            active.remove(drop_slot);
            continue;
        }
        let absorber = (0..count)
            .rev()
            .find(|slot| plans[active[*slot]].fit == ColumnFit::Grow)
            .filter(|slot| plans[active[*slot]].user.is_none());
        if let Some(slot) = absorber {
            widths[slot] += total.saturating_sub(used) as u16;
        }
        let mut result = vec![0u16; plans.len()];
        for (slot, index) in active.iter().enumerate() {
            result[*index] = widths[slot];
        }
        return result;
    }
    vec![0; plans.len()]
}

// Message for the empty-state of a filtered or waiting table.
pub fn empty_pair(lang: Lang, headline: Msg, next_step: Msg) -> Option<(String, String)> {
    Some((
        i18n::text(lang, headline).to_string(),
        i18n::text(lang, next_step).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(natural: u16, fit: ColumnFit, user: Option<u16>) -> ColumnPlan {
        ColumnPlan {
            natural,
            floor: if fit == ColumnFit::Fixed {
                natural
            } else {
                MIN_COLUMN_WIDTH.min(natural)
            },
            fit,
            user,
            keep: false,
        }
    }

    // Two text columns and one numeric column (natural widths 10, 20, 6).
    fn sample_plans() -> Vec<ColumnPlan> {
        vec![
            plan(10, ColumnFit::Shrink, None),
            plan(20, ColumnFit::Grow, None),
            plan(6, ColumnFit::Fixed, None),
        ]
    }

    fn all(count: usize) -> Vec<bool> {
        vec![true; count]
    }

    #[test]
    fn wide_card_gives_text_its_natural_width_and_the_grow_column_takes_the_slack() {
        let widths = resolve_column_widths(&sample_plans(), 100, &all(3));
        assert_eq!(widths, vec![10, 82, 6]);
        assert_eq!(
            resolve_column_widths(&sample_plans(), 38, &all(3)),
            vec![10, 20, 6]
        );
    }

    #[test]
    fn narrow_card_shrinks_widest_text_first_and_never_touches_numbers() {
        let widths = resolve_column_widths(&sample_plans(), 26, &all(3));
        assert_eq!(widths[2], 6);
        assert_eq!(widths[0] + widths[1] + widths[2] + 2, 26);
        assert!(widths[1] >= widths[0], "{widths:?}");
        let widths = resolve_column_widths(&sample_plans(), 9, &all(3));
        assert_eq!(widths[2], 0, "the right-most column hides last-resort");
        assert_eq!(widths[0] + widths[1] + 1, 9);
        assert!(widths[0] >= 3 && widths[1] >= 3);
    }

    #[test]
    fn dragged_widths_are_kept_clamped_to_the_floor_and_shrunk_last() {
        let mut plans = sample_plans();
        plans[0].user = Some(15);
        assert_eq!(resolve_column_widths(&plans, 100, &all(3)), vec![15, 77, 6]);
        plans[0].user = Some(1);
        plans[2].user = Some(2);
        assert_eq!(resolve_column_widths(&plans, 100, &all(3)), vec![3, 89, 6]);

        let mut plans = sample_plans();
        plans[0].user = Some(10);
        assert_eq!(resolve_column_widths(&plans, 26, &all(3)), vec![10, 8, 6]);
        let widths = resolve_column_widths(&plans, 16, &all(3));
        assert_eq!(widths.iter().sum::<u16>() + 2, 16);
        assert!(widths[0] <= 10 && widths[1] == 3, "{widths:?}");

        let mut plans = sample_plans();
        plans[1].user = Some(12);
        assert_eq!(resolve_column_widths(&plans, 100, &all(3)), vec![10, 12, 6]);
    }

    #[test]
    fn a_huge_dragged_width_is_capped_at_512_and_resolves_at_once() {
        let started = std::time::Instant::now();
        let mut plans = sample_plans();
        plans[1].user = Some(u16::MAX);
        assert_eq!(
            resolve_column_widths(&plans, 2_000, &all(3)),
            vec![10, MAX_COLUMN_WIDTH, 6]
        );
        let mut many = vec![plan(10, ColumnFit::Shrink, Some(u16::MAX)); MAX_TABLE_COLUMNS];
        many[MAX_TABLE_COLUMNS - 1].fit = ColumnFit::Grow;
        let widths = resolve_column_widths(&many, 100, &all(MAX_TABLE_COLUMNS));
        assert!(widths.iter().sum::<u16>() + (MAX_TABLE_COLUMNS as u16 - 1) <= 100);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn disabled_columns_get_zero_width_and_free_their_room() {
        let widths = resolve_column_widths(&sample_plans(), 100, &[true, false, true]);
        assert_eq!(widths[1], 0);
        assert_eq!(widths[0] + widths[2] + 1, 10 + 6 + 1);
        // Grow column disabled: nobody absorbs the slack.
        assert_eq!(widths, vec![10, 0, 6]);
    }

    #[test]
    fn sorting_rows_follows_typed_keys_and_capture_order_when_unsorted() {
        let ui = Ui::new(Some(std::path::PathBuf::from("tui.toml")));
        let make = |ids: &[(u64, i64)]| -> Vec<TableRow> {
            ids.iter()
                .map(|(id, value)| TableRow {
                    id: *id,
                    cells: vec![vec![Span::raw(value.to_string())]],
                    keys: vec![SortKey::Num(*value)],
                    style: Style::default(),
                })
                .collect()
        };
        let unsorted = sort_rows(&ui, CardId::ConnTable, make(&[(1, 30), (2, 10), (3, 20)]));
        assert_eq!(
            unsorted.iter().map(|row| row.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let mut sorted_ui = Ui::new(Some(std::path::PathBuf::from("tui.toml")));
        sorted_ui.sorts[CardId::ConnTable as usize] = Some(crate::tui::sort::SortState {
            column: 0,
            descending: true,
        });
        let sorted = sort_rows(
            &sorted_ui,
            CardId::ConnTable,
            make(&[(1, 30), (2, 10), (3, 20)]),
        );
        assert_eq!(
            sorted.iter().map(|row| row.id).collect::<Vec<_>>(),
            [1, 3, 2]
        );
    }
}
