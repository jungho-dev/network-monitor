//! Card layout: split boundaries stored as ratios, card order per group,
//! card rects returned by card index, and boundary drag areas.

use ratatui::layout::Rect;

use super::cards::{HitAreas, SlotGroupHit};

// Most boundaries and slots a group can hold; ratios use ten-thousandths.
pub const MAX_CUTS: usize = 3;
pub const MAX_SLOTS: usize = MAX_CUTS + 1;
const RATIO_SCALE: u32 = 10_000;
pub const GROUP_COUNT: usize = SplitGroup::InspectHistory as usize + 1;
// Order of a group that was never rearranged (slot = card index).
pub const IDENTITY_ORDER: [u8; MAX_SLOTS] = [0, 1, 2, 3];

// 1. groups and drag areas ----------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitGroup {
    // Main area over the hex pane; each slot holds a bundle, so no reordering.
    CaptureRows,
    CaptureCols,
    CaptureFlows,
    CaptureStats,
    CaptureAlerts,
    InspectConn,
    InspectProc,
    InspectDomain,
    InspectAlert,
    InspectHistory,
}

impl SplitGroup {
    pub const ALL: [Self; GROUP_COUNT] = [
        Self::CaptureRows,
        Self::CaptureCols,
        Self::CaptureFlows,
        Self::CaptureStats,
        Self::CaptureAlerts,
        Self::InspectConn,
        Self::InspectProc,
        Self::InspectDomain,
        Self::InspectAlert,
        Self::InspectHistory,
    ];

    // Fixed names the state file uses (enum numbers are never written).
    pub fn group_key(self) -> &'static str {
        match self {
            Self::CaptureRows => "capture_rows",
            Self::CaptureCols => "capture_cols",
            Self::CaptureFlows => "capture_flows",
            Self::CaptureStats => "capture_stats",
            Self::CaptureAlerts => "capture_alerts",
            Self::InspectConn => "inspect_conn",
            Self::InspectProc => "inspect_proc",
            Self::InspectDomain => "inspect_domain",
            Self::InspectAlert => "inspect_alert",
            Self::InspectHistory => "inspect_history",
        }
    }

    pub fn card_count(self) -> usize {
        2
    }

    pub fn axis(self) -> SplitAxis {
        match self {
            Self::CaptureCols | Self::CaptureStats => SplitAxis::Columns,
            _ => SplitAxis::Rows,
        }
    }

    // Only groups whose slot is one card are reorderable.
    pub fn is_reorderable(self) -> bool {
        self != Self::CaptureRows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    // Side by side with one empty column between.
    Columns,
    // Stacked, borders touching.
    Rows,
}

// Drag area of one boundary (positions are cells from `origin`).
#[derive(Debug, Clone, Copy)]
pub struct SplitHit {
    pub group: SplitGroup,
    pub index: usize,
    pub axis: SplitAxis,
    pub grab: Rect,
    pub line: Rect,
    pub origin: u16,
    pub span_len: u16,
    pub cuts: [u16; MAX_CUTS],
    pub prev_end: u16,
    pub next_cut: u16,
    pub low: u16,
    pub high: u16,
}

// grip = distance from the press to the boundary, moved = actually dragged.
#[derive(Debug, Clone, Copy)]
pub struct SplitDrag {
    pub group: SplitGroup,
    pub index: usize,
    pub grip: i32,
    pub moved: bool,
}

// 2. card order ---------------------------------------------------------------
// Insertion move: take `card` out of `baseline` and insert it at slot `target`.
pub fn insert_card_order(
    baseline: &[u8; MAX_SLOTS],
    count: usize,
    card: u8,
    target: usize,
) -> [u8; MAX_SLOTS] {
    let mut order = *baseline;
    let count = count.min(MAX_SLOTS);
    let Some(from) = baseline[..count].iter().position(|placed| *placed == card) else {
        return order;
    };
    let target = target.min(count.saturating_sub(1));
    if from < target {
        order.copy_within(from + 1..=target, from);
    } else if from > target {
        order.copy_within(target..from, target + 1);
    }
    order[target] = card;
    order
}

// 3. layout state -------------------------------------------------------------
// Boundary ratios and slot order per group; None = default layout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CardLayout {
    ratios: [Option<[u16; MAX_CUTS]>; GROUP_COUNT],
    orders: [Option<[u8; MAX_SLOTS]>; GROUP_COUNT],
}

impl CardLayout {
    // Splits `area` into a group's cards: places boundaries, registers drag
    // areas and slots, and returns rects indexed by card (slot sizes, ratios,
    // and minimums stay with the slot when cards are reordered).
    // defaults = default boundary cells, mins = minimum size per slot.
    pub fn split_cards(
        &self,
        hit_areas: &mut HitAreas,
        group: SplitGroup,
        area: Rect,
        defaults: &[u16],
        mins: &[u16],
    ) -> Vec<Rect> {
        debug_assert_eq!(mins.len(), defaults.len() + 1);
        let axis = group.axis();
        let (origin, span_len, gap) = match axis {
            SplitAxis::Columns => (area.x, area.width, 1),
            SplitAxis::Rows => (area.y, area.height, 0),
        };
        let count = defaults.len().min(MAX_CUTS);

        // 1. boundary cells (only a rearranged group applies ratios and minimums)
        let ratios = self.ratios[group as usize];
        let mut cuts = [0u16; MAX_CUTS];
        let mut pane_start = 0u16;
        for (index, default_cut) in defaults.iter().take(count).enumerate() {
            cuts[index] = match ratios {
                Some(ratio) => {
                    let scaled = u32::from(ratio[index]) * u32::from(span_len);
                    let wanted = ((scaled + RATIO_SCALE / 2) / RATIO_SCALE) as u16;
                    let tail_min: u16 = mins[index + 1..].iter().map(|min| min + gap).sum();
                    let low = pane_start.saturating_add(mins[index]);
                    wanted
                        .min(span_len.saturating_sub(tail_min))
                        .max(low)
                        .min(span_len)
                }
                None => (*default_cut).min(span_len),
            };
            pane_start = cuts[index].saturating_add(gap);
        }

        // 2. slot rects in slot order
        let mut slots = Vec::with_capacity(count + 1);
        let mut pane_start = 0u16;
        for pane_end in cuts[..count].iter().copied().chain([span_len]) {
            let pane_len = pane_end.saturating_sub(pane_start);
            slots.push(match axis {
                SplitAxis::Columns => Rect::new(origin + pane_start, area.y, pane_len, area.height),
                SplitAxis::Rows => Rect::new(area.x, origin + pane_start, area.width, pane_len),
            });
            pane_start = pane_end.saturating_add(gap).min(span_len);
        }

        // 3. card order: card k receives the rect of the slot it occupies
        let order = self.order_of(group);
        let mut by_card = slots.clone();
        for (slot, rect) in slots.iter().enumerate() {
            let placed = usize::from(order[slot]);
            let card = if placed < slots.len() { placed } else { slot };
            by_card[card] = *rect;
        }
        if group.is_reorderable() {
            hit_areas.slot_groups.push(SlotGroupHit { group, slots });
        }

        // 4. boundary drag areas between the neighbors' minimum sizes
        for (index, cut) in cuts[..count].iter().copied().enumerate() {
            let prev_end = if index == 0 { 0 } else { cuts[index - 1] + gap };
            let next_cut = if index + 1 < count {
                cuts[index + 1]
            } else {
                span_len
            };
            let low = prev_end.saturating_add(mins[index]);
            let high = next_cut.saturating_sub(gap + mins[index + 1]);
            if low > high || cut == 0 {
                continue;
            }
            // A default boundary outside the limits widens them so the first drag does not jump.
            let (low, high) = (low.min(cut), high.max(cut));
            let (grab, line) = match axis {
                SplitAxis::Columns => (
                    Rect::new(origin + cut - 1, area.y, 3, area.height),
                    Rect::new(origin + cut, area.y, 1, area.height),
                ),
                SplitAxis::Rows => (
                    Rect::new(area.x, origin + cut - 1, area.width, 2),
                    Rect::new(area.x, origin + cut - 1, area.width, 1),
                ),
            };
            hit_areas.split_hits.push(SplitHit {
                group,
                index,
                axis,
                grab: grab.intersection(area),
                line: line.intersection(area),
                origin,
                span_len,
                cuts,
                prev_end,
                next_cut,
                low,
                high,
            });
        }
        by_card
    }

    // Moves a boundary to `pointer`, stores the whole group as ratios, and
    // returns the sizes before and after the boundary.
    pub fn drag_cut(&mut self, hit: &SplitHit, pointer: i32) -> Option<(u16, u16)> {
        if hit.span_len == 0 || hit.low > hit.high {
            return None;
        }
        let offset = pointer - i32::from(hit.origin);
        let cut = offset.clamp(i32::from(hit.low), i32::from(hit.high)) as u16;
        let mut cuts = hit.cuts;
        cuts[hit.index] = cut;
        let span_len = u32::from(hit.span_len);
        self.ratios[hit.group as usize] =
            Some(cuts.map(|cut| ((u32::from(cut) * RATIO_SCALE + span_len / 2) / span_len) as u16));
        let gap = match hit.axis {
            SplitAxis::Columns => 1,
            SplitAxis::Rows => 0,
        };
        Some((cut - hit.prev_end, hit.next_cut.saturating_sub(cut + gap)))
    }

    pub fn order_of(&self, group: SplitGroup) -> [u8; MAX_SLOTS] {
        self.orders[group as usize].unwrap_or(IDENTITY_ORDER)
    }

    pub fn slot_of(&self, group: SplitGroup, card: usize) -> Option<usize> {
        let order = self.order_of(group);
        order[..group.card_count()]
            .iter()
            .position(|placed| usize::from(*placed) == card)
    }

    // An order equal to the default is not stored.
    pub fn set_order(&mut self, group: SplitGroup, order: [u8; MAX_SLOTS]) {
        self.orders[group as usize] = (order != IDENTITY_ORDER).then_some(order);
    }

    pub fn move_card_to_slot(
        &mut self,
        group: SplitGroup,
        baseline: &[u8; MAX_SLOTS],
        card: usize,
        target: usize,
    ) {
        let moved = insert_card_order(baseline, group.card_count(), card as u8, target);
        self.set_order(group, moved);
    }

    // State-file values: customized order and ratios, sized to the group.
    pub fn custom_order(&self, group: SplitGroup) -> Option<Vec<u8>> {
        self.orders[group as usize].map(|order| order[..group.card_count()].to_vec())
    }

    pub fn custom_ratios(&self, group: SplitGroup) -> Option<Vec<u16>> {
        self.ratios[group as usize].map(|ratio| ratio[..group.card_count() - 1].to_vec())
    }

    // Restores only when length, range, and uniqueness hold; returns whether it applied.
    pub fn restore_order(&mut self, group: SplitGroup, values: &[i64]) -> bool {
        let count = group.card_count();
        if !group.is_reorderable() || values.len() != count {
            return false;
        }
        let mut order = IDENTITY_ORDER;
        let mut seen = [false; MAX_SLOTS];
        for (slot, value) in values.iter().enumerate() {
            let Ok(card) = usize::try_from(*value) else {
                return false;
            };
            if card >= count || seen[card] {
                return false;
            }
            seen[card] = true;
            order[slot] = card as u8;
        }
        self.set_order(group, order);
        true
    }

    pub fn restore_ratios(&mut self, group: SplitGroup, values: &[i64]) -> bool {
        if values.len() != group.card_count() - 1 {
            return false;
        }
        let mut ratios = [0u16; MAX_CUTS];
        for (slot, value) in values.iter().enumerate() {
            let Ok(ratio) = u16::try_from(*value) else {
                return false;
            };
            if u32::from(ratio) > RATIO_SCALE {
                return false;
            }
            ratios[slot] = ratio;
        }
        self.ratios[group as usize] = Some(ratios);
        true
    }

    pub fn reset_groups(&mut self, groups: &[SplitGroup]) {
        for group in groups {
            self.ratios[*group as usize] = None;
            self.orders[*group as usize] = None;
        }
    }

    #[cfg(test)]
    pub fn has_custom_groups(&self, groups: &[SplitGroup]) -> bool {
        groups.iter().any(|group| {
            self.ratios[*group as usize].is_some() || self.orders[*group as usize].is_some()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_capture_rows(layout: &CardLayout, height: u16) -> (Vec<Rect>, HitAreas) {
        let mut hit_areas = HitAreas::default();
        let panes = layout.split_cards(
            &mut hit_areas,
            SplitGroup::CaptureRows,
            Rect::new(0, 10, 100, height),
            &[height - 6],
            &[5, 3],
        );
        (panes, hit_areas)
    }

    fn split_cols(layout: &CardLayout) -> (Vec<Rect>, HitAreas) {
        let mut hit_areas = HitAreas::default();
        let panes = layout.split_cards(
            &mut hit_areas,
            SplitGroup::CaptureCols,
            Rect::new(2, 5, 81, 8),
            &[50],
            &[20, 16],
        );
        (panes, hit_areas)
    }

    #[test]
    fn default_rows_keep_the_given_cut_and_register_a_grab_area() {
        let (panes, hit_areas) = split_capture_rows(&CardLayout::default(), 30);
        assert_eq!(panes[0].height, 24);
        assert_eq!(panes[1], Rect::new(0, 34, 100, 6));
        assert_eq!(hit_areas.split_hits.len(), 1);
        assert_eq!(hit_areas.split_hits[0].grab, Rect::new(0, 33, 100, 2));
        assert_eq!(hit_areas.split_hits[0].line, Rect::new(0, 33, 100, 1));
        // A bundle group never registers slots, so it cannot be reordered.
        assert!(hit_areas.slot_groups.is_empty());
    }

    #[test]
    fn dragged_cut_clamps_to_neighbor_minimum_and_keeps_its_ratio_on_resize() {
        let mut layout = CardLayout::default();
        let (_, hit_areas) = split_capture_rows(&layout, 30);
        assert_eq!(layout.drag_cut(&hit_areas.split_hits[0], 0), Some((5, 25)));
        let (panes, _) = split_capture_rows(&layout, 30);
        assert_eq!((panes[0].height, panes[1].height), (5, 25));
        // Twice the height keeps the same ratio.
        let (panes, _) = split_capture_rows(&layout, 60);
        assert_eq!((panes[0].height, panes[1].height), (10, 50));
        // Dragging far below stops at the lower neighbor's minimum.
        let (_, hit_areas) = split_capture_rows(&layout, 30);
        layout.drag_cut(&hit_areas.split_hits[0], 1000);
        let (panes, _) = split_capture_rows(&layout, 30);
        assert_eq!(panes[1].height, 3);
    }

    #[test]
    fn column_split_leaves_a_gap_and_grabs_both_borders() {
        let (panes, hit_areas) = split_cols(&CardLayout::default());
        assert_eq!(panes[0], Rect::new(2, 5, 50, 8));
        assert_eq!(panes[1], Rect::new(53, 5, 30, 8));
        let hit = hit_areas.split_hits[0];
        assert_eq!(hit.line, Rect::new(52, 5, 1, 8));
        assert_eq!(hit.grab, Rect::new(51, 5, 3, 8));
        assert_eq!((hit.low, hit.high), (20, 64));
    }

    #[test]
    fn insertion_move_shifts_the_cards_between_source_and_target() {
        let base = IDENTITY_ORDER;
        assert_eq!(insert_card_order(&base, 4, 0, 2), [1, 2, 0, 3]);
        assert_eq!(insert_card_order(&base, 4, 3, 1), [0, 3, 1, 2]);
        assert_eq!(insert_card_order(&base, 4, 2, 2), base);
        assert_eq!(insert_card_order(&base, 2, 0, 1), [1, 0, 2, 3]);
        assert_eq!(insert_card_order(&base, 3, 0, 9), [1, 2, 0, 3]);
        assert_eq!(insert_card_order(&base, 2, 3, 0), base);
    }

    #[test]
    fn reordered_cards_take_each_others_slot_rects_and_slots_keep_their_size() {
        let (default_panes, _) = split_cols(&CardLayout::default());
        let mut layout = CardLayout::default();
        layout.move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
        assert_eq!(layout.order_of(SplitGroup::CaptureCols)[..2], [1, 0]);
        assert_eq!(layout.slot_of(SplitGroup::CaptureCols, 0), Some(1));
        let (panes, hit_areas) = split_cols(&layout);
        assert_eq!(panes[0], default_panes[1]);
        assert_eq!(panes[1], default_panes[0]);
        let recorded = &hit_areas.slot_groups[0];
        assert_eq!(recorded.slots, default_panes);
    }

    #[test]
    fn reset_clears_orders_and_ratios_and_moving_home_equals_the_default() {
        let mut layout = CardLayout::default();
        layout.move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
        assert!(layout.restore_ratios(SplitGroup::InspectConn, &[6_000]));
        assert!(layout.has_custom_groups(&[SplitGroup::CaptureCols]));
        layout.reset_groups(&[SplitGroup::InspectConn]);
        assert!(layout.has_custom_groups(&[SplitGroup::CaptureCols]));
        layout.reset_groups(&SplitGroup::ALL);
        assert_eq!(layout, CardLayout::default());

        layout.move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
        let moved = layout.order_of(SplitGroup::CaptureCols);
        layout.move_card_to_slot(SplitGroup::CaptureCols, &moved, 0, 0);
        assert_eq!(layout, CardLayout::default());
    }

    #[test]
    fn group_metadata_is_unique_and_matches_axis_and_reorder_scope() {
        let mut keys: Vec<&str> = SplitGroup::ALL
            .iter()
            .map(|group| group.group_key())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), GROUP_COUNT);
        for (position, group) in SplitGroup::ALL.iter().enumerate() {
            assert_eq!(*group as usize, position);
            assert!((2..=MAX_SLOTS).contains(&group.card_count()));
        }
        assert!(!SplitGroup::CaptureRows.is_reorderable());
        assert_eq!(SplitGroup::CaptureCols.axis(), SplitAxis::Columns);
        assert_eq!(SplitGroup::InspectConn.axis(), SplitAxis::Rows);
    }

    #[test]
    fn restore_rejects_wrong_length_duplicates_and_out_of_range_values() {
        let mut layout = CardLayout::default();
        assert!(layout.restore_order(SplitGroup::CaptureCols, &[1, 0]));
        assert_eq!(
            layout.custom_order(SplitGroup::CaptureCols),
            Some(vec![1, 0])
        );

        let mut layout = CardLayout::default();
        assert!(!layout.restore_order(SplitGroup::CaptureCols, &[0]));
        assert!(!layout.restore_order(SplitGroup::CaptureCols, &[1, 1]));
        assert!(!layout.restore_order(SplitGroup::CaptureCols, &[0, 2]));
        assert!(!layout.restore_order(SplitGroup::CaptureCols, &[0, -1]));
        assert!(!layout.restore_order(SplitGroup::CaptureRows, &[0, 1]));
        assert!(!layout.restore_ratios(SplitGroup::InspectConn, &[3_000, 5_000]));
        assert!(!layout.restore_ratios(SplitGroup::InspectConn, &[10_001]));
        assert!(!layout.restore_ratios(SplitGroup::InspectConn, &[-1]));
        assert_eq!(layout, CardLayout::default());

        assert!(layout.restore_ratios(SplitGroup::InspectConn, &[6_000]));
        assert_eq!(
            layout.custom_ratios(SplitGroup::InspectConn),
            Some(vec![6_000])
        );
    }
}
