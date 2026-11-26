//! Capture TUI state and its transitions: the packet ring and filters, the
//! selection and follow mode, key handling that returns a `Command`, the sort
//! order cache, and the statistics cache.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use network_monitor_core::analysis::ThreatTracker;
use network_monitor_core::decode::{DecodedPacket, Direction as PktDir};
use network_monitor_core::enrich::names::SharedNameResolver;
use network_monitor_core::filter::ast::FilterExpr;
use network_monitor_core::filter::eval::eval_filter;
use network_monitor_core::filter::parser::parse_filter;
use network_monitor_core::storage::ring::PacketRing;

use super::capture_io::PacketWriter;
use super::cards::{CardId, CardKind, CursorMotion, UiAction};
use super::columns::PACKET_COLUMNS;
use super::i18n::{self, Msg};
use super::interaction::{MouseOutcome, ToastLevel, Ui};
use super::layout::SplitGroup;
use super::level::{OutputLevel, packet_level};
use super::sort::{SortChange, SortKey, sorted_positions};
use super::stats::{self, StatsCache};
use super::tab_move::TabDrag;

pub const COLUMN_COUNT: usize = PACKET_COLUMNS.len();
const THROUGHPUT_WINDOW: usize = 60;
const STATS_REFRESH: Duration = Duration::from_millis(500);
const ORDER_REFRESH: Duration = Duration::from_millis(500);
// A live feed with no packet for this long reads as idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(3);

// 1. modes and commands -------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    FilterInput,
    Search,
    // Digit keys 1-8 toggle the packet-list columns.
    ColumnConfig,
}

// Tabs of the capture screen, in the default tab-bar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTab {
    Packets,
    Flows,
    Stats,
    Alerts,
    Stream,
    Settings,
}

impl CaptureTab {
    pub const ALL: [CaptureTab; 6] = [
        CaptureTab::Packets,
        CaptureTab::Flows,
        CaptureTab::Stats,
        CaptureTab::Alerts,
        CaptureTab::Stream,
        CaptureTab::Settings,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    // Card that takes the keys when the tab opens.
    pub fn main_card(self) -> CardId {
        match self {
            Self::Packets => CardId::PacketList,
            Self::Flows => CardId::StatFlows,
            Self::Stats => CardId::StatTalkers,
            Self::Alerts => CardId::ThreatTable,
            Self::Stream => CardId::StatStream,
            Self::Settings => CardId::SettingsList,
        }
    }

    // Reorderable card group of the tab, if it has two cards.
    pub fn group(self) -> Option<SplitGroup> {
        match self {
            Self::Packets => Some(SplitGroup::CaptureCols),
            Self::Flows => Some(SplitGroup::CaptureFlows),
            Self::Stats => Some(SplitGroup::CaptureStats),
            Self::Alerts => Some(SplitGroup::CaptureAlerts),
            Self::Stream | Self::Settings => None,
        }
    }

    // Table cards whose column widths `l` resets on this tab.
    pub fn table_cards(self) -> &'static [CardId] {
        match self {
            Self::Packets => &[CardId::PacketList],
            Self::Flows => &[CardId::StatFlows],
            Self::Stats => &[CardId::StatTalkers, CardId::StatProtocols],
            Self::Alerts => &[CardId::ThreatTable, CardId::StatBookmarks],
            Self::Stream | Self::Settings => &[],
        }
    }

    // Stable name the state file stores (enum numbers are never written).
    pub fn key(self) -> &'static str {
        match self {
            Self::Packets => "packets",
            Self::Flows => "flows",
            Self::Stats => "stats",
            Self::Alerts => "alerts",
            Self::Stream => "stream",
            Self::Settings => "settings",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.key() == key)
    }
}

// What the host must do after a transition: I/O stays out of the state code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    None,
    SaveState,
    SaveLog,
}

// Outcomes of background work.
pub enum UiMessage {
    LogSaved(Result<(usize, PathBuf), String>),
    // The state writer thread failed to write `tui.toml` (raw error text).
    StateSaveFailed { detail: String },
}

// Where packets come from: the title of the app bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSource {
    Interface(String),
    File(String),
}

// 2. state --------------------------------------------------------------------
pub struct App {
    pub ui: Ui,
    pub source: CaptureSource,
    pub packets: PacketRing,
    pub filtered_indices: Option<Vec<usize>>,
    pub active_filter: Option<FilterExpr>,
    pub dir_filter: Option<PktDir>,
    pub mode: InputMode,
    // Index into the visible (filtered) packets, in capture order.
    pub selected: usize,
    pub paused: bool,
    // LIVE: the view is pinned to the newest packet.
    pub follow: bool,
    pub should_quit: bool,
    pub total_received: u64,
    pub total_dropped: u64,
    pub last_packet_at: Option<Instant>,
    pub filter_input: String,
    pub filter_cursor: usize,
    pub search_query: String,
    pub search_re: Option<regex::Regex>,
    pub writer: Option<PacketWriter>,
    pub tab: CaptureTab,
    // An open tab press or drag (the order itself lives in `ui.tab_order`).
    pub tab_drag: Option<TabDrag>,
    pub stream: Option<(String, network_monitor_core::flow::StreamData)>,
    pub throughput_pps: VecDeque<u64>,
    pub throughput_bps: VecDeque<u64>,
    pub last_tick: Instant,
    pub tick_pkts: u64,
    pub tick_bytes: u64,
    pub proto_counts: BTreeMap<String, u64>,
    pub saved_filters: HashMap<String, String>,
    pub bookmarks: HashSet<u64>,
    pub show_detail: bool,
    pub show_hex: bool,
    pub col_visible: [bool; COLUMN_COUNT],
    pub threat_tracker: ThreatTracker,
    pub local_ips: Vec<IpAddr>,
    pub names: SharedNameResolver,
    pub stats: StatsCache,
    pub stats_dirty: bool,
    pub stats_built: Option<Instant>,
    // Display row -> visible index while a sort is active (capture order otherwise).
    pub order: Vec<usize>,
    pub order_dirty: bool,
    // Packets arrived (or the ring evicted) while a sort was active: the order
    // is rebuilt at the next ORDER_REFRESH tick, not on every packet.
    pub order_pending: bool,
    pub order_built: Option<Instant>,
    pub last_selected_number: Option<u64>,
    // Time of the last loop turn; views read it for idle and stale markers.
    pub clock: Instant,
}

impl App {
    pub fn new(buffer_size: usize, state_file: Option<PathBuf>, source: CaptureSource) -> Self {
        let mut ui = Ui::new(state_file);
        ui.default_focus = CardId::PacketList;
        Self {
            ui,
            source,
            packets: PacketRing::new(buffer_size),
            filtered_indices: None,
            active_filter: None,
            dir_filter: None,
            mode: InputMode::Normal,
            selected: 0,
            paused: false,
            follow: true,
            should_quit: false,
            total_received: 0,
            total_dropped: 0,
            last_packet_at: None,
            filter_input: String::new(),
            filter_cursor: 0,
            search_query: String::new(),
            search_re: None,
            writer: None,
            tab: CaptureTab::Packets,
            tab_drag: None,
            stream: None,
            throughput_pps: VecDeque::new(),
            throughput_bps: VecDeque::new(),
            last_tick: Instant::now(),
            tick_pkts: 0,
            tick_bytes: 0,
            proto_counts: BTreeMap::new(),
            saved_filters: HashMap::new(),
            bookmarks: HashSet::new(),
            show_detail: true,
            show_hex: true,
            col_visible: [true; COLUMN_COUNT],
            threat_tracker: ThreatTracker::new(),
            local_ips: Vec::new(),
            names: SharedNameResolver::new(),
            stats: StatsCache::default(),
            stats_dirty: true,
            stats_built: None,
            order: Vec::new(),
            order_dirty: false,
            order_pending: false,
            order_built: None,
            last_selected_number: None,
            clock: Instant::now(),
        }
    }

    // 3. packet access --------------------------------------------------------
    pub fn visible_count(&self) -> usize {
        match &self.filtered_indices {
            Some(indices) => indices.len(),
            None => self.packets.len(),
        }
    }

    pub fn visible_packet(&self, visible_idx: usize) -> Option<&DecodedPacket> {
        match &self.filtered_indices {
            Some(indices) => indices
                .get(visible_idx)
                .and_then(|&ring_idx| self.packets.get(ring_idx)),
            None => self.packets.get(visible_idx),
        }
    }

    pub fn selected_packet(&self) -> Option<&DecodedPacket> {
        self.visible_packet(self.selected)
    }

    // Whether a packet passes the text filter, the direction filter, and the
    // output level (the level pass is skipped at `debug`, which shows all).
    fn passes(&self, packet: &DecodedPacket) -> bool {
        let text_ok = self
            .active_filter
            .as_ref()
            .is_none_or(|filter| eval_filter(filter, packet));
        let dir_ok = self
            .dir_filter
            .is_none_or(|wanted| packet.direction(&self.local_ips) == wanted);
        let min_level = self.ui.min_level();
        let level_ok = min_level == OutputLevel::Debug || packet_level(packet) >= min_level;
        text_ok && dir_ok && level_ok
    }

    pub fn rebuild_filter_indices(&mut self) {
        if self.active_filter.is_none()
            && self.dir_filter.is_none()
            && self.ui.min_level() == OutputLevel::Debug
        {
            self.filtered_indices = None;
        } else {
            let indices: Vec<usize> = (0..self.packets.len())
                .filter(|&index| {
                    self.packets
                        .get(index)
                        .is_some_and(|packet| self.passes(packet))
                })
                .collect();
            self.filtered_indices = Some(indices);
        }
        self.order_dirty = true;
        self.stats_dirty = true;
    }

    // 4. sort order -----------------------------------------------------------
    // Display row -> visible index (capture order when no sort is active).
    pub fn display_to_visible(&self, row: usize) -> usize {
        if self.ui.sorts[CardId::PacketList as usize].is_some() {
            self.order.get(row).copied().unwrap_or(row)
        } else {
            row
        }
    }

    pub fn visible_to_display(&self, visible_idx: usize) -> usize {
        if self.ui.sorts[CardId::PacketList as usize].is_some() {
            self.order
                .iter()
                .position(|candidate| *candidate == visible_idx)
                .unwrap_or(visible_idx)
        } else {
            visible_idx
        }
    }

    // Sort key of one packet for a packet-list column (typed, never the label).
    pub fn packet_sort_key(&self, packet: &DecodedPacket, column: usize) -> SortKey {
        match column {
            0 => SortKey::Num(packet.timestamp.timestamp_micros()),
            1 => SortKey::Text(match packet.direction(&self.local_ips) {
                PktDir::In => "IN".into(),
                PktDir::Out => "OUT".into(),
                PktDir::Unknown => String::new(),
            }),
            2 => SortKey::Text(
                packet
                    .process
                    .as_ref()
                    .map_or(String::new(), |p| p.name.clone()),
            ),
            3 => {
                let name = super::views::packets::remote_endpoint(packet, &self.local_ips)
                    .0
                    .and_then(|ip| self.names.resolve(ip))
                    .map(|entry| entry.name)
                    .unwrap_or_default();
                SortKey::Text(name)
            }
            4 => {
                let (ip, port) = super::views::packets::remote_endpoint(packet, &self.local_ips);
                SortKey::Text(match (ip, port) {
                    (Some(ip), Some(port)) => format!("{ip}:{port}"),
                    (Some(ip), None) => ip.to_string(),
                    _ => String::new(),
                })
            }
            5 => SortKey::Text(packet.summary.protocol.clone()),
            6 => SortKey::Num(i64::from(packet.summary.length)),
            _ => SortKey::Text(packet.summary.info.clone()),
        }
    }

    // Rebuilds the display order when a sort is active and it is stale; new
    // packets and ring evictions alone refresh it at most every ORDER_REFRESH
    // (`drain_packets` keeps the rows already ordered valid in between).
    pub fn refresh_order(&mut self, now: Instant) {
        let Some(sort) = self.ui.sorts[CardId::PacketList as usize] else {
            self.order.clear();
            self.order_dirty = false;
            self.order_pending = false;
            return;
        };
        let count = self.visible_count();
        let refresh_due = self
            .order_built
            .is_none_or(|built| now.saturating_duration_since(built) >= ORDER_REFRESH);
        let stale = (self.order_pending || self.order.len() != count) && refresh_due;
        if !self.order_dirty && !stale {
            return;
        }
        let keys: Vec<SortKey> = (0..count)
            .map(|index| {
                self.visible_packet(index)
                    .map_or(SortKey::Num(0), |packet| {
                        self.packet_sort_key(packet, sort.column)
                    })
            })
            .collect();
        self.order = sorted_positions(&keys, sort.descending);
        self.order_dirty = false;
        self.order_pending = false;
        self.order_built = Some(now);
    }

    // 5. throughput and stats -------------------------------------------------
    pub fn tick_throughput(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_tick) >= Duration::from_secs(1) {
            push_capped(&mut self.throughput_pps, self.tick_pkts, THROUGHPUT_WINDOW);
            push_capped(&mut self.throughput_bps, self.tick_bytes, THROUGHPUT_WINDOW);
            self.tick_pkts = 0;
            self.tick_bytes = 0;
            self.last_tick = now;
        }
    }

    pub fn refresh_stats(&mut self, now: Instant) {
        if !matches!(
            self.tab,
            CaptureTab::Flows | CaptureTab::Stats | CaptureTab::Alerts
        ) {
            return;
        }
        let due = self
            .stats_built
            .is_none_or(|built| now.saturating_duration_since(built) >= STATS_REFRESH);
        if !self.stats_dirty && !due {
            return;
        }
        self.stats = stats::build_stats(
            self.packets.iter(),
            &self.bookmarks,
            &self.proto_counts,
            self.ui.min_level(),
        );
        self.stats_dirty = false;
        self.stats_built = Some(now);
    }

    // One call per loop turn: timers, caches, toast expiry; true when a redraw is due.
    pub fn on_tick(&mut self, now: Instant) -> bool {
        let before = (
            self.throughput_pps.len(),
            self.stats_built,
            self.order_built,
        );
        let was_idle = self.is_idle();
        self.clock = now;
        self.tick_throughput(now);
        self.refresh_stats(now);
        self.refresh_order(now);
        let mut dirty = self.ui.tick(now);
        dirty |= self.ui.expire_toast(now);
        dirty
            || was_idle != self.is_idle()
            || before
                != (
                    self.throughput_pps.len(),
                    self.stats_built,
                    self.order_built,
                )
    }

    // A live feed that has delivered packets but none for IDLE_AFTER.
    pub fn is_idle(&self) -> bool {
        self.last_packet_at
            .is_some_and(|last| self.clock.saturating_duration_since(last) >= IDLE_AFTER)
    }

    // Longest the loop may wait for input: a pending long press or toast expiry.
    pub fn next_wake(&self, now: Instant, base: Duration) -> Duration {
        let mut wait = base;
        if let Some(remaining) = self.ui.press_remaining(now) {
            wait = wait.min(remaining);
        }
        if let Some(toast) = &self.ui.toast {
            wait = wait.min(toast.expires_at.saturating_duration_since(now));
        }
        wait
    }

    // 6. packets in -----------------------------------------------------------
    pub fn drain_packets(&mut self, rx: &Receiver<DecodedPacket>, now: Instant) {
        // A level above debug restored from the state file needs the visible
        // index list before the first packet is placed.
        if self.filtered_indices.is_none() && self.ui.min_level() != OutputLevel::Debug {
            self.rebuild_filter_indices();
        }
        let mut arrived = false;
        // Visible rows the ring pushed out of the front during this batch.
        let mut evicted_visible = 0usize;
        while let Ok(mut packet) = rx.try_recv() {
            arrived = true;
            self.total_received += 1;
            self.tick_pkts += 1;
            self.tick_bytes += u64::from(packet.wire_len);
            self.last_packet_at = Some(now);
            // Annotate before filtering so the `threat` filter, the row tint, and
            // the Host column all see it.
            packet.threat = self.threat_tracker.observe(&packet);
            self.names.observe(&packet);
            *self
                .proto_counts
                .entry(packet.summary.protocol.clone())
                .or_insert(0) += 1;
            if let Some(writer) = &self.writer {
                writer.queue_packet(&packet);
            }
            let matches_filter = self.passes(&packet);
            let ring_idx = self.packets.len();
            let evicted = self.packets.push(packet);
            if evicted.is_some() {
                match self.filtered_indices.as_mut() {
                    Some(indices) => {
                        evicted_visible += usize::from(indices.first() == Some(&0));
                        indices.retain_mut(|index| {
                            if *index == 0 {
                                false
                            } else {
                                *index -= 1;
                                true
                            }
                        });
                    }
                    None => evicted_visible += 1,
                }
            }
            if matches_filter && let Some(indices) = self.filtered_indices.as_mut() {
                let actual = if evicted.is_some() {
                    ring_idx.min(self.packets.len() - 1)
                } else {
                    ring_idx
                };
                indices.push(actual);
            }
        }
        // With a sort active, the display order holds visible indices, and an
        // eviction shifts every index down: drop the evicted rows and shift the
        // rest once per batch so the rows keep their order, then let the
        // throttled refresh place the new packets.
        if arrived && self.ui.sorts[CardId::PacketList as usize].is_some() {
            if evicted_visible > 0 {
                self.order.retain_mut(|index| {
                    if *index < evicted_visible {
                        false
                    } else {
                        *index -= evicted_visible;
                        true
                    }
                });
            }
            self.order_pending = true;
        }
        // LIVE pins to the newest packet. BROWSE keeps the same packet selected:
        // eviction shifts every visible index down, and a selected packet that
        // left the ring falls back to the oldest row.
        let count = self.visible_count();
        if self.follow {
            if count > 0 {
                self.selected = count - 1;
            }
        } else {
            self.selected = self.selected.saturating_sub(evicted_visible);
            self.selected = self.selected.min(count.saturating_sub(1));
        }
    }

    pub fn apply_message(&mut self, message: UiMessage) {
        match message {
            UiMessage::LogSaved(Ok((count, path))) => {
                let text = i18n::log_saved(self.ui.lang, count, &path.display().to_string());
                self.ui.show_toast(&text, ToastLevel::Success);
            }
            UiMessage::LogSaved(Err(detail)) => {
                let text = i18n::log_save_failed(self.ui.lang, &detail);
                self.ui.show_toast(&text, ToastLevel::Error);
            }
            UiMessage::StateSaveFailed { detail } => self.ui.state_save_failed(&detail),
        }
    }

    // Copies of the visible packets for the background log export.
    pub fn visible_packets_cloned(&self) -> Vec<DecodedPacket> {
        (0..self.visible_count())
            .filter_map(|index| self.visible_packet(index).cloned())
            .collect()
    }

    // 7. selection ------------------------------------------------------------
    fn toast(&mut self, msg: Msg, level: ToastLevel) {
        let text = i18n::text(self.ui.lang, msg);
        self.ui.show_toast(text, level);
    }

    // Keyboard motion over the packet list: moves the selected packet, keeps it
    // in view, and re-enters LIVE only when it lands on the newest row.
    pub fn move_packet_selection(&mut self, motion: CursorMotion) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        let row = self.visible_to_display(self.selected);
        let page = self.ui.views[CardId::PacketList as usize].visible.max(1) as isize;
        let target = match motion {
            CursorMotion::Lines(delta) => row.saturating_add_signed(delta),
            CursorMotion::Pages(pages) => row.saturating_add_signed(pages * page),
            CursorMotion::Home => 0,
            CursorMotion::End => count - 1,
        }
        .min(count - 1);
        self.selected = self.display_to_visible(target);
        self.follow = target + 1 >= count && self.ui.sorts[CardId::PacketList as usize].is_none();
        self.ui.views[CardId::PacketList as usize].total = count;
        self.ui.select_row(CardId::PacketList, target);
    }

    fn select_clicked_row(&mut self, card: CardId, index: usize) {
        if card == CardId::PacketList {
            let count = self.visible_count();
            if index < count {
                self.selected = self.display_to_visible(index);
                // A click surfaces the detail pane and leaves LIVE.
                self.show_detail = true;
                self.follow = false;
            }
        }
        self.ui.select_row(card, index);
    }

    fn change_sort(&mut self, card: CardId, change: SortChange) {
        if !self.ui.change_sort(card, change) {
            return;
        }
        if card == CardId::PacketList {
            self.order_dirty = true;
            if self.ui.sorts[card as usize].is_some() {
                self.follow = false;
            }
            self.refresh_order(Instant::now());
        }
    }

    // Clears any card move, commits a drag that had moved (its release may
    // never arrive once the tab changes, so it is saved now), gives the keys to
    // the tab's main card, and asks for fresh statistics on a statistics tab.
    pub fn switch_tab(&mut self, tab: CaptureTab) -> Command {
        self.ui.cancel_card_move();
        let drag_moved = self.ui.commit_drags();
        // The input row lives on the Packets tab: leaving it ends any input.
        if tab != CaptureTab::Packets {
            self.mode = InputMode::Normal;
        }
        self.tab = tab;
        self.ui.context = tab.index();
        self.ui.default_focus = tab.main_card();
        self.ui.focus = None;
        if matches!(
            tab,
            CaptureTab::Flows | CaptureTab::Stats | CaptureTab::Alerts
        ) {
            self.stats_dirty = true;
        }
        if drag_moved {
            Command::SaveState
        } else {
            Command::None
        }
    }

    fn open_stream(&mut self) -> Command {
        let built = self.selected_packet().and_then(|selected| {
            let packets = self.packets.iter();
            stats::build_stream(selected, packets)
        });
        match built {
            Some(stream) => {
                let (client, server) = (stream.client, stream.server);
                let title = format!("{}:{} <-> {}:{}", client.0, client.1, server.0, server.1);
                self.stream = Some((title, stream.data));
                self.switch_tab(CaptureTab::Stream)
            }
            None => {
                self.toast(Msg::ToastSelectTcp, ToastLevel::Error);
                Command::None
            }
        }
    }

    fn toggle_bookmark(&mut self) {
        if let Some(number) = self.selected_packet().map(|packet| packet.number)
            && !self.bookmarks.insert(number)
        {
            self.bookmarks.remove(&number);
        }
        self.stats_dirty = true;
    }

    fn next_search_match(&mut self) {
        let Some(re) = self.search_re.clone() else {
            return;
        };
        let count = self.visible_count();
        for offset in 1..=count {
            let index = (self.selected + offset) % count.max(1);
            if let Some(packet) = self.visible_packet(index)
                && (re.is_match(&packet.summary.info)
                    || re.is_match(&packet.summary.source)
                    || re.is_match(&packet.summary.destination))
            {
                self.selected = index;
                self.follow = false;
                let row = self.visible_to_display(index);
                self.ui.select_row(CardId::PacketList, row);
                return;
            }
        }
    }

    fn set_direction_filter(&mut self, wanted: PktDir) {
        self.dir_filter = if self.dir_filter == Some(wanted) {
            None
        } else {
            Some(wanted)
        };
        self.rebuild_filter_indices();
        self.selected = 0;
        let msg = match (self.dir_filter, wanted) {
            (None, _) => Msg::ToastDirCleared,
            (Some(_), PktDir::Out) => Msg::ToastDirOut,
            (Some(_), _) => Msg::ToastDirIn,
        };
        self.toast(msg, ToastLevel::Info);
    }

    // 8. actions shared by keys and chips -------------------------------------
    pub fn perform_action(&mut self, action: UiAction) -> Command {
        match action {
            UiAction::Quit => {
                self.should_quit = true;
                Command::None
            }
            UiAction::ShowHelp => {
                self.ui.show_help = true;
                Command::None
            }
            UiAction::ToggleLanguage => {
                self.ui.toggle_language();
                Command::SaveState
            }
            UiAction::ResetLayout => {
                // Order, sizes, and column widths of the current tab only; the
                // hex pane split belongs to the Packets tab as well.
                let tab = self.tab;
                let mut groups: Vec<SplitGroup> = tab.group().into_iter().collect();
                if tab == CaptureTab::Packets {
                    groups.push(SplitGroup::CaptureRows);
                }
                self.ui.reset_layout(&groups, tab.table_cards());
                Command::SaveState
            }
            UiAction::CycleLevel => {
                self.ui.cycle_level();
                self.rebuild_filter_indices();
                let count = self.visible_count();
                self.selected = if self.follow {
                    count.saturating_sub(1)
                } else {
                    0
                };
                let text = i18n::output_level_set(self.ui.lang, self.ui.min_level().code());
                self.ui.show_toast(&text, ToastLevel::Info);
                Command::SaveState
            }
            UiAction::TogglePause => {
                self.paused = !self.paused;
                let msg = if self.paused {
                    Msg::ToastPaused
                } else {
                    Msg::ToastResumed
                };
                self.toast(msg, ToastLevel::Info);
                Command::None
            }
            // Input modes draw their row on the Packets tab, so they open it first.
            UiAction::OpenFilter => {
                let command = self.switch_tab(CaptureTab::Packets);
                self.mode = InputMode::FilterInput;
                self.filter_input.clear();
                self.filter_cursor = 0;
                command
            }
            UiAction::OpenSearch => {
                let command = self.switch_tab(CaptureTab::Packets);
                self.mode = InputMode::Search;
                self.search_query.clear();
                command
            }
            UiAction::OpenColumns => {
                let command = self.switch_tab(CaptureTab::Packets);
                self.mode = InputMode::ColumnConfig;
                command
            }
            UiAction::SaveLog => {
                if self.visible_count() == 0 {
                    self.toast(Msg::ToastNoPackets, ToastLevel::Error);
                    Command::None
                } else {
                    self.toast(Msg::ToastSaveStarted, ToastLevel::Info);
                    Command::SaveLog
                }
            }
            UiAction::Refresh => Command::None,
            UiAction::ForceRestart => Command::None,
        }
    }

    // 9. keys -----------------------------------------------------------------
    pub fn handle_key(&mut self, key: KeyEvent) -> Command {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Command::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('f') {
            return self.perform_action(UiAction::OpenSearch);
        }
        // Help closes on any key; a card move or pending press only takes Esc.
        if self.ui.show_help {
            self.ui.show_help = false;
            return Command::None;
        }
        // Unmoved tab press ends; a moved tab drag takes only Esc.
        if self
            .tab_drag
            .is_some_and(|drag| drag.baseline == self.ui.tab_order)
        {
            self.tab_drag = None;
        }
        if self.tab_drag.is_some() {
            if key.code == KeyCode::Esc {
                self.cancel_tab_drag();
            }
            return Command::None;
        }
        if self.ui.card_move.is_some() || self.ui.card_press.is_some() {
            if key.code == KeyCode::Esc {
                self.ui.cancel_card_move();
            }
            return Command::None;
        }
        match self.mode {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::FilterInput => {
                self.handle_filter_key(key);
                Command::None
            }
            InputMode::Search => {
                self.handle_search_key(key);
                Command::None
            }
            InputMode::ColumnConfig => {
                self.handle_column_key(key);
                Command::None
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Command {
        let card = self.ui.focused_card();
        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        let slot = self
            .ui
            .tab_order
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        let count = self.ui.tab_order.len();
        let previous = self.ui.tab_order[(slot + count - 1) % count];
        let next = self.ui.tab_order[(slot + 1) % count];
        match key.code {
            KeyCode::Char('q') => self.perform_action(UiAction::Quit),
            KeyCode::Esc if self.tab != CaptureTab::Packets => self.switch_tab(CaptureTab::Packets),
            KeyCode::Esc => self.perform_action(UiAction::Quit),
            KeyCode::Char('?') => self.perform_action(UiAction::ShowHelp),
            KeyCode::BackTab => self.switch_tab(previous),
            KeyCode::Tab if shifted => self.switch_tab(previous),
            KeyCode::Tab => self.switch_tab(next),
            KeyCode::Left if !shifted => self.switch_tab(previous),
            KeyCode::Right if !shifted => self.switch_tab(next),
            KeyCode::Char(digit @ '1'..='6') => {
                self.switch_tab(self.ui.tab_order[digit as usize - '1' as usize])
            }
            KeyCode::Char('<') => self.move_active_tab(-1),
            KeyCode::Char('>') => self.move_active_tab(1),
            KeyCode::Enter if self.tab == CaptureTab::Settings => {
                // Row 0 is the output level, row 1 the language.
                let action = if self.ui.views[CardId::SettingsList as usize].selected == 0 {
                    UiAction::CycleLevel
                } else {
                    UiAction::ToggleLanguage
                };
                self.perform_action(action)
            }
            KeyCode::Enter => {
                // Keyboard path between the cards of the tab.
                self.ui.cycle_focus(1);
                Command::None
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right if shifted => {
                if self.ui.shift_card(card, key.code) {
                    Command::SaveState
                } else {
                    Command::None
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.step_card(card, CursorMotion::Lines(-1)),
            KeyCode::Down | KeyCode::Char('j') => self.step_card(card, CursorMotion::Lines(1)),
            KeyCode::PageUp => self.step_card(card, CursorMotion::Pages(-1)),
            KeyCode::PageDown => self.step_card(card, CursorMotion::Pages(1)),
            KeyCode::Home | KeyCode::Char('g') => self.step_card(card, CursorMotion::Home),
            KeyCode::End | KeyCode::Char('G') => self.step_card(card, CursorMotion::End),
            KeyCode::Char(letter @ ('s' | 'S')) => {
                let target = if super::columns::card_columns(card).is_empty() {
                    self.ui.default_focus
                } else {
                    card
                };
                let change = if letter == 's' {
                    SortChange::NextColumn
                } else {
                    SortChange::ReverseDirection
                };
                self.change_sort(target, change);
                Command::None
            }
            KeyCode::Char('t' | 'P') => self.switch_tab(CaptureTab::Stats),
            KeyCode::Char('F' | 'T') => self.switch_tab(CaptureTab::Flows),
            KeyCode::Char('\'') => self.switch_tab(CaptureTab::Alerts),
            KeyCode::Char('f') => self.open_stream(),
            KeyCode::Char('v') => self.perform_action(UiAction::CycleLevel),
            KeyCode::Char('m') => {
                self.toggle_bookmark();
                Command::None
            }
            KeyCode::Char('n') => {
                self.next_search_match();
                Command::None
            }
            KeyCode::Char('d') => {
                self.show_detail = !self.show_detail;
                Command::None
            }
            KeyCode::Char('x') => {
                self.show_hex = !self.show_hex;
                Command::None
            }
            KeyCode::Char('z') => {
                // Master toggle: hide both panes, or restore both when either is hidden.
                let any = self.show_detail || self.show_hex;
                self.show_detail = !any;
                self.show_hex = !any;
                Command::None
            }
            KeyCode::Char('c') => self.perform_action(UiAction::OpenColumns),
            KeyCode::Char('i') => {
                self.set_direction_filter(PktDir::In);
                Command::None
            }
            KeyCode::Char('o') => {
                self.set_direction_filter(PktDir::Out);
                Command::None
            }
            KeyCode::Char(' ') => self.perform_action(UiAction::TogglePause),
            KeyCode::Char('/') => self.perform_action(UiAction::OpenFilter),
            KeyCode::Char('w') => self.perform_action(UiAction::SaveLog),
            KeyCode::Char('l') => self.perform_action(UiAction::ResetLayout),
            KeyCode::Char('L') => self.perform_action(UiAction::ToggleLanguage),
            _ => Command::None,
        }
    }

    // Keyboard motion on the focused card: packet selection for the list, the
    // shared row or scroll motion for every other card.
    fn step_card(&mut self, card: CardId, motion: CursorMotion) -> Command {
        if card == CardId::PacketList {
            self.move_packet_selection(motion);
        } else if card.kind() == CardKind::Selectable || card.kind() == CardKind::Scrollable {
            self.ui.move_cursor(card, motion);
        }
        Command::None
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.apply_filter_input();
                self.mode = InputMode::Normal;
            }
            KeyCode::Esc => self.mode = InputMode::Normal,
            KeyCode::Backspace if self.filter_cursor > 0 => {
                self.filter_cursor -= prev_char_width(&self.filter_input, self.filter_cursor);
                self.filter_input.remove(self.filter_cursor);
            }
            KeyCode::Left if self.filter_cursor > 0 => {
                self.filter_cursor -= prev_char_width(&self.filter_input, self.filter_cursor);
            }
            KeyCode::Right if self.filter_cursor < self.filter_input.len() => {
                self.filter_cursor += next_char_width(&self.filter_input, self.filter_cursor);
            }
            KeyCode::Char(letter) => {
                self.filter_input.insert(self.filter_cursor, letter);
                self.filter_cursor += letter.len_utf8();
            }
            _ => {}
        }
    }

    // Enter in the filter bar: empty clears, `:name` recalls a saved filter,
    // anything else is parsed as a display filter.
    fn apply_filter_input(&mut self) {
        let lang = self.ui.lang;
        if self.filter_input.is_empty() {
            self.active_filter = None;
            self.rebuild_filter_indices();
            self.selected = 0;
            self.toast(Msg::ToastFilterCleared, ToastLevel::Info);
            return;
        }
        let expr = match self.filter_input.strip_prefix(':') {
            Some(name) => match self.saved_filters.get(name) {
                Some(saved) => saved.clone(),
                None => {
                    let text = i18n::no_saved_filter(lang, name);
                    self.ui.show_toast(&text, ToastLevel::Error);
                    return;
                }
            },
            None => self.filter_input.clone(),
        };
        match parse_filter(&expr) {
            Ok(filter) => {
                self.active_filter = Some(filter);
                self.rebuild_filter_indices();
                self.selected = 0;
                let text = i18n::filter_applied(lang, &expr);
                self.ui.show_toast(&text, ToastLevel::Info);
            }
            Err(error) => {
                let text = i18n::filter_error(lang, &error.to_string());
                self.ui.show_toast(&text, ToastLevel::Error);
            }
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                match regex::Regex::new(&self.search_query) {
                    Ok(re) => {
                        self.search_re = Some(re);
                        self.next_search_match();
                    }
                    Err(error) => {
                        let text = i18n::bad_regex(self.ui.lang, &error.to_string());
                        self.ui.show_toast(&text, ToastLevel::Error);
                    }
                }
                self.mode = InputMode::Normal;
            }
            KeyCode::Esc => {
                self.search_query.clear();
                self.search_re = None;
                self.mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char(letter) => self.search_query.push(letter),
            _ => {}
        }
    }

    fn handle_column_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('c') => self.mode = InputMode::Normal,
            KeyCode::Char(digit @ '1'..='8') => {
                let index = digit as usize - '1' as usize;
                self.col_visible[index] = !self.col_visible[index];
            }
            _ => {}
        }
    }

    // 10. mouse ---------------------------------------------------------------
    pub fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) -> Command {
        // 1. open tab drag: preview on drag, commit on release
        if self.tab_drag.is_some() && mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            self.preview_tab_move(Position::new(mouse.column, mouse.row));
            return Command::None;
        }
        if self.tab_drag.is_some() && mouse.kind == MouseEventKind::Up(MouseButton::Left) {
            return if self.finish_tab_drag() {
                Command::SaveState
            } else {
                Command::None
            };
        }

        // 2. lost release (buttonless move or new press) commits a moved drag
        let lost_release = matches!(
            mouse.kind,
            MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)
        );
        if lost_release && self.finish_tab_drag() {
            return Command::SaveState;
        }

        // 3. shared outcome; a tab press switches and opens a tab drag
        match self.ui.handle_mouse(mouse, now) {
            MouseOutcome::None => Command::None,
            MouseOutcome::SaveState => Command::SaveState,
            MouseOutcome::Action(action) => self.perform_action(action),
            MouseOutcome::Tab(index) => {
                let Some(tab) = self.ui.tab_order.get(index).copied() else {
                    return Command::None;
                };
                let command = self.switch_tab(tab);
                self.tab_drag = Some(TabDrag {
                    tab,
                    baseline: self.ui.tab_order,
                });
                command
            }
            MouseOutcome::Header { card, column } => {
                self.change_sort(card, SortChange::SelectColumn(column));
                Command::None
            }
            MouseOutcome::Row { card, index } => {
                self.select_clicked_row(card, index);
                Command::None
            }
            MouseOutcome::Scrolled(card) => {
                if card == CardId::PacketList {
                    // Manual scrolling leaves LIVE; reaching the end re-enters it.
                    let view = self.ui.views[card as usize];
                    self.follow = view.scroll + view.visible >= view.total
                        && self.ui.sorts[card as usize].is_none();
                }
                Command::None
            }
        }
    }

    // A tab drag that already moved the order commits like a lost release.
    pub fn handle_resize(&mut self) -> Command {
        let tab_moved = self.finish_tab_drag();
        if self.ui.handle_resize() || tab_moved {
            Command::SaveState
        } else {
            Command::None
        }
    }
}

fn push_capped(ring: &mut VecDeque<u64>, value: u64, cap: usize) {
    ring.push_back(value);
    while ring.len() > cap {
        ring.pop_front();
    }
}

// The filter cursor is a byte index that must stay on a UTF-8 boundary, so it
// moves by whole characters (a one-byte step panics on multibyte input).
fn prev_char_width(text: &str, at: usize) -> usize {
    text[..at]
        .chars()
        .next_back()
        .map_or(1, |letter| letter.len_utf8())
}

fn next_char_width(text: &str, at: usize) -> usize {
    text[at..]
        .chars()
        .next()
        .map_or(1, |letter| letter.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{
        blank_packet, capture_app, key, populate, shift_key, tcp_packet, threat_packet,
    };
    use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
    use network_monitor_core::analysis::ThreatKind;

    #[test]
    fn release_events_are_ignored_so_a_toggle_sticks() {
        let mut app = capture_app();
        let press =
            KeyEvent::new_with_kind(KeyCode::Char(' '), KeyModifiers::NONE, KeyEventKind::Press);
        app.handle_key(press);
        assert!(app.paused);
        // The host loop drops Release before it reaches the state; the contract
        // is tested in mod.rs. Here a second press toggles back.
        app.handle_key(press);
        assert!(!app.paused);
    }

    #[test]
    fn capital_l_switches_language_and_lowercase_l_resets_layout_keeping_it() {
        let mut app = capture_app();
        assert_eq!(app.ui.lang, i18n::Lang::En);
        assert_eq!(app.handle_key(key(KeyCode::Char('L'))), Command::SaveState);
        assert_eq!(app.ui.lang, i18n::Lang::Ko);
        app.ui.layout.move_card_to_slot(
            SplitGroup::CaptureCols,
            &crate::tui::layout::IDENTITY_ORDER,
            0,
            1,
        );
        assert_eq!(app.handle_key(key(KeyCode::Char('l'))), Command::SaveState);
        assert_eq!(app.ui.lang, i18n::Lang::Ko, "reset keeps the language");
        assert_eq!(app.ui.layout, crate::tui::layout::CardLayout::default());
        assert_eq!(app.handle_key(key(KeyCode::Char('L'))), Command::SaveState);
        assert_eq!(app.ui.lang, i18n::Lang::En);
    }

    #[test]
    fn w_saves_the_log_and_s_no_longer_does() {
        let mut app = capture_app();
        assert_eq!(app.handle_key(key(KeyCode::Char('w'))), Command::None);
        assert!(
            app.ui
                .toast
                .as_ref()
                .is_some_and(|t| t.level == ToastLevel::Error)
        );
        populate(&mut app, 3);
        assert_eq!(app.handle_key(key(KeyCode::Char('w'))), Command::SaveLog);
        assert_ne!(app.handle_key(key(KeyCode::Char('s'))), Command::SaveLog);
        assert_eq!(app.visible_packets_cloned().len(), 3);
    }

    #[test]
    fn q_esc_and_ctrl_c_quit_on_packets_while_esc_on_another_tab_returns_to_packets() {
        for quit in [
            key(KeyCode::Char('q')),
            key(KeyCode::Esc),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut app = capture_app();
            app.handle_key(quit);
            assert!(app.should_quit);
        }
        let mut app = capture_app();
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.tab, CaptureTab::Stats);
        assert_eq!(app.ui.default_focus, CardId::StatTalkers);
        app.handle_key(key(KeyCode::Esc));
        assert!(app.tab == CaptureTab::Packets && !app.should_quit);
        assert_eq!(app.ui.default_focus, CardId::PacketList);
        for (letter, tab) in [
            ('P', CaptureTab::Stats),
            ('F', CaptureTab::Flows),
            ('T', CaptureTab::Flows),
            ('\'', CaptureTab::Alerts),
        ] {
            app.handle_key(key(KeyCode::Char(letter)));
            assert_eq!(app.tab, tab, "{letter}");
        }
    }

    #[test]
    fn help_swallows_one_key_so_the_closing_key_does_not_switch_tabs() {
        let mut app = capture_app();
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.ui.show_help);
        app.handle_key(key(KeyCode::Char('t')));
        assert!(!app.ui.show_help);
        assert_eq!(app.tab, CaptureTab::Packets, "the closing key is consumed");
    }

    #[test]
    fn v_cycles_the_output_level_and_the_list_keeps_only_packets_at_or_above_it() {
        let mut app = capture_app();
        let info = tcp_packet(
            1,
            [10, 0, 0, 1],
            40000,
            [93, 184, 216, 34],
            9000,
            80,
            b"GET / HTTP/1.1\r\nHost: example\r\n\r\n",
        );
        let mut retransmitted =
            tcp_packet(2, [10, 0, 0, 1], 40000, [93, 184, 216, 34], 443, 60, b"");
        retransmitted.retransmission = true;
        let fixtures = [
            blank_packet(0),
            info,
            retransmitted,
            threat_packet(3, ThreatKind::PortScan),
        ];
        let levels: Vec<OutputLevel> = fixtures.iter().map(packet_level).collect();
        assert_eq!(levels, OutputLevel::ALL, "one fixture per level");
        for packet in fixtures {
            app.packets.push(packet);
        }
        for expected in [OutputLevel::Info, OutputLevel::Warn, OutputLevel::Error] {
            assert_eq!(app.handle_key(key(KeyCode::Char('v'))), Command::SaveState);
            assert_eq!(app.ui.min_level(), expected);
            let shown: Vec<OutputLevel> = (0..app.visible_count())
                .map(|index| packet_level(app.visible_packet(index).unwrap()))
                .collect();
            assert_eq!(shown.len(), 4 - expected as usize, "{expected:?}");
            assert!(shown.iter().all(|level| *level >= expected), "{shown:?}");
        }
        let exported = app.visible_packets_cloned();
        assert_eq!(exported.len(), 1, "the log export holds only error packets");
        assert_eq!(exported[0].number, 3);
        assert_eq!(
            app.ui.toast.as_ref().map(|toast| toast.text.as_str()),
            Some("Output level: error+")
        );
        assert_eq!(app.handle_key(key(KeyCode::Char('v'))), Command::SaveState);
        assert_eq!(app.ui.min_level(), OutputLevel::Debug);
        assert!(app.filtered_indices.is_none());
        assert_eq!(app.visible_count(), 4);
    }

    #[test]
    fn a_level_restored_from_the_state_file_filters_the_first_drained_packets() {
        let mut app = capture_app();
        app.ui.level_choice = Some(OutputLevel::Warn);
        let (tx, rx) = crossbeam_channel::unbounded();
        // The drain re-annotates threats through the tracker, so the warn
        // packet is a retransmission.
        let mut retransmitted =
            tcp_packet(1, [10, 0, 0, 1], 40000, [93, 184, 216, 34], 443, 60, b"");
        retransmitted.retransmission = true;
        tx.send(blank_packet(0)).unwrap();
        tx.send(retransmitted).unwrap();
        app.drain_packets(&rx, Instant::now());
        assert_eq!(app.packets.len(), 2);
        assert_eq!(app.visible_count(), 1);
        assert_eq!(app.selected_packet().map(|packet| packet.number), Some(1));
    }

    #[test]
    fn l_on_the_flows_tab_resets_only_the_flows_group_and_its_table() {
        let mut app = capture_app();
        app.ui.level_choice = Some(OutputLevel::Warn);
        assert!(
            app.ui
                .layout
                .restore_ratios(SplitGroup::CaptureFlows, &[3_000])
        );
        assert!(
            app.ui
                .layout
                .restore_ratios(SplitGroup::CaptureCols, &[3_000])
        );
        app.ui.column_widths[CardId::StatFlows as usize][0] = Some(20);
        app.ui.column_widths[CardId::PacketList as usize][0] = Some(20);
        app.handle_key(key(KeyCode::Char('2')));
        assert_eq!(app.tab, CaptureTab::Flows);
        assert_eq!(app.handle_key(key(KeyCode::Char('l'))), Command::SaveState);
        assert!(!app.ui.layout.has_custom_groups(&[SplitGroup::CaptureFlows]));
        assert!(app.ui.layout.has_custom_groups(&[SplitGroup::CaptureCols]));
        assert_eq!(app.ui.column_widths[CardId::StatFlows as usize][0], None);
        assert_eq!(
            app.ui.column_widths[CardId::PacketList as usize][0],
            Some(20)
        );
        assert_eq!(app.ui.level_choice, Some(OutputLevel::Warn));
        assert_eq!(app.ui.lang, i18n::Lang::En);
    }

    #[test]
    fn selection_keys_move_in_capture_order_and_re_enter_live_at_the_end() {
        let mut app = capture_app();
        populate(&mut app, 30);
        app.selected = 29;
        app.ui.clamp_view(CardId::PacketList, 30, 10);
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected, 28);
        assert!(!app.follow);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 29);
        assert!(app.follow, "landing on the newest row re-enters LIVE");
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.selected, 0);
        assert!(!app.follow);
        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.selected, 29);
        assert!(app.follow);
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.selected, 19, "a page is the visible row count");
    }

    #[test]
    fn sort_keys_cycle_the_packet_list_to_typed_order_and_back_to_capture_order() {
        let mut app = capture_app();
        let lens = [10u32, 50, 30, 20, 40];
        for (index, len) in lens.iter().enumerate() {
            let packet = tcp_packet(
                index as u64,
                [10, 0, 0, 1],
                40000,
                [93, 184, 216, 34],
                443,
                *len,
                b"",
            );
            app.packets.push(packet);
        }
        // Column 6 is Len (numeric): first click descending.
        app.ui
            .change_sort(CardId::PacketList, SortChange::SelectColumn(6));
        app.order_dirty = true;
        app.refresh_order(Instant::now());
        let shown: Vec<u32> = app
            .order
            .iter()
            .map(|index| app.visible_packet(*index).unwrap().summary.length)
            .collect();
        assert_eq!(shown, [50, 40, 30, 20, 10]);
        assert_eq!(app.display_to_visible(0), 1);
        assert_eq!(app.visible_to_display(1), 0);
        // Third click resets to capture order.
        app.ui
            .change_sort(CardId::PacketList, SortChange::SelectColumn(6));
        app.ui
            .change_sort(CardId::PacketList, SortChange::SelectColumn(6));
        assert!(app.ui.sorts[CardId::PacketList as usize].is_none());
        assert_eq!(app.display_to_visible(3), 3);
    }

    #[test]
    fn s_and_upper_s_walk_the_same_cycle_as_the_header_click() {
        let mut app = capture_app();
        populate(&mut app, 3);
        app.handle_key(key(KeyCode::Char('s')));
        let first = app.ui.sorts[CardId::PacketList as usize].unwrap();
        assert_eq!(first.column, 0);
        assert!(first.descending, "the time column starts descending");
        app.handle_key(key(KeyCode::Char('S')));
        assert!(
            !app.ui.sorts[CardId::PacketList as usize]
                .unwrap()
                .descending
        );
        app.handle_key(key(KeyCode::Char('S')));
        assert!(app.ui.sorts[CardId::PacketList as usize].is_none());
    }

    #[test]
    fn direction_keys_filter_and_toggle_back() {
        let mut app = capture_app();
        populate(&mut app, 4);
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.dir_filter, Some(PktDir::In));
        assert!(app.filtered_indices.is_some());
        app.handle_key(key(KeyCode::Char('i')));
        assert!(app.dir_filter.is_none() && app.filtered_indices.is_none());
        app.handle_key(key(KeyCode::Char('o')));
        assert_eq!(app.dir_filter, Some(PktDir::Out));
    }

    #[test]
    fn multibyte_filter_input_keeps_char_boundaries() {
        let mut app = capture_app();
        app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(app.mode, InputMode::FilterInput);
        for letter in ['한', 'g', '글'] {
            app.handle_key(key(KeyCode::Char(letter)));
        }
        assert_eq!(app.filter_input, "한g글");
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Char('ä')));
        assert_eq!(app.filter_input, "한äg글");
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.filter_input, "한g글");
    }

    #[test]
    fn filter_enter_applies_clears_recalls_saved_names_and_reports_errors() {
        let mut app = capture_app();
        app.saved_filters
            .insert("https".into(), "tcp.port == 443".into());
        for text in ["tcp.port == 443", ":https"] {
            app.mode = InputMode::FilterInput;
            app.filter_input = text.into();
            app.handle_key(key(KeyCode::Enter));
            assert!(app.active_filter.is_some(), "{text}");
            assert_eq!(app.mode, InputMode::Normal);
        }
        app.mode = InputMode::FilterInput;
        app.filter_input = ":missing".into();
        app.handle_key(key(KeyCode::Enter));
        let toast = app.ui.toast.as_ref().unwrap();
        assert_eq!(toast.level, ToastLevel::Error);
        assert!(toast.text.contains(":missing"));
        app.filter_input = "((".into();
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.ui.toast.as_ref().unwrap().level, ToastLevel::Error);
        app.mode = InputMode::FilterInput;
        app.filter_input.clear();
        app.handle_key(key(KeyCode::Enter));
        assert!(app.active_filter.is_none());
    }

    #[test]
    fn column_editor_toggles_columns_with_digits_and_closes_with_escape() {
        let mut app = capture_app();
        app.handle_key(key(KeyCode::Char('c')));
        assert_eq!(app.mode, InputMode::ColumnConfig);
        app.handle_key(key(KeyCode::Char('3')));
        assert!(!app.col_visible[2]);
        app.handle_key(key(KeyCode::Char('3')));
        assert!(app.col_visible[2]);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.mode, InputMode::Normal);
    }

    #[test]
    fn enter_cycles_focus_between_drawn_cards_and_shift_arrows_reorder() {
        let mut app = capture_app();
        let list = ratatui::layout::Rect::new(0, 2, 40, 10);
        let detail = ratatui::layout::Rect::new(41, 2, 39, 10);
        app.ui.hits.card_rects = vec![(CardId::PacketList, list), (CardId::PacketDetail, detail)];
        app.ui
            .hits
            .slot_groups
            .push(crate::tui::cards::SlotGroupHit {
                group: SplitGroup::CaptureCols,
                slots: vec![list, detail],
            });
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.ui.focused_card(), CardId::PacketDetail);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.ui.focused_card(), CardId::PacketList);
        assert_eq!(app.tab, CaptureTab::Packets, "Enter never switches tabs");
        assert_eq!(
            app.handle_key(shift_key(KeyCode::Right)),
            Command::SaveState
        );
        assert_eq!(app.ui.layout.order_of(SplitGroup::CaptureCols)[..2], [1, 0]);
    }

    #[test]
    fn browse_selection_is_clamped_when_the_ring_evicts() {
        let mut app = capture_app();
        app.packets = PacketRing::new(4);
        populate(&mut app, 4);
        app.follow = false;
        app.selected = 3;
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(tx);
        app.drain_packets(&rx, Instant::now());
        assert!(app.selected < app.visible_count().max(1));
        app.selected = 3;
        app.filtered_indices = Some(vec![0]);
        app.drain_packets(&rx, Instant::now());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn drain_follows_the_newest_packet_in_live_mode_and_records_arrival() {
        let mut app = capture_app();
        let (tx, rx) = crossbeam_channel::unbounded();
        for number in 0..3 {
            tx.send(crate::tui::test_support::blank_packet(number))
                .unwrap();
        }
        let now = Instant::now();
        app.drain_packets(&rx, now);
        assert_eq!(app.total_received, 3);
        assert_eq!(app.selected, 2);
        assert_eq!(app.last_packet_at, Some(now));
        assert_eq!(app.proto_counts.values().sum::<u64>(), 3);
    }

    #[test]
    fn mouse_header_click_sorts_row_click_selects_and_wheel_leaves_live() {
        let mut app = capture_app();
        populate(&mut app, 40);
        let area = ratatui::layout::Rect::new(0, 2, 80, 14);
        app.ui.hits.header_cells.push(crate::tui::cards::HeaderHit {
            card: CardId::PacketList,
            column: 6,
            rect: ratatui::layout::Rect::new(60, 3, 5, 1),
        });
        app.ui.hits.row_hits.push(crate::tui::cards::RowHit {
            card: CardId::PacketList,
            rect: ratatui::layout::Rect::new(2, 4, 70, 10),
            offset: 0,
            len: 40,
        });
        app.ui.hits.card_rects.push((CardId::PacketList, area));
        let now = Instant::now();
        let click = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(click(61, 3), now);
        assert!(app.ui.sorts[CardId::PacketList as usize].is_some());
        assert!(!app.follow);
        app.ui.sorts[CardId::PacketList as usize] = None;
        app.handle_mouse(click(10, 6), now);
        assert_eq!(app.selected, 2);
        assert!(app.show_detail && !app.follow);
    }

    #[test]
    fn log_save_outcomes_become_typed_toasts_in_the_active_language() {
        let mut app = capture_app();
        app.apply_message(UiMessage::LogSaved(Ok((3, PathBuf::from("/x/a.log")))));
        let toast = app.ui.toast.as_ref().unwrap();
        assert_eq!(toast.level, ToastLevel::Success);
        assert!(toast.text.contains("/x/a.log") && toast.text.starts_with("Saved"));
        app.ui.toggle_language();
        app.apply_message(UiMessage::LogSaved(Err("disk full".into())));
        let toast = app.ui.toast.as_ref().unwrap();
        assert_eq!(toast.level, ToastLevel::Error);
        assert!(toast.text.contains("disk full") && toast.text.contains("실패"));
    }

    #[test]
    fn next_wake_caps_the_wait_at_the_hold_remainder_and_toast_expiry() {
        let mut app = capture_app();
        let now = Instant::now();
        let base = Duration::from_millis(100);
        assert_eq!(app.next_wake(now, base), base);
        app.ui.card_press = Some(crate::tui::cards::CardPress {
            card: CardId::PacketList,
            group: SplitGroup::CaptureCols,
            index: 0,
            started: now,
            cell: ratatui::layout::Position::new(1, 1),
        });
        assert_eq!(app.next_wake(now, base), base);
        assert_eq!(
            app.next_wake(now + Duration::from_millis(350), base),
            Duration::from_millis(50)
        );
    }

    // Lengths of the rows as displayed (display row -> visible packet).
    fn shown_lengths(app: &App) -> Vec<u32> {
        (0..app.visible_count())
            .map(|row| {
                app.visible_packet(app.display_to_visible(row))
                    .unwrap()
                    .summary
                    .length
            })
            .collect()
    }

    fn send_lengths(tx: &crossbeam_channel::Sender<DecodedPacket>, first: u64, lengths: &[u32]) {
        for (offset, len) in lengths.iter().enumerate() {
            let packet = tcp_packet(
                first + offset as u64,
                [10, 0, 0, 1],
                40000,
                [93, 184, 216, 34],
                443,
                *len,
                b"",
            );
            tx.send(packet).unwrap();
        }
    }

    #[test]
    fn a_sorted_list_on_a_full_ring_keeps_its_rows_and_click_selection_through_eviction() {
        let mut app = capture_app();
        app.packets = PacketRing::new(4);
        let (tx, rx) = crossbeam_channel::unbounded();
        let start = Instant::now();
        send_lengths(&tx, 0, &[10, 40, 20, 30]);
        app.drain_packets(&rx, start);
        // Column 6 is Len (numeric): the first click sorts descending.
        app.change_sort(CardId::PacketList, SortChange::SelectColumn(6));
        assert_eq!(shown_lengths(&app), [40, 30, 20, 10]);

        // Two packets arrive; the ring (capacity 4) evicts its two oldest (10, 40).
        send_lengths(&tx, 4, &[25, 5]);
        app.drain_packets(&rx, start + Duration::from_millis(10));
        assert_eq!(app.packets.len(), 4);
        assert!(app.order_pending);
        assert_eq!(
            shown_lengths(&app),
            [30, 20, 25, 5],
            "surviving rows keep their order, new packets trail until the refresh"
        );
        for (row, expected) in [30u32, 20, 25, 5].into_iter().enumerate() {
            app.select_clicked_row(CardId::PacketList, row);
            assert_eq!(app.selected_packet().unwrap().summary.length, expected);
        }

        // The refresh stays throttled to ORDER_REFRESH, then places the new rows.
        app.refresh_order(start + Duration::from_millis(20));
        assert_eq!(shown_lengths(&app), [30, 20, 25, 5]);
        app.refresh_order(Instant::now() + ORDER_REFRESH + Duration::from_millis(1));
        assert!(!app.order_pending);
        assert_eq!(shown_lengths(&app), [30, 25, 20, 5]);
        for (row, expected) in [30u32, 25, 20, 5].into_iter().enumerate() {
            app.select_clicked_row(CardId::PacketList, row);
            assert_eq!(app.selected_packet().unwrap().summary.length, expected);
        }
    }

    #[test]
    fn eviction_with_a_filter_shifts_only_the_visible_rows_that_moved() {
        let mut app = capture_app();
        app.packets = PacketRing::new(4);
        app.active_filter = Some(parse_filter("tcp.port == 443").unwrap());
        app.rebuild_filter_indices();
        let (tx, rx) = crossbeam_channel::unbounded();
        let start = Instant::now();
        send_lengths(&tx, 0, &[10, 40, 20, 30]);
        app.drain_packets(&rx, start);
        app.change_sort(CardId::PacketList, SortChange::SelectColumn(6));
        assert_eq!(shown_lengths(&app), [40, 30, 20, 10]);

        // A packet the filter rejects arrives and evicts the oldest visible row.
        let rejected = blank_packet(9);
        tx.send(rejected).unwrap();
        app.drain_packets(&rx, start + Duration::from_millis(10));
        assert_eq!(app.visible_count(), 3);
        assert_eq!(shown_lengths(&app), [40, 30, 20]);
    }

    #[test]
    fn browse_mode_keeps_the_same_packet_selected_while_the_ring_evicts() {
        let mut app = capture_app();
        app.packets = PacketRing::new(4);
        let (tx, rx) = crossbeam_channel::unbounded();
        let start = Instant::now();
        send_lengths(&tx, 0, &[10, 40, 20, 30]);
        app.drain_packets(&rx, start);
        app.follow = false;
        app.selected = 2;
        assert_eq!(app.selected_packet().unwrap().summary.length, 20);

        // One eviction (10) shifts the selection with its packet.
        send_lengths(&tx, 4, &[25]);
        app.drain_packets(&rx, start + Duration::from_millis(10));
        assert_eq!(app.selected_packet().unwrap().summary.length, 20);

        // The selected packet itself leaves the ring: fall back to the oldest row.
        send_lengths(&tx, 5, &[5, 7, 8]);
        app.drain_packets(&rx, start + Duration::from_millis(20));
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_packet().unwrap().summary.length, 25);
    }

    #[test]
    fn angle_bracket_keys_move_the_active_tab_one_slot_stop_at_the_ends_and_skip_input_modes() {
        let mut app = capture_app();
        assert_eq!(app.handle_key(key(KeyCode::Char('<'))), Command::None);
        assert_eq!(app.handle_key(key(KeyCode::Char('>'))), Command::SaveState);
        assert_eq!(
            app.ui.tab_order[..2],
            [CaptureTab::Flows, CaptureTab::Packets]
        );
        assert_eq!(app.tab, CaptureTab::Packets, "the active tab stays");
        assert_eq!(
            app.ui.toast.as_ref().map(|toast| toast.text.as_str()),
            Some("Tab order changed")
        );
        assert_eq!(app.handle_key(key(KeyCode::Char('<'))), Command::SaveState);
        assert_eq!(app.ui.tab_order, CaptureTab::ALL);
        app.switch_tab(CaptureTab::Settings);
        assert_eq!(app.handle_key(key(KeyCode::Char('>'))), Command::None);
        assert_eq!(app.ui.tab_order, CaptureTab::ALL);
        for mode in [
            InputMode::FilterInput,
            InputMode::Search,
            InputMode::ColumnConfig,
        ] {
            app.switch_tab(CaptureTab::Flows);
            app.mode = mode;
            app.handle_key(key(KeyCode::Char('<')));
            app.handle_key(key(KeyCode::Char('>')));
            assert_eq!(app.ui.tab_order, CaptureTab::ALL, "{mode:?}");
        }
    }

    #[test]
    fn input_keys_on_another_tab_open_the_packets_tab_and_leaving_it_ends_input() {
        let mut app = capture_app();
        for (code, ctrl, mode) in [
            (KeyCode::Char('/'), false, InputMode::FilterInput),
            (KeyCode::Char('f'), true, InputMode::Search),
            (KeyCode::Char('c'), false, InputMode::ColumnConfig),
        ] {
            app.switch_tab(CaptureTab::Stats);
            let event = if ctrl {
                KeyEvent::new(code, KeyModifiers::CONTROL)
            } else {
                key(code)
            };
            app.handle_key(event);
            assert_eq!(app.tab, CaptureTab::Packets, "{mode:?}");
            assert_eq!(app.mode, mode);
            app.switch_tab(CaptureTab::Alerts);
            assert_eq!(app.mode, InputMode::Normal, "{mode:?}");
        }
    }
}
