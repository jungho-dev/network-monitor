//! Card ids and kinds, per-card view state, click actions, mouse hit areas,
//! and scrollbar thumb math shared by the capture TUI and the inspector.

use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use super::layout::{MAX_SLOTS, SplitGroup, SplitHit};

pub const CARD_COUNT: usize = CardId::HistoryDetail as usize + 1;
// Holding a card title this long enters move mode.
pub const CARD_HOLD_TIME: Duration = Duration::from_millis(400);
// Most columns any table card has, and the cap on a user-dragged width.
pub const MAX_TABLE_COLUMNS: usize = 14;
pub const MAX_COLUMN_WIDTH: u16 = 512;

// 1. card ids -----------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardId {
    PacketList,
    PacketDetail,
    HexDump,
    StatTalkers,
    StatProtocols,
    StatFlows,
    StatTimeline,
    StatStream,
    StatBookmarks,
    ConnTable,
    ConnDetail,
    ProcTable,
    ProcDetail,
    DomainTable,
    DomainDetail,
    AlertTable,
    AlertDetail,
    HistoryTable,
    HistoryDetail,
}

impl CardId {
    pub const ALL: [CardId; CARD_COUNT] = [
        CardId::PacketList,
        CardId::PacketDetail,
        CardId::HexDump,
        CardId::StatTalkers,
        CardId::StatProtocols,
        CardId::StatFlows,
        CardId::StatTimeline,
        CardId::StatStream,
        CardId::StatBookmarks,
        CardId::ConnTable,
        CardId::ConnDetail,
        CardId::ProcTable,
        CardId::ProcDetail,
        CardId::DomainTable,
        CardId::DomainDetail,
        CardId::AlertTable,
        CardId::AlertDetail,
        CardId::HistoryTable,
        CardId::HistoryDetail,
    ];

    pub fn kind(self) -> CardKind {
        match self {
            Self::PacketList
            | Self::StatTalkers
            | Self::StatProtocols
            | Self::StatFlows
            | Self::StatBookmarks
            | Self::ConnTable
            | Self::ProcTable
            | Self::DomainTable
            | Self::AlertTable
            | Self::HistoryTable => CardKind::Selectable,
            _ => CardKind::Scrollable,
        }
    }

    // Group and position of a card whose order can change.
    pub fn reorder_slot(self) -> Option<(SplitGroup, usize)> {
        match self {
            Self::PacketList => Some((SplitGroup::CaptureCols, 0)),
            Self::PacketDetail => Some((SplitGroup::CaptureCols, 1)),
            Self::ConnTable => Some((SplitGroup::InspectConn, 0)),
            Self::ConnDetail => Some((SplitGroup::InspectConn, 1)),
            Self::ProcTable => Some((SplitGroup::InspectProc, 0)),
            Self::ProcDetail => Some((SplitGroup::InspectProc, 1)),
            Self::DomainTable => Some((SplitGroup::InspectDomain, 0)),
            Self::DomainDetail => Some((SplitGroup::InspectDomain, 1)),
            Self::AlertTable => Some((SplitGroup::InspectAlert, 0)),
            Self::AlertDetail => Some((SplitGroup::InspectAlert, 1)),
            Self::HistoryTable => Some((SplitGroup::InspectHistory, 0)),
            Self::HistoryDetail => Some((SplitGroup::InspectHistory, 1)),
            _ => None,
        }
    }

    // Stable name and column count of a card whose column widths are saved.
    pub fn table_key(self) -> Option<(&'static str, usize)> {
        match self {
            Self::PacketList => Some(("packet_list", 8)),
            Self::StatTalkers => Some(("stat_talkers", 3)),
            Self::StatProtocols => Some(("stat_protocols", 4)),
            Self::StatFlows => Some(("stat_flows", 5)),
            Self::StatBookmarks => Some(("stat_bookmarks", 4)),
            Self::ConnTable => Some(("conn_table", 6)),
            Self::ProcTable => Some(("proc_table", 4)),
            Self::DomainTable => Some(("domain_table", 4)),
            Self::AlertTable => Some(("alert_table", 3)),
            Self::HistoryTable => Some(("history_table", 5)),
            _ => None,
        }
    }
}

// Wheel and key behavior: scroll the view, or move the selected row with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    Scrollable,
    Selectable,
}

// 2. view state and actions ---------------------------------------------------
// scroll = first shown row, selected = selected row, visible and total are
// recorded by the last render.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CardView {
    pub scroll: usize,
    pub selected: usize,
    pub visible: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMotion {
    Lines(isize),
    Pages(isize),
    Home,
    End,
}

// Footer chips and their keys run the same action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    Quit,
    ShowHelp,
    ToggleLanguage,
    ResetLayout,
    TogglePause,
    OpenFilter,
    OpenColumns,
    SaveLog,
    Refresh,
    OpenSearch,
}

// 3. hit areas ----------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct HeaderHit {
    pub card: CardId,
    pub column: usize,
    pub rect: Rect,
}

// Body rows of a selectable card: clicked row = offset + (y - rect.y).
#[derive(Debug, Clone, Copy)]
pub struct RowHit {
    pub card: CardId,
    pub rect: Rect,
    pub offset: usize,
    pub len: usize,
}

// Scrollbar gutter plus the border column, with the view it was drawn for.
#[derive(Debug, Clone, Copy)]
pub struct ScrollHit {
    pub card: CardId,
    pub track: Rect,
    pub total: usize,
    pub visible: usize,
    pub scroll: usize,
}

// grip = distance from the press to the thumb start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollDrag {
    pub card: CardId,
    pub grip: u16,
}

// Boundary between two header cells (column = left column).
#[derive(Debug, Clone, Copy)]
pub struct ColumnHit {
    pub card: CardId,
    pub column: usize,
    pub grab: Rect,
    pub line: Rect,
    pub width: u16,
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnDrag {
    pub card: CardId,
    pub column: usize,
    pub start_x: u16,
    pub start_width: u16,
    pub moved: bool,
}

// Card title text on the top border: a long press here starts a card move.
#[derive(Debug, Clone, Copy)]
pub struct TitleHit {
    pub card: CardId,
    pub rect: Rect,
}

// One reorderable group as drawn this frame: its slot rects.
#[derive(Debug, Clone)]
pub struct SlotGroupHit {
    pub group: SplitGroup,
    pub slots: Vec<Rect>,
}

// Title pressed and waiting for the hold time; a move to another cell cancels.
#[derive(Debug, Clone, Copy)]
pub struct CardPress {
    pub card: CardId,
    pub group: SplitGroup,
    pub index: usize,
    pub started: Instant,
    pub cell: Position,
}

// baseline = slot order before the press, target = slot being previewed.
#[derive(Debug, Clone, Copy)]
pub struct CardMove {
    pub card: CardId,
    pub group: SplitGroup,
    pub index: usize,
    pub baseline: [u8; MAX_SLOTS],
    pub target: usize,
}

// Rebuilt by every render so clicks always match what is on screen.
#[derive(Debug, Default, Clone)]
pub struct HitAreas {
    pub tab_rects: Vec<(usize, Rect)>,
    pub button_rects: Vec<(UiAction, Rect)>,
    pub header_cells: Vec<HeaderHit>,
    pub column_hits: Vec<ColumnHit>,
    pub row_hits: Vec<RowHit>,
    pub split_hits: Vec<SplitHit>,
    pub scroll_hits: Vec<ScrollHit>,
    pub card_rects: Vec<(CardId, Rect)>,
    pub title_hits: Vec<TitleHit>,
    pub slot_groups: Vec<SlotGroupHit>,
}

// 4. scrollbar thumb ----------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollThumb {
    pub start: u16,
    pub len: u16,
}

// View -> thumb; None when the content fits or the track is under 2 cells.
pub fn scroll_thumb(
    total: usize,
    visible: usize,
    scroll: usize,
    track: u16,
) -> Option<ScrollThumb> {
    if visible == 0 || total <= visible || track < 2 {
        return None;
    }
    let track_len = usize::from(track);
    let len = ((visible * track_len + total / 2) / total).clamp(1, track_len - 1);
    let free = track_len - len;
    let max_scroll = total - visible;
    let start = (scroll.min(max_scroll) * free + max_scroll / 2) / max_scroll;
    Some(ScrollThumb {
        start: start as u16,
        len: len as u16,
    })
}

// Thumb start -> scroll (inverse of `scroll_thumb`, endpoints preserved).
pub fn scroll_for_thumb(total: usize, visible: usize, track: u16, thumb_start: u16) -> usize {
    let Some(thumb) = scroll_thumb(total, visible, 0, track) else {
        return 0;
    };
    let free = usize::from(track - thumb.len);
    let max_scroll = total - visible;
    (usize::from(thumb_start).min(free) * max_scroll + free / 2) / free
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::columns::card_columns;

    #[test]
    fn card_table_exposes_every_card_once_in_order() {
        for (position, card) in CardId::ALL.iter().enumerate() {
            assert_eq!(*card as usize, position);
        }
        assert_eq!(CardId::ALL.len(), CARD_COUNT);
    }

    #[test]
    fn reorder_slots_cover_every_reorderable_group_exactly_once() {
        for group in SplitGroup::ALL {
            let mut indices: Vec<usize> = CardId::ALL
                .iter()
                .filter_map(|card| card.reorder_slot())
                .filter(|(owner, _)| *owner == group)
                .map(|(_, index)| index)
                .collect();
            indices.sort_unstable();
            if group.is_reorderable() {
                let expected: Vec<usize> = (0..group.card_count()).collect();
                assert_eq!(indices, expected, "{group:?}");
            } else {
                assert!(indices.is_empty(), "{group:?}");
            }
        }
    }

    #[test]
    fn table_keys_match_the_column_definitions_and_stay_unique() {
        let mut names = Vec::new();
        for card in CardId::ALL {
            match card.table_key() {
                Some((name, count)) => {
                    assert_eq!(card_columns(card).len(), count, "{card:?}");
                    assert!(count <= MAX_TABLE_COLUMNS);
                    names.push(name);
                }
                None => assert!(card_columns(card).is_empty(), "{card:?}"),
            }
        }
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total);
    }

    #[test]
    fn scroll_thumb_matches_proportion_and_hits_both_ends() {
        let top = scroll_thumb(50, 10, 0, 10).unwrap();
        let middle = scroll_thumb(50, 10, 20, 10).unwrap();
        let bottom = scroll_thumb(50, 10, 40, 10).unwrap();
        assert_eq!((top.len, middle.len, bottom.len), (2, 2, 2));
        assert_eq!((top.start, middle.start, bottom.start), (0, 4, 8));
        assert_eq!(scroll_thumb(50, 10, 999, 10).unwrap().start, 8);
    }

    #[test]
    fn scroll_thumb_is_absent_when_content_fits_or_track_is_tiny() {
        assert_eq!(scroll_thumb(10, 10, 0, 8), None);
        assert_eq!(scroll_thumb(5, 10, 0, 8), None);
        assert_eq!(scroll_thumb(50, 0, 0, 8), None);
        assert_eq!(scroll_thumb(50, 10, 0, 1), None);
    }

    #[test]
    fn scroll_for_thumb_round_trips_endpoints_and_is_monotonic() {
        for (total, visible, track) in [(50usize, 10usize, 10u16), (200, 15, 15), (17, 9, 9)] {
            let max_scroll = total - visible;
            let mut previous = 0usize;
            for scroll in 0..=max_scroll {
                let thumb = scroll_thumb(total, visible, scroll, track).unwrap();
                let back = scroll_for_thumb(total, visible, track, thumb.start);
                assert!(back >= previous, "{total}/{visible}/{track} at {scroll}");
                previous = back;
            }
            assert_eq!(scroll_for_thumb(total, visible, track, 0), 0);
            let last = scroll_thumb(total, visible, max_scroll, track).unwrap();
            assert_eq!(
                scroll_for_thumb(total, visible, track, last.start),
                max_scroll
            );
            assert_eq!(scroll_for_thumb(total, visible, track, 999), max_scroll);
        }
    }
}
