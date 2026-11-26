//! Capture tab reordering: dragging a pressed tab previews the new order,
//! release commits it, Esc restores it, and `<` / `>` move the active tab one
//! slot.

use ratatui::layout::Position;

use super::i18n::{self, Msg};
use super::interaction::ToastLevel;
use super::state::{App, CaptureTab, Command};

// 1. drag state ---------------------------------------------------------------
// baseline = order before the press (Esc restores it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDrag {
    pub tab: CaptureTab,
    pub baseline: [CaptureTab; 6],
}

impl App {
    // 2. drag preview ---------------------------------------------------------
    // Moves the dragged tab to the first slot whose span holds the pointer; the
    // current slot holding it keeps the order, so tabs of other widths settle.
    pub fn preview_tab_move(&mut self, position: Position) {
        let Some(drag) = self.tab_drag else {
            return;
        };
        // 1. drawn tab widths and gap (single-digit numbers: order-independent)
        let rects = &self.ui.hits.capture_tabs;
        if rects.len() != CaptureTab::ALL.len() {
            return;
        }
        let start = rects[0].1.x;
        let gap = rects[1].1.x.saturating_sub(rects[0].1.right());
        let width_of = |tab: CaptureTab| {
            rects
                .iter()
                .find(|(drawn, _)| *drawn == tab)
                .map_or(0, |(_, rect)| rect.width)
        };

        // 2. dragged tab span per slot, first slot holding the pointer
        let others: Vec<CaptureTab> = self
            .ui
            .tab_order
            .iter()
            .copied()
            .filter(|tab| *tab != drag.tab)
            .collect();
        let current = self.ui.tab_order.iter().position(|tab| *tab == drag.tab);
        let width = width_of(drag.tab);
        let mut left = start;
        let mut target = None;
        for slot in 0..CaptureTab::ALL.len() {
            let holds = (left..left.saturating_add(width)).contains(&position.x);
            if holds && Some(slot) == current {
                return;
            }
            if holds {
                target.get_or_insert(slot);
            }
            if let Some(next) = others.get(slot) {
                left = left.saturating_add(width_of(*next) + gap);
            }
        }

        // 3. order with the dragged tab at the target slot
        let Some(target) = target else {
            return;
        };
        let mut order = others;
        order.insert(target, drag.tab);
        if let Ok(order) = <[CaptureTab; 6]>::try_from(order) {
            self.ui.tab_order = order;
        }
    }

    // 3. drag commit and cancel -----------------------------------------------
    // Release or lost release keeps the shown order; true when it changed.
    pub fn finish_tab_drag(&mut self) -> bool {
        let Some(drag) = self.tab_drag.take() else {
            return false;
        };
        if drag.baseline == self.ui.tab_order {
            return false;
        }
        let text = i18n::text(self.ui.lang, Msg::ToastTabMoved);
        self.ui.show_toast(text, ToastLevel::Info);
        true
    }

    // Esc: the order from before the press.
    pub fn cancel_tab_drag(&mut self) {
        if let Some(drag) = self.tab_drag.take() {
            self.ui.tab_order = drag.baseline;
        }
    }

    // 4. keyboard move --------------------------------------------------------
    // `<` left, `>` right by one slot and saved; ignored at the row ends.
    pub fn move_active_tab(&mut self, step: isize) -> Command {
        let Some(slot) = self.ui.tab_order.iter().position(|tab| *tab == self.tab) else {
            return Command::None;
        };
        let Some(target) = slot
            .checked_add_signed(step)
            .filter(|target| *target < self.ui.tab_order.len())
        else {
            return Command::None;
        };
        self.ui.tab_order.swap(slot, target);
        let text = i18n::text(self.ui.lang, Msg::ToastTabMoved);
        self.ui.show_toast(text, ToastLevel::Info);
        Command::SaveState
    }
}
