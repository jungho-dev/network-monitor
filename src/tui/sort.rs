//! Table sorting: typed keys, column kinds, and the three-step click cycle.
//!
//! Numeric and time columns start descending, text columns ascending, and the
//! third step returns to the table's capture order (`None`). Rows sort by typed
//! keys, never by display labels, so switching the language never reorders.

use std::cmp::Ordering;

// 1. kinds and keys -----------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKind {
    Text,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortKey {
    Num(i64),
    Text(String),
}

impl SortKey {
    pub fn compare(&self, other: &SortKey) -> Ordering {
        match (self, other) {
            (SortKey::Num(left), SortKey::Num(right)) => left.cmp(right),
            (SortKey::Text(left), SortKey::Text(right)) => left
                .to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right)),
            (SortKey::Num(_), SortKey::Text(_)) => Ordering::Less,
            (SortKey::Text(_), SortKey::Num(_)) => Ordering::Greater,
        }
    }
}

// 2. state and cycle ----------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortState {
    pub column: usize,
    pub descending: bool,
}

// None = capture order.
pub type Sort = Option<SortState>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortChange {
    SelectColumn(usize),
    NextColumn,
    ReverseDirection,
}

impl SortState {
    pub fn sort_arrow(self) -> &'static str {
        if self.descending { "↓" } else { "↑" }
    }
}

// Direction a column starts in: numbers and times descending, text ascending.
pub fn first_descending(kinds: &[SortKind], column: usize) -> bool {
    kinds.get(column) == Some(&SortKind::Number)
}

// Header click, `s`, and `S` share one cycle: a new column starts in its first
// direction, the same column flips, and the third step resets to capture order.
pub fn apply_change(current: Sort, change: SortChange, kinds: &[SortKind]) -> Sort {
    let column_count = kinds.len();
    if column_count == 0 {
        return None;
    }
    let column = match change {
        SortChange::NextColumn => {
            return match current {
                None => Some(first_state(kinds, 0)),
                Some(state) if state.column + 1 < column_count => {
                    Some(first_state(kinds, state.column + 1))
                }
                Some(_) => None,
            };
        }
        SortChange::SelectColumn(column) => column,
        SortChange::ReverseDirection => current.map_or(0, |state| state.column),
    };
    if column >= column_count {
        return current;
    }
    match current {
        Some(state) if state.column == column => {
            if state.descending == first_descending(kinds, column) {
                Some(SortState {
                    column,
                    descending: !state.descending,
                })
            } else {
                None
            }
        }
        _ => Some(first_state(kinds, column)),
    }
}

fn first_state(kinds: &[SortKind], column: usize) -> SortState {
    SortState {
        column,
        descending: first_descending(kinds, column),
    }
}

// 3. ordering -----------------------------------------------------------------
// Stable ordering of row positions by their typed keys (capture order when None).
pub fn sorted_positions(keys: &[SortKey], descending: bool) -> Vec<usize> {
    let mut positions: Vec<usize> = (0..keys.len()).collect();
    positions.sort_by(|left, right| {
        let order = keys[*left].compare(&keys[*right]);
        if descending { order.reverse() } else { order }
    });
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [SortKind; 3] = [SortKind::Text, SortKind::Number, SortKind::Number];

    #[test]
    fn numeric_columns_start_descending_and_text_columns_ascending() {
        let numeric = apply_change(None, SortChange::SelectColumn(1), &KINDS);
        assert_eq!(
            numeric,
            Some(SortState {
                column: 1,
                descending: true
            })
        );
        let text = apply_change(None, SortChange::SelectColumn(0), &KINDS);
        assert_eq!(
            text,
            Some(SortState {
                column: 0,
                descending: false
            })
        );
    }

    #[test]
    fn click_cycle_is_first_direction_then_flip_then_capture_order() {
        for column in 0..KINDS.len() {
            let first = apply_change(None, SortChange::SelectColumn(column), &KINDS);
            let second = apply_change(first, SortChange::SelectColumn(column), &KINDS);
            let third = apply_change(second, SortChange::SelectColumn(column), &KINDS);
            assert_eq!(first.unwrap().descending, first_descending(&KINDS, column));
            assert_eq!(second.unwrap().descending, !first.unwrap().descending);
            assert_eq!(third, None, "column {column}");
        }
    }

    #[test]
    fn another_column_restarts_in_its_own_first_direction() {
        let first = apply_change(None, SortChange::SelectColumn(0), &KINDS);
        let moved = apply_change(first, SortChange::SelectColumn(2), &KINDS).unwrap();
        assert_eq!((moved.column, moved.descending), (2, true));
    }

    #[test]
    fn s_walks_columns_and_wraps_to_capture_order_while_upper_s_follows_the_click_cycle() {
        let mut current = None;
        let mut seen = Vec::new();
        for _ in 0..KINDS.len() {
            current = apply_change(current, SortChange::NextColumn, &KINDS);
            seen.push(current.unwrap().column);
        }
        assert_eq!(seen, [0, 1, 2]);
        assert_eq!(apply_change(current, SortChange::NextColumn, &KINDS), None);

        let started = apply_change(None, SortChange::ReverseDirection, &KINDS).unwrap();
        assert_eq!(started.column, 0);
        let flipped = apply_change(Some(started), SortChange::ReverseDirection, &KINDS);
        assert_eq!(flipped.unwrap().descending, !started.descending);
        assert_eq!(
            apply_change(flipped, SortChange::ReverseDirection, &KINDS),
            None
        );
    }

    #[test]
    fn out_of_range_column_and_empty_kinds_change_nothing() {
        assert_eq!(
            apply_change(None, SortChange::SelectColumn(9), &KINDS),
            None
        );
        assert_eq!(apply_change(None, SortChange::NextColumn, &[]), None);
    }

    #[test]
    fn typed_keys_order_numbers_numerically_and_text_case_insensitively() {
        let keys = vec![
            SortKey::Num(10),
            SortKey::Num(9),
            SortKey::Num(100),
            SortKey::Num(9),
        ];
        assert_eq!(sorted_positions(&keys, false), [1, 3, 0, 2]);
        assert_eq!(sorted_positions(&keys, true), [2, 0, 1, 3]);
        let names = vec![
            SortKey::Text("beta".into()),
            SortKey::Text("Alpha".into()),
            SortKey::Text("alpha".into()),
        ];
        assert_eq!(sorted_positions(&names, false), [1, 2, 0]);
    }
}
