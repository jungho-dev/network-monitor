//! Column definitions of every table card: header label, minimum width, fit
//! rule, alignment, and sort kind. One source for the renderer, the sort
//! cycle, and the state file's column counts.

use ratatui::layout::Alignment;

use super::cards::CardId;
use super::i18n::Msg;
use super::sort::SortKind;

// How a column gets its width: numbers and short fields stay fixed, text
// shrinks first when room runs out, and the right-most Grow column takes slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFit {
    Fixed,
    Shrink,
    Grow,
}

// keep = the last column to hide when the card is too narrow.
#[derive(Debug, Clone, Copy)]
pub struct ColumnDef {
    pub label: Msg,
    pub min: u16,
    pub fit: ColumnFit,
    pub align: Alignment,
    pub kind: SortKind,
    pub keep: bool,
}

const fn column(
    label: Msg,
    min: u16,
    fit: ColumnFit,
    align: Alignment,
    kind: SortKind,
    keep: bool,
) -> ColumnDef {
    ColumnDef {
        label,
        min,
        fit,
        align,
        kind,
        keep,
    }
}

const LEFT: Alignment = Alignment::Left;
const RIGHT: Alignment = Alignment::Right;

pub const PACKET_COLUMNS: [ColumnDef; 8] = [
    column(
        Msg::ColTime,
        12,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDir,
        5,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColProcess,
        4,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColHost,
        4,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColIp,
        4,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColProto,
        5,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColLen,
        3,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(Msg::ColInfo, 4, ColumnFit::Grow, LEFT, SortKind::Text, true),
];

const TALKER_COLUMNS: [ColumnDef; 3] = [
    column(
        Msg::ColEndpoint,
        8,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColPackets,
        7,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColBytes,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
];

const PROTOCOL_COLUMNS: [ColumnDef; 4] = [
    column(
        Msg::ColProtocol,
        8,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColCount,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColPct,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColShare,
        5,
        ColumnFit::Grow,
        LEFT,
        SortKind::Number,
        false,
    ),
];

const FLOW_COLUMNS: [ColumnDef; 5] = [
    column(Msg::ColFlow, 8, ColumnFit::Grow, LEFT, SortKind::Text, true),
    column(
        Msg::ColProto,
        5,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColPackets,
        7,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColBytes,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDuration,
        8,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
];

const BOOKMARK_COLUMNS: [ColumnDef; 4] = [
    column(
        Msg::ColNumber,
        3,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColTime,
        12,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColEndpoints,
        8,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(Msg::ColInfo, 4, ColumnFit::Grow, LEFT, SortKind::Text, true),
];

const THREAT_COLUMNS: [ColumnDef; 4] = [
    column(
        Msg::ColTime,
        12,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColKind,
        6,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColNo,
        3,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDetail,
        4,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
];

const CONN_COLUMNS: [ColumnDef; 6] = [
    column(
        Msg::ColProcess,
        7,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColDestination,
        11,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColGeo,
        3,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColUp,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDown,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColProto,
        5,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Text,
        false,
    ),
];

const PROC_COLUMNS: [ColumnDef; 4] = [
    column(
        Msg::ColProcess,
        7,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColConns,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColUp,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDown,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
];

const DOMAIN_COLUMNS: [ColumnDef; 4] = [
    column(
        Msg::ColDomain,
        8,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColConns,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColUp,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDown,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
];

const ALERT_COLUMNS: [ColumnDef; 3] = [
    column(
        Msg::ColSeverity,
        8,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColKind,
        6,
        ColumnFit::Shrink,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColTitle,
        8,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
];

const HISTORY_COLUMNS: [ColumnDef; 5] = [
    column(
        Msg::ColDestination,
        11,
        ColumnFit::Grow,
        LEFT,
        SortKind::Text,
        true,
    ),
    column(
        Msg::ColProtoPort,
        9,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Text,
        false,
    ),
    column(
        Msg::ColUp,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColDown,
        5,
        ColumnFit::Fixed,
        RIGHT,
        SortKind::Number,
        false,
    ),
    column(
        Msg::ColStarted,
        19,
        ColumnFit::Fixed,
        LEFT,
        SortKind::Number,
        false,
    ),
];

// Columns of a table card; empty for a card that is not a table.
pub fn card_columns(card: CardId) -> &'static [ColumnDef] {
    match card {
        CardId::PacketList => &PACKET_COLUMNS,
        CardId::StatTalkers => &TALKER_COLUMNS,
        CardId::StatProtocols => &PROTOCOL_COLUMNS,
        CardId::StatFlows => &FLOW_COLUMNS,
        CardId::StatBookmarks => &BOOKMARK_COLUMNS,
        CardId::ThreatTable => &THREAT_COLUMNS,
        CardId::ConnTable => &CONN_COLUMNS,
        CardId::ProcTable => &PROC_COLUMNS,
        CardId::DomainTable => &DOMAIN_COLUMNS,
        CardId::AlertTable => &ALERT_COLUMNS,
        CardId::HistoryTable => &HISTORY_COLUMNS,
        _ => &[],
    }
}

// Sort kinds of a card's columns, in column order.
pub fn card_sort_kinds(card: CardId) -> Vec<SortKind> {
    card_columns(card).iter().map(|def| def.kind).collect()
}
