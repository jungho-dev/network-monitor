//! Shared interaction state: focus and hover, the card long press with an
//! injected clock, split and column drags, scrollbar drags, sorting, view
//! scrolling, toasts, and the language. Both TUIs embed one `Ui` and map its
//! `MouseOutcome` onto their own state.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::cards::{
    CARD_COUNT, CARD_HOLD_TIME, CardId, CardKind, CardMove, CardPress, CardView, ColumnDrag,
    CursorMotion, HitAreas, MAX_TABLE_COLUMNS, ScrollDrag, UiAction, scroll_for_thumb,
    scroll_thumb,
};
use super::columns::card_sort_kinds;
use super::i18n::{self, Lang, Msg};
use super::layout::{CardLayout, SplitAxis, SplitDrag, SplitGroup};
use super::sort::{Sort, SortChange, apply_change};

// Lines per wheel notch, double-click window on a column edge, toast lifetime.
const WHEEL_STEP: isize = 3;
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
pub const TOAST_LIFETIME: Duration = Duration::from_secs(4);

// 1. toast and outcome types --------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: ToastLevel,
    pub expires_at: Instant,
}

// What a mouse event asks the host to do; view state it can change itself
// (focus, hover, drags, wheel) is already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseOutcome {
    None,
    SaveState,
    Action(UiAction),
    Tab(usize),
    Header { card: CardId, column: usize },
    Row { card: CardId, index: usize },
    Scrolled(CardId),
}

// Hover targets that decide whether a mouse move needs a redraw.
pub type HoverSignature = (
    Option<CardId>,
    Option<(SplitGroup, usize)>,
    Option<(CardId, usize)>,
    Option<CardId>,
);

// 2. shared state -------------------------------------------------------------
pub struct Ui {
    pub lang: Lang,
    pub layout: CardLayout,
    pub column_widths: [[Option<u16>; MAX_TABLE_COLUMNS]; CARD_COUNT],
    pub sticky_widths: [[u16; MAX_TABLE_COLUMNS]; CARD_COUNT],
    pub sorts: [Sort; CARD_COUNT],
    pub views: [CardView; CARD_COUNT],
    pub selected_keys: [Option<u64>; CARD_COUNT],
    pub row_keys: [Vec<u64>; CARD_COUNT],
    pub hits: HitAreas,
    pub context: usize,
    pub default_focus: CardId,
    pub focus: Option<(usize, CardId)>,
    pub hover_card: Option<CardId>,
    pub split_hover: Option<(SplitGroup, usize)>,
    pub column_hover: Option<(CardId, usize)>,
    pub scroll_hover: Option<CardId>,
    pub card_press: Option<CardPress>,
    pub card_move: Option<CardMove>,
    pub split_drag: Option<SplitDrag>,
    pub column_drag: Option<ColumnDrag>,
    pub scroll_drag: Option<ScrollDrag>,
    pub last_column_click: Option<(CardId, usize, Instant)>,
    pub toast: Option<Toast>,
    pub show_help: bool,
    // None without a home directory: state saving is then off.
    pub state_file: Option<PathBuf>,
}

impl Ui {
    pub fn new(state_file: Option<PathBuf>) -> Self {
        Self {
            lang: Lang::default(),
            layout: CardLayout::default(),
            column_widths: [[None; MAX_TABLE_COLUMNS]; CARD_COUNT],
            sticky_widths: [[0; MAX_TABLE_COLUMNS]; CARD_COUNT],
            sorts: [None; CARD_COUNT],
            views: [CardView::default(); CARD_COUNT],
            selected_keys: [None; CARD_COUNT],
            row_keys: std::array::from_fn(|_| Vec::new()),
            hits: HitAreas::default(),
            context: 0,
            default_focus: CardId::PacketList,
            focus: None,
            hover_card: None,
            split_hover: None,
            column_hover: None,
            scroll_hover: None,
            card_press: None,
            card_move: None,
            split_drag: None,
            column_drag: None,
            scroll_drag: None,
            last_column_click: None,
            toast: None,
            show_help: false,
            state_file,
        }
    }

    // 3. focus, toast, language -----------------------------------------------
    // The clicked card counts only in the context it was clicked in.
    pub fn focused_card(&self) -> CardId {
        match self.focus {
            Some((context, card)) if context == self.context => card,
            _ => self.default_focus,
        }
    }

    // Tab and Shift+Tab: next or previous card drawn this frame.
    pub fn cycle_focus(&mut self, delta: isize) {
        let count = self.hits.card_rects.len();
        if count == 0 {
            return;
        }
        let current = self.focused_card();
        let position = self
            .hits
            .card_rects
            .iter()
            .position(|(card, _)| *card == current)
            .unwrap_or(0);
        let next = (position as isize + delta).rem_euclid(count as isize) as usize;
        let card = self.hits.card_rects[next].0;
        self.set_focus(card);
    }

    pub fn set_focus(&mut self, card: CardId) {
        self.focus = Some((self.context, card));
    }

    pub fn show_toast(&mut self, text: &str, level: ToastLevel) {
        self.toast = Some(Toast {
            text: text.to_string(),
            level,
            expires_at: Instant::now() + TOAST_LIFETIME,
        });
    }

    // True when a toast expired and the screen needs a redraw.
    pub fn expire_toast(&mut self, now: Instant) -> bool {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| now >= toast.expires_at)
        {
            self.toast = None;
            return true;
        }
        false
    }

    pub fn toggle_language(&mut self) {
        self.lang = self.lang.toggled();
    }

    // Default order, sizes, and column widths for the listed groups and cards;
    // the language stays.
    pub fn reset_layout(&mut self, groups: &[SplitGroup], cards: &[CardId]) {
        self.layout.reset_groups(groups);
        for card in cards {
            self.column_widths[*card as usize] = [None; MAX_TABLE_COLUMNS];
            self.sticky_widths[*card as usize] = [0; MAX_TABLE_COLUMNS];
        }
        self.cancel_card_move();
        self.show_toast(
            i18n::text(self.lang, Msg::ToastLayoutReset),
            ToastLevel::Info,
        );
    }

    pub fn hover_signature(&self) -> HoverSignature {
        (
            self.hover_card,
            self.split_hover,
            self.column_hover,
            self.scroll_hover,
        )
    }

    pub fn arranging(&self) -> bool {
        self.card_press.is_some() || self.card_move.is_some()
    }

    // A split, column, or scrollbar drag is open (its release has not arrived).
    pub fn dragging(&self) -> bool {
        self.split_drag.is_some() || self.column_drag.is_some() || self.scroll_drag.is_some()
    }

    // Ends every open drag without a button release: a lost release, a new
    // press, or a tab switch. True when a dragged boundary or width had moved
    // and must be saved.
    pub fn commit_drags(&mut self) -> bool {
        let split_moved = self.split_drag.take().is_some_and(|drag| drag.moved);
        let column_moved = self.column_drag.take().is_some_and(|drag| drag.moved);
        self.scroll_drag = None;
        split_moved || column_moved
    }

    // 4. view state -----------------------------------------------------------
    // Records the card's size and keeps scroll and selection inside it.
    pub fn clamp_view(&mut self, card: CardId, total: usize, visible: usize) {
        let view = &mut self.views[card as usize];
        view.total = total;
        view.visible = visible;
        view.scroll = view.scroll.min(total.saturating_sub(visible));
        view.selected = view.selected.min(total.saturating_sub(1));
    }

    // Moves the view only (the selected row stays); true when it changed.
    pub fn scroll_by(&mut self, card: CardId, delta: isize) -> bool {
        let view = &mut self.views[card as usize];
        let max_scroll = view.total.saturating_sub(view.visible);
        let target = view.scroll.saturating_add_signed(delta).min(max_scroll);
        let changed = target != view.scroll;
        view.scroll = target;
        changed
    }

    pub fn set_scroll(&mut self, card: CardId, scroll: usize) {
        let view = &mut self.views[card as usize];
        view.scroll = scroll.min(view.total.saturating_sub(view.visible));
    }

    // Selects a row and scrolls only as far as needed to show it.
    pub fn select_row(&mut self, card: CardId, index: usize) {
        let view = &mut self.views[card as usize];
        if view.total == 0 {
            view.selected = 0;
            return;
        }
        view.selected = index.min(view.total - 1);
        if view.selected < view.scroll {
            view.scroll = view.selected;
        } else if view.visible > 0 && view.selected >= view.scroll + view.visible {
            view.scroll = view.selected + 1 - view.visible;
        }
        let key = self.row_keys[card as usize].get(self.views[card as usize].selected);
        self.selected_keys[card as usize] = key.copied();
    }

    // Records the table's row ids in display order. Once the user has chosen a
    // row, the selection stays on that item when a re-sort or refresh moves it,
    // and the view scrolls only when the item itself moved.
    pub fn sync_selection(&mut self, card: CardId, ids: Vec<u64>) {
        let slot = card as usize;
        if let Some(key) = self.selected_keys[slot]
            && let Some(position) = ids.iter().position(|id| *id == key)
        {
            let view = &mut self.views[slot];
            if position != view.selected {
                view.selected = position;
                if position < view.scroll {
                    view.scroll = position;
                } else if view.visible > 0 && position >= view.scroll + view.visible {
                    view.scroll = position + 1 - view.visible;
                }
            }
        }
        self.row_keys[slot] = ids;
    }

    // Keyboard motion: a selectable card moves its selected row, any other card
    // scrolls. Returns whether the view or selection changed.
    pub fn move_cursor(&mut self, card: CardId, motion: CursorMotion) -> bool {
        let view = self.views[card as usize];
        let page = view.visible.max(1) as isize;
        let last = view.total.saturating_sub(1);
        if card.kind() == CardKind::Selectable {
            let target = match motion {
                CursorMotion::Lines(delta) => view.selected.saturating_add_signed(delta),
                CursorMotion::Pages(pages) => view.selected.saturating_add_signed(pages * page),
                CursorMotion::Home => 0,
                CursorMotion::End => last,
            }
            .min(last);
            let before = (view.selected, view.scroll);
            self.select_row(card, target);
            let after = &self.views[card as usize];
            return before != (after.selected, after.scroll);
        }
        match motion {
            CursorMotion::Lines(delta) => self.scroll_by(card, delta),
            CursorMotion::Pages(pages) => self.scroll_by(card, pages * page),
            CursorMotion::Home => self.scroll_by(card, isize::MIN / 2),
            CursorMotion::End => self.scroll_by(card, isize::MAX / 2),
        }
    }

    // Applies a sort change to a table card; true when the order changed.
    pub fn change_sort(&mut self, card: CardId, change: SortChange) -> bool {
        let kinds = card_sort_kinds(card);
        let before = self.sorts[card as usize];
        self.sorts[card as usize] = apply_change(before, change, &kinds);
        self.sorts[card as usize] != before
    }

    // 5. long press and card move ---------------------------------------------
    // Enters move mode once the title was held for CARD_HOLD_TIME, whatever the
    // mouse did meanwhile; true when the state changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(press) = self.card_press else {
            return false;
        };
        if now.saturating_duration_since(press.started) < CARD_HOLD_TIME {
            return false;
        }
        self.card_press = None;
        let Some(slot) = self.layout.slot_of(press.group, press.index) else {
            return false;
        };
        self.card_move = Some(CardMove {
            card: press.card,
            group: press.group,
            index: press.index,
            baseline: self.layout.order_of(press.group),
            target: slot,
        });
        true
    }

    // Time left before move mode starts, to cap the event wait.
    pub fn press_remaining(&self, now: Instant) -> Option<Duration> {
        self.card_press.map(|press| {
            CARD_HOLD_TIME.saturating_sub(now.saturating_duration_since(press.started))
        })
    }

    // Drops a pending press and restores the order from before the press.
    pub fn cancel_card_move(&mut self) -> bool {
        let pressed = self.card_press.take().is_some();
        match self.card_move.take() {
            Some(moving) => {
                self.layout.set_order(moving.group, moving.baseline);
                true
            }
            None => pressed,
        }
    }

    // Re-computes the insertion from the baseline for the slot under the pointer.
    fn preview_card_move(&mut self, position: Position) {
        let Some(moving) = self.card_move else {
            return;
        };
        let target = self
            .hits
            .slot_groups
            .iter()
            .find(|slots| slots.group == moving.group)
            .and_then(|slots| slots.slots.iter().position(|slot| slot.contains(position)));
        let Some(target) = target else {
            return;
        };
        if target == moving.target {
            return;
        }
        self.layout
            .move_card_to_slot(moving.group, &moving.baseline, moving.index, target);
        self.card_move = Some(CardMove { target, ..moving });
    }

    // True (save needed) only when the order actually changed.
    pub fn commit_card_move(&mut self) -> bool {
        let Some(moving) = self.card_move.take() else {
            return false;
        };
        if self.layout.order_of(moving.group) == moving.baseline {
            return false;
        }
        self.show_toast(i18n::text(self.lang, Msg::ToastCardMoved), ToastLevel::Info);
        true
    }

    // Shift+arrow: one slot along the group's axis; only groups drawn this frame.
    pub fn shift_card(&mut self, card: CardId, code: KeyCode) -> bool {
        let Some((group, index)) = card.reorder_slot() else {
            return false;
        };
        if !self
            .hits
            .slot_groups
            .iter()
            .any(|slots| slots.group == group)
        {
            return false;
        }
        let delta: isize = match (group.axis(), code) {
            (SplitAxis::Rows, KeyCode::Up) | (SplitAxis::Columns, KeyCode::Left) => -1,
            (SplitAxis::Rows, KeyCode::Down) | (SplitAxis::Columns, KeyCode::Right) => 1,
            _ => return false,
        };
        let Some(slot) = self.layout.slot_of(group, index) else {
            return false;
        };
        let Some(target) = slot
            .checked_add_signed(delta)
            .filter(|target| *target < group.card_count())
        else {
            return false;
        };
        let baseline = self.layout.order_of(group);
        self.layout
            .move_card_to_slot(group, &baseline, index, target);
        self.show_toast(i18n::text(self.lang, Msg::ToastCardMoved), ToastLevel::Info);
        true
    }

    // Every rect moved: drop hover, drags, and any card move; true when a
    // dragged boundary or width had changed and must be saved.
    pub fn handle_resize(&mut self) -> bool {
        self.cancel_card_move();
        let split_moved = self.split_drag.take().is_some_and(|drag| drag.moved);
        let column_moved = self.column_drag.take().is_some_and(|drag| drag.moved);
        self.split_hover = None;
        self.column_hover = None;
        self.hover_card = None;
        self.scroll_hover = None;
        self.scroll_drag = None;
        split_moved || column_moved
    }

    // 6. mouse ----------------------------------------------------------------
    pub fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) -> MouseOutcome {
        // A hold that ended without any motion still enters move mode first.
        self.tick(now);

        if self.show_help && !matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_))
        {
            return MouseOutcome::None;
        }
        // In move mode a buttonless move or a new press means the release was
        // lost: commit the preview. The wheel is ignored.
        if self.card_move.is_some() {
            match mouse.kind {
                MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left) => {
                    return self.commit_outcome();
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    return MouseOutcome::None;
                }
                _ => {}
            }
        }
        let position = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Moved => {
                // A buttonless move while a drag is open means its release was
                // lost: the drag ends here like an Up.
                self.card_press = None;
                let saved = self.commit_drags();
                self.track_hover(position);
                if saved {
                    MouseOutcome::SaveState
                } else {
                    MouseOutcome::None
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => self.drag_left(position),
            MouseEventKind::Up(MouseButton::Left) => self.release_left(now),
            MouseEventKind::ScrollUp => self.wheel_card(position, -WHEEL_STEP),
            MouseEventKind::ScrollDown => self.wheel_card(position, WHEEL_STEP),
            MouseEventKind::Down(MouseButton::Left) => self.press_left(position, now),
            _ => MouseOutcome::None,
        }
    }

    fn commit_outcome(&mut self) -> MouseOutcome {
        if self.commit_card_move() {
            MouseOutcome::SaveState
        } else {
            MouseOutcome::None
        }
    }

    fn drag_left(&mut self, position: Position) -> MouseOutcome {
        if self.card_move.is_some() {
            self.preview_card_move(position);
        } else if let Some(press) = self.card_press {
            // Another cell before the hold time cancels; jitter inside the cell keeps it.
            if position != press.cell {
                self.card_press = None;
            }
        } else if self.scroll_drag.is_some() {
            return self.drag_scrollbar(position);
        } else if self.column_drag.is_some() {
            self.drag_column(position);
        } else {
            self.drag_split(position);
        }
        MouseOutcome::None
    }

    fn release_left(&mut self, now: Instant) -> MouseOutcome {
        self.card_press = None;
        if self.card_move.is_some() {
            return self.commit_outcome();
        }
        let split_moved = self.split_drag.take().is_some_and(|drag| drag.moved);
        self.scroll_drag = None;
        let mut column_moved = false;
        if let Some(drag) = self.column_drag.take() {
            column_moved = drag.moved;
            // Only an edge click without a drag counts toward a double click.
            self.last_column_click = if drag.moved {
                None
            } else {
                Some((drag.card, drag.column, now))
            };
        }
        if split_moved || column_moved {
            MouseOutcome::SaveState
        } else {
            MouseOutcome::None
        }
    }

    // Scrollbar track first, then column edge and split boundary, then the card.
    fn track_hover(&mut self, position: Position) {
        if self.show_help {
            return;
        }
        let track_card = self
            .hits
            .scroll_hits
            .iter()
            .find(|hit| hit.track.contains(position))
            .map(|hit| hit.card);
        if track_card.is_some() {
            self.scroll_hover = track_card;
            self.hover_card = track_card;
            self.split_hover = None;
            self.column_hover = None;
            return;
        }
        self.scroll_hover = None;
        self.column_hover = self
            .hits
            .column_hits
            .iter()
            .find(|hit| hit.grab.contains(position))
            .map(|hit| (hit.card, hit.column));
        let split = self
            .hits
            .split_hits
            .iter()
            .find(|hit| hit.grab.contains(position))
            .map(|hit| (hit.group, hit.index));
        self.hover_card = if split.is_some() {
            None
        } else {
            self.hits
                .card_rects
                .iter()
                .find(|(_, rect)| rect.contains(position))
                .map(|(card, _)| *card)
        };
        self.split_hover = split;
    }

    fn drag_split(&mut self, position: Position) {
        let Some(drag) = self.split_drag else {
            return;
        };
        let grabbed = self
            .hits
            .split_hits
            .iter()
            .find(|hit| hit.group == drag.group && hit.index == drag.index)
            .copied();
        let Some(hit) = grabbed else {
            self.split_drag = None;
            return;
        };
        let coord = match hit.axis {
            SplitAxis::Columns => i32::from(position.x),
            SplitAxis::Rows => i32::from(position.y),
        };
        if let Some((before, after)) = self.layout.drag_cut(&hit, coord - drag.grip) {
            self.split_drag = Some(SplitDrag {
                moved: true,
                ..drag
            });
            let text = i18n::card_resized(self.lang, hit.axis, before, after);
            self.show_toast(&text, ToastLevel::Info);
        }
    }

    fn drag_scrollbar(&mut self, position: Position) -> MouseOutcome {
        let Some(drag) = self.scroll_drag else {
            return MouseOutcome::None;
        };
        let grabbed = self
            .hits
            .scroll_hits
            .iter()
            .find(|hit| hit.card == drag.card)
            .copied();
        let Some(hit) = grabbed else {
            self.scroll_drag = None;
            return MouseOutcome::None;
        };
        let thumb_top = i32::from(position.y) - i32::from(hit.track.y) - i32::from(drag.grip);
        let thumb_start = u16::try_from(thumb_top.max(0)).unwrap_or(u16::MAX);
        let scroll = scroll_for_thumb(hit.total, hit.visible, hit.track.height, thumb_start);
        self.set_scroll(drag.card, scroll);
        MouseOutcome::Scrolled(drag.card)
    }

    fn drag_column(&mut self, position: Position) {
        let Some(drag) = self.column_drag else {
            return;
        };
        let grabbed = self
            .hits
            .column_hits
            .iter()
            .find(|hit| hit.card == drag.card && hit.column == drag.column)
            .copied();
        let Some(hit) = grabbed else {
            self.column_drag = None;
            return;
        };
        if !drag.moved && position.x == drag.start_x {
            return;
        }
        let wanted = i32::from(drag.start_width) + i32::from(position.x) - i32::from(drag.start_x);
        let width = wanted.clamp(i32::from(hit.min), i32::from(hit.max.max(hit.min))) as u16;
        self.column_widths[drag.card as usize][drag.column] = Some(width);
        self.column_drag = Some(ColumnDrag {
            moved: true,
            ..drag
        });
        let text = i18n::column_width(self.lang, width);
        self.show_toast(&text, ToastLevel::Info);
    }

    // The wheel scrolls the card under the pointer.
    fn wheel_card(&mut self, position: Position, delta: isize) -> MouseOutcome {
        let card = self
            .hits
            .card_rects
            .iter()
            .find(|(_, rect)| rect.contains(position))
            .map(|(card, _)| *card);
        match card {
            Some(card) if self.scroll_by(card, delta) => MouseOutcome::Scrolled(card),
            _ => MouseOutcome::None,
        }
    }

    // Priority: help close, scrollbar, card title, split boundary, column edge,
    // card focus, chip, tab, header, row.
    fn press_left(&mut self, position: Position, now: Instant) -> MouseOutcome {
        if self.show_help {
            self.show_help = false;
            return MouseOutcome::None;
        }
        self.cancel_card_move();

        // A press with a drag still open means its release was lost: every
        // drag state resets first, and a drag that had moved is saved and the
        // press is consumed (like a lost card drop).
        if self.commit_drags() {
            return MouseOutcome::SaveState;
        }

        // 1. scrollbar: thumb grab or track page
        let track_hit = self
            .hits
            .scroll_hits
            .iter()
            .find(|hit| hit.track.contains(position))
            .copied();
        if let Some(hit) = track_hit {
            self.set_focus(hit.card);
            let offset = position.y - hit.track.y;
            let page = hit.visible as isize;
            if let Some(thumb) = scroll_thumb(hit.total, hit.visible, hit.scroll, hit.track.height)
            {
                if offset < thumb.start {
                    self.scroll_by(hit.card, -page);
                    return MouseOutcome::Scrolled(hit.card);
                }
                if offset >= thumb.start + thumb.len {
                    self.scroll_by(hit.card, page);
                    return MouseOutcome::Scrolled(hit.card);
                }
                self.scroll_drag = Some(ScrollDrag {
                    card: hit.card,
                    grip: offset - thumb.start,
                });
            }
            return MouseOutcome::None;
        }

        // 2. card title of a group drawn this frame: wait for the hold time
        let title_card = self
            .hits
            .title_hits
            .iter()
            .find(|hit| hit.rect.contains(position))
            .map(|hit| hit.card);
        if let Some(card) = title_card
            && let Some((group, index)) = card.reorder_slot()
            && self
                .hits
                .slot_groups
                .iter()
                .any(|slots| slots.group == group)
        {
            self.set_focus(card);
            self.card_press = Some(CardPress {
                card,
                group,
                index,
                started: now,
                cell: position,
            });
            return MouseOutcome::None;
        }

        // 3. split boundary
        self.split_drag = None;
        let grabbed = self
            .hits
            .split_hits
            .iter()
            .find(|hit| hit.grab.contains(position))
            .copied();
        if let Some(hit) = grabbed {
            let coord = match hit.axis {
                SplitAxis::Columns => position.x,
                SplitAxis::Rows => position.y,
            };
            let cut_at = hit.origin + hit.cuts[hit.index];
            self.split_drag = Some(SplitDrag {
                group: hit.group,
                index: hit.index,
                grip: i32::from(coord) - i32::from(cut_at),
                moved: false,
            });
            self.split_hover = Some((hit.group, hit.index));
            self.show_toast(i18n::text(self.lang, Msg::ToastSplitDrag), ToastLevel::Info);
            return MouseOutcome::None;
        }

        // 4. column edge: a second press right after an undragged one resets to auto
        self.column_drag = None;
        let column_hit = self
            .hits
            .column_hits
            .iter()
            .find(|hit| hit.grab.contains(position))
            .copied();
        if let Some(hit) = column_hit {
            self.set_focus(hit.card);
            let double_click = self.last_column_click.is_some_and(|(card, column, at)| {
                card == hit.card
                    && column == hit.column
                    && now.saturating_duration_since(at) < DOUBLE_CLICK_WINDOW
            });
            self.last_column_click = None;
            if double_click {
                self.column_widths[hit.card as usize] = [None; MAX_TABLE_COLUMNS];
                self.show_toast(
                    i18n::text(self.lang, Msg::ToastColumnsAuto),
                    ToastLevel::Info,
                );
                return MouseOutcome::SaveState;
            }
            self.column_drag = Some(ColumnDrag {
                card: hit.card,
                column: hit.column,
                start_x: position.x,
                start_width: hit.width,
                moved: false,
            });
            self.column_hover = Some((hit.card, hit.column));
            self.show_toast(
                i18n::text(self.lang, Msg::ToastColumnDrag),
                ToastLevel::Info,
            );
            return MouseOutcome::None;
        }

        // 5. card focus, then chip, tab, header, row
        let clicked_card = self
            .hits
            .card_rects
            .iter()
            .find(|(_, rect)| rect.contains(position))
            .map(|(card, _)| *card);
        if let Some(card) = clicked_card {
            self.set_focus(card);
        }
        let chip = self
            .hits
            .button_rects
            .iter()
            .find(|(_, rect)| rect.contains(position))
            .map(|(action, _)| *action);
        if let Some(action) = chip {
            return MouseOutcome::Action(action);
        }
        let tab = self
            .hits
            .tab_rects
            .iter()
            .find(|(_, rect)| rect.contains(position))
            .map(|(tab, _)| *tab);
        if let Some(tab) = tab {
            return MouseOutcome::Tab(tab);
        }
        let header = self
            .hits
            .header_cells
            .iter()
            .find(|hit| hit.rect.contains(position))
            .map(|hit| (hit.card, hit.column));
        if let Some((card, column)) = header {
            return MouseOutcome::Header { card, column };
        }
        let row_hit = self
            .hits
            .row_hits
            .iter()
            .find(|hit| hit.rect.contains(position))
            .copied();
        if let Some(hit) = row_hit {
            let index = hit.offset + usize::from(position.y - hit.rect.y);
            if index < hit.len {
                return MouseOutcome::Row {
                    card: hit.card,
                    index,
                };
            }
        }
        MouseOutcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::cards::{ColumnHit, ScrollHit, SlotGroupHit, TitleHit};
    use crate::tui::layout::IDENTITY_ORDER;
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Rect;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn after(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    // Two side-by-side cards drawn in slots 0 and 1 with titles on the top border.
    fn arranged_ui() -> Ui {
        let mut ui = Ui::new(Some(PathBuf::from("tui.toml")));
        let slots = vec![Rect::new(0, 2, 40, 10), Rect::new(41, 2, 39, 10)];
        ui.hits.slot_groups.push(SlotGroupHit {
            group: SplitGroup::CaptureCols,
            slots: slots.clone(),
        });
        ui.hits.title_hits.push(TitleHit {
            card: CardId::PacketList,
            rect: Rect::new(1, 2, 10, 1),
        });
        ui.hits.title_hits.push(TitleHit {
            card: CardId::PacketDetail,
            rect: Rect::new(42, 2, 10, 1),
        });
        ui.hits.card_rects.push((CardId::PacketList, slots[0]));
        ui.hits.card_rects.push((CardId::PacketDetail, slots[1]));
        ui
    }

    fn down(ui: &mut Ui, column: u16, row: u16, now: Instant) -> MouseOutcome {
        ui.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), column, row),
            now,
        )
    }

    fn drag(ui: &mut Ui, column: u16, row: u16, now: Instant) -> MouseOutcome {
        ui.handle_mouse(
            mouse(MouseEventKind::Drag(MouseButton::Left), column, row),
            now,
        )
    }

    fn up(ui: &mut Ui, column: u16, row: u16, now: Instant) -> MouseOutcome {
        ui.handle_mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            now,
        )
    }

    fn moved(ui: &mut Ui, column: u16, row: u16, now: Instant) -> MouseOutcome {
        ui.handle_mouse(mouse(MouseEventKind::Moved, column, row), now)
    }

    fn order(ui: &Ui) -> [u8; 2] {
        let slots = ui.layout.order_of(SplitGroup::CaptureCols);
        [slots[0], slots[1]]
    }

    #[test]
    fn title_press_released_before_400ms_is_a_plain_click_that_only_focuses() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        assert_eq!(down(&mut ui, 3, 2, start), MouseOutcome::None);
        assert!(ui.card_press.is_some());
        assert_eq!(ui.focused_card(), CardId::PacketList);
        assert_eq!(up(&mut ui, 3, 2, after(start, 399)), MouseOutcome::None);
        assert!(ui.card_press.is_none() && ui.card_move.is_none());
        assert_eq!(order(&ui), [0, 1]);
    }

    #[test]
    fn title_hold_enters_move_mode_at_exactly_400ms_without_any_motion() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        assert!(!ui.tick(after(start, 399)));
        assert!(ui.card_move.is_none());
        assert_eq!(
            ui.press_remaining(after(start, 100)),
            Some(Duration::from_millis(300))
        );
        assert!(ui.tick(after(start, 400)));
        let moving = ui.card_move.expect("move mode");
        assert_eq!((moving.card, moving.target), (CardId::PacketList, 0));
        assert!(ui.card_press.is_none());
        assert_eq!(ui.press_remaining(after(start, 500)), None);
    }

    #[test]
    fn first_event_after_the_hold_time_starts_the_move_even_without_a_tick() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        // The drag at 450 ms is the first event after the hold: move mode starts
        // before it is handled, so the preview already applies.
        drag(&mut ui, 60, 5, after(start, 450));
        assert!(ui.card_move.is_some());
        assert_eq!(order(&ui), [1, 0]);
    }

    #[test]
    fn dragging_to_another_cell_before_the_hold_time_cancels_the_press() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        drag(&mut ui, 4, 2, after(start, 100));
        assert!(ui.card_press.is_none());
        ui.tick(after(start, 500));
        assert!(ui.card_move.is_none());
        assert_eq!(order(&ui), [0, 1]);
    }

    #[test]
    fn same_cell_jitter_keeps_the_press_while_another_cell_cancels_it() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        drag(&mut ui, 3, 2, after(start, 50));
        drag(&mut ui, 3, 2, after(start, 120));
        assert!(ui.card_press.is_some());
        assert!(ui.tick(after(start, 400)));
        assert!(ui.card_move.is_some());
    }

    #[test]
    fn dragging_in_move_mode_previews_the_new_order_and_release_commits_and_saves() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        assert!(ui.tick(after(start, 400)));
        drag(&mut ui, 60, 5, after(start, 450));
        assert_eq!(order(&ui), [1, 0], "live preview");
        assert!(ui.card_move.is_some());
        assert_eq!(
            up(&mut ui, 60, 5, after(start, 500)),
            MouseOutcome::SaveState
        );
        assert_eq!(order(&ui), [1, 0]);
        assert!(ui.card_move.is_none());
        assert!(ui.toast.is_some());
    }

    #[test]
    fn releasing_on_the_original_slot_changes_nothing_and_requests_no_save() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        drag(&mut ui, 60, 5, after(start, 450));
        drag(&mut ui, 10, 5, after(start, 480));
        assert_eq!(up(&mut ui, 10, 5, after(start, 500)), MouseOutcome::None);
        assert_eq!(ui.layout.order_of(SplitGroup::CaptureCols), IDENTITY_ORDER);
    }

    #[test]
    fn escape_restores_the_order_from_before_the_press() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        drag(&mut ui, 60, 5, after(start, 450));
        assert_eq!(order(&ui), [1, 0]);
        assert!(ui.cancel_card_move());
        assert_eq!(order(&ui), [0, 1]);
        assert!(ui.card_move.is_none());
        assert_eq!(up(&mut ui, 60, 5, after(start, 500)), MouseOutcome::None);
        assert_eq!(order(&ui), [0, 1]);
    }

    #[test]
    fn a_lost_release_is_committed_by_the_first_buttonless_move_and_saved() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        drag(&mut ui, 60, 5, after(start, 450));
        assert_eq!(
            moved(&mut ui, 62, 6, after(start, 600)),
            MouseOutcome::SaveState
        );
        assert!(ui.card_move.is_none());
        assert_eq!(order(&ui), [1, 0]);
    }

    #[test]
    fn a_buttonless_move_without_a_change_commits_nothing_and_requests_no_save() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        assert_eq!(moved(&mut ui, 5, 6, after(start, 450)), MouseOutcome::None);
        assert!(ui.card_move.is_none());
        assert_eq!(order(&ui), [0, 1]);
    }

    #[test]
    fn a_new_press_in_move_mode_commits_the_preview_and_is_consumed_not_run_as_a_click() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        drag(&mut ui, 60, 5, after(start, 450));
        let outcome = down(&mut ui, 60, 5, after(start, 500));
        assert_eq!(outcome, MouseOutcome::SaveState);
        assert!(ui.card_move.is_none() && ui.card_press.is_none());
        assert_eq!(order(&ui), [1, 0]);
    }

    #[test]
    fn moved_during_a_pending_press_ends_it_as_a_plain_click() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        moved(&mut ui, 3, 2, after(start, 100));
        assert!(ui.card_press.is_none());
        ui.tick(after(start, 500));
        assert!(ui.card_move.is_none());
    }

    #[test]
    fn wheel_and_other_events_are_ignored_while_a_card_is_moving() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        ui.views[CardId::PacketList as usize].total = 100;
        ui.views[CardId::PacketList as usize].visible = 10;
        let outcome = ui.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5), after(start, 450));
        assert_eq!(outcome, MouseOutcome::None);
        assert_eq!(ui.views[CardId::PacketList as usize].scroll, 0);
        assert!(ui.card_move.is_some());
    }

    #[test]
    fn resize_cancels_the_move_and_clears_hover_and_drags() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        down(&mut ui, 3, 2, start);
        ui.tick(after(start, 400));
        drag(&mut ui, 60, 5, after(start, 450));
        ui.hover_card = Some(CardId::PacketList);
        assert!(!ui.handle_resize());
        assert_eq!(order(&ui), [0, 1]);
        assert!(ui.card_move.is_none() && ui.hover_card.is_none());
    }

    #[test]
    fn shift_arrows_move_the_card_along_the_axis_only_for_groups_drawn_this_frame() {
        let mut ui = arranged_ui();
        assert!(!ui.shift_card(CardId::PacketList, KeyCode::Down));
        assert!(!ui.shift_card(CardId::HexDump, KeyCode::Right));
        assert!(ui.shift_card(CardId::PacketList, KeyCode::Right));
        assert_eq!(order(&ui), [1, 0]);
        assert!(
            !ui.shift_card(CardId::PacketList, KeyCode::Right),
            "already last"
        );
        assert!(ui.shift_card(CardId::PacketList, KeyCode::Left));
        assert_eq!(order(&ui), [0, 1]);

        let mut undrawn = Ui::new(Some(PathBuf::from("tui.toml")));
        assert!(!undrawn.shift_card(CardId::PacketList, KeyCode::Right));
    }

    #[test]
    fn scroll_and_selection_helpers_clamp_and_keep_the_selected_row_visible() {
        let mut ui = Ui::new(Some(PathBuf::from("tui.toml")));
        ui.clamp_view(CardId::PacketList, 100, 10);
        assert!(ui.move_cursor(CardId::PacketList, CursorMotion::Lines(12)));
        let view = ui.views[CardId::PacketList as usize];
        assert_eq!((view.selected, view.scroll), (12, 3));
        assert!(ui.move_cursor(CardId::PacketList, CursorMotion::End));
        assert_eq!(ui.views[CardId::PacketList as usize].selected, 99);
        assert_eq!(ui.views[CardId::PacketList as usize].scroll, 90);
        assert!(!ui.move_cursor(CardId::PacketList, CursorMotion::End));
        assert!(ui.move_cursor(CardId::PacketList, CursorMotion::Home));
        assert_eq!(ui.views[CardId::PacketList as usize].scroll, 0);

        // A scrollable card moves only its view.
        ui.clamp_view(CardId::PacketDetail, 30, 10);
        assert!(ui.move_cursor(CardId::PacketDetail, CursorMotion::Pages(1)));
        assert_eq!(ui.views[CardId::PacketDetail as usize].scroll, 10);
        assert!(ui.move_cursor(CardId::PacketDetail, CursorMotion::End));
        assert_eq!(ui.views[CardId::PacketDetail as usize].scroll, 20);
        assert!(!ui.scroll_by(CardId::PacketDetail, 5));
    }

    #[test]
    fn change_sort_reports_whether_the_order_changed_and_cycles_to_capture_order() {
        let mut ui = Ui::new(Some(PathBuf::from("tui.toml")));
        assert!(ui.change_sort(CardId::PacketList, SortChange::SelectColumn(6)));
        assert_eq!(
            ui.sorts[CardId::PacketList as usize].map(|s| s.descending),
            Some(true)
        );
        assert!(ui.change_sort(CardId::PacketList, SortChange::SelectColumn(6)));
        assert!(ui.change_sort(CardId::PacketList, SortChange::SelectColumn(6)));
        assert_eq!(ui.sorts[CardId::PacketList as usize], None);
        assert!(!ui.change_sort(CardId::PacketList, SortChange::SelectColumn(99)));
    }

    #[test]
    fn toast_expires_only_after_its_lifetime_and_language_toggles_both_ways() {
        let mut ui = Ui::new(Some(PathBuf::from("tui.toml")));
        ui.show_toast("hello", ToastLevel::Info);
        let expires = ui.toast.as_ref().unwrap().expires_at;
        assert!(!ui.expire_toast(expires - Duration::from_millis(1)));
        assert!(ui.expire_toast(expires));
        assert!(ui.toast.is_none());
        assert_eq!(ui.lang, Lang::En);
        ui.toggle_language();
        assert_eq!(ui.lang, Lang::Ko);
        ui.toggle_language();
        assert_eq!(ui.lang, Lang::En);
    }

    #[test]
    fn help_swallows_everything_but_a_press_that_closes_it() {
        let start = Instant::now();
        let mut ui = arranged_ui();
        ui.show_help = true;
        assert_eq!(
            ui.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5), start),
            MouseOutcome::None
        );
        assert!(ui.show_help);
        assert_eq!(down(&mut ui, 3, 2, start), MouseOutcome::None);
        assert!(!ui.show_help);
        assert!(ui.card_press.is_none(), "the closing press is consumed");
    }

    // A split boundary, a column edge, and a scrollbar thumb drawn in one frame.
    fn draggable_ui() -> Ui {
        let mut ui = Ui::new(Some(PathBuf::from("tui.toml")));
        let area = Rect::new(0, 2, 80, 30);
        ui.layout
            .split_cards(&mut ui.hits, SplitGroup::CaptureRows, area, &[15], &[3, 3]);
        ui.hits.column_hits.push(ColumnHit {
            card: CardId::PacketList,
            column: 2,
            grab: Rect::new(30, 3, 2, 1),
            line: Rect::new(31, 3, 1, 1),
            width: 10,
            min: 3,
            max: 40,
        });
        ui.hits.scroll_hits.push(ScrollHit {
            card: CardId::PacketList,
            track: Rect::new(78, 4, 2, 10),
            total: 100,
            visible: 10,
            scroll: 0,
        });
        ui.views[CardId::PacketList as usize].total = 100;
        ui.views[CardId::PacketList as usize].visible = 10;
        ui
    }

    const SPLIT_GRAB: (u16, u16) = (10, 17);
    const COLUMN_GRAB: (u16, u16) = (30, 3);
    const THUMB_GRAB: (u16, u16) = (78, 4);

    #[test]
    fn a_lost_release_ends_a_moved_split_drag_and_saves() {
        let start = Instant::now();
        let mut ui = draggable_ui();
        assert_eq!(
            down(&mut ui, SPLIT_GRAB.0, SPLIT_GRAB.1, start),
            MouseOutcome::None
        );
        assert!(ui.split_drag.is_some());
        drag(&mut ui, 10, 22, after(start, 50));
        assert!(ui.split_drag.is_some_and(|drag| drag.moved));
        assert!(ui.layout.custom_ratios(SplitGroup::CaptureRows).is_some());
        assert_eq!(
            moved(&mut ui, 12, 23, after(start, 120)),
            MouseOutcome::SaveState
        );
        assert!(ui.split_drag.is_none() && !ui.dragging());
    }

    #[test]
    fn a_lost_release_ends_a_moved_column_drag_and_saves() {
        let start = Instant::now();
        let mut ui = draggable_ui();
        down(&mut ui, COLUMN_GRAB.0, COLUMN_GRAB.1, start);
        assert!(ui.column_drag.is_some());
        drag(&mut ui, 36, 3, after(start, 50));
        assert_eq!(
            ui.column_widths[CardId::PacketList as usize][2],
            Some(16),
            "10 + 6 cells of drag"
        );
        assert_eq!(
            moved(&mut ui, 37, 4, after(start, 100)),
            MouseOutcome::SaveState
        );
        assert!(ui.column_drag.is_none() && !ui.dragging());
        assert!(
            ui.last_column_click.is_none(),
            "a lost release is not a click"
        );
    }

    #[test]
    fn a_lost_release_ends_a_scrollbar_drag_without_a_save() {
        let start = Instant::now();
        let mut ui = draggable_ui();
        down(&mut ui, THUMB_GRAB.0, THUMB_GRAB.1, start);
        assert!(ui.scroll_drag.is_some());
        drag(&mut ui, 78, 9, after(start, 50));
        assert!(ui.views[CardId::PacketList as usize].scroll > 0);
        assert_eq!(moved(&mut ui, 60, 9, after(start, 100)), MouseOutcome::None);
        assert!(ui.scroll_drag.is_none() && !ui.dragging());
        // Later buttonless moves only hover.
        assert_eq!(moved(&mut ui, 61, 9, after(start, 150)), MouseOutcome::None);
    }

    #[test]
    fn an_unmoved_drag_ends_on_a_buttonless_move_without_a_save() {
        let start = Instant::now();
        for (column, row) in [SPLIT_GRAB, COLUMN_GRAB] {
            let mut ui = draggable_ui();
            down(&mut ui, column, row, start);
            assert!(ui.dragging());
            assert_eq!(
                moved(&mut ui, column, row, after(start, 30)),
                MouseOutcome::None
            );
            assert!(!ui.dragging());
        }
    }

    #[test]
    fn a_new_press_resets_every_drag_first_and_saves_a_moved_one() {
        let start = Instant::now();

        // Moved split drag, lost release, new press elsewhere: saved and consumed.
        let mut ui = draggable_ui();
        down(&mut ui, SPLIT_GRAB.0, SPLIT_GRAB.1, start);
        drag(&mut ui, 10, 22, after(start, 40));
        assert_eq!(
            down(&mut ui, 5, 25, after(start, 80)),
            MouseOutcome::SaveState
        );
        assert!(!ui.dragging());

        // Unmoved drags reset and the new press then acts normally (a title press here).
        let mut ui = draggable_ui();
        ui.hits.title_hits.push(TitleHit {
            card: CardId::PacketList,
            rect: Rect::new(1, 2, 10, 1),
        });
        ui.hits.slot_groups.push(SlotGroupHit {
            group: SplitGroup::CaptureCols,
            slots: vec![Rect::new(0, 2, 40, 10), Rect::new(41, 2, 39, 10)],
        });
        down(&mut ui, COLUMN_GRAB.0, COLUMN_GRAB.1, start);
        ui.split_drag = Some(SplitDrag {
            group: SplitGroup::CaptureRows,
            index: 0,
            grip: 0,
            moved: false,
        });
        ui.scroll_drag = Some(ScrollDrag {
            card: CardId::PacketList,
            grip: 0,
        });
        assert_eq!(down(&mut ui, 3, 2, after(start, 80)), MouseOutcome::None);
        assert!(!ui.dragging(), "every drag state was reset");
        assert!(ui.card_press.is_some(), "the press itself ran");
    }

    #[test]
    fn commit_drags_reports_only_a_moved_drag() {
        let mut ui = draggable_ui();
        assert!(!ui.commit_drags());
        ui.column_drag = Some(ColumnDrag {
            card: CardId::PacketList,
            column: 2,
            start_x: 30,
            start_width: 10,
            moved: true,
        });
        ui.scroll_drag = Some(ScrollDrag {
            card: CardId::PacketList,
            grip: 0,
        });
        assert!(ui.commit_drags());
        assert!(!ui.dragging());
    }
}
