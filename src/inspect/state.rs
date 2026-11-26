//! Inspector view state: the daemon snapshot as plain rows, tabs, the search
//! text, and key and mouse transitions over the shared interaction state.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::tui::cards::{CardId, CursorMotion, UiAction};
use crate::tui::columns::card_columns;
use crate::tui::i18n::{self, Msg};
use crate::tui::interaction::{MouseOutcome, ToastLevel, Ui};
use crate::tui::layout::SplitGroup;
use crate::tui::sort::SortChange;

// 1. snapshot rows ------------------------------------------------------------
// The daemon's data as plain values, so the view code never touches IPC types.
#[derive(Debug, Clone, Default)]
pub struct ConnItem {
    pub id: u64,
    pub process: String,
    pub pid: Option<u32>,
    pub dest_name: String,
    pub remote_ip: String,
    pub remote_port: u16,
    pub local_port: u16,
    pub protocol: u8,
    pub country: Option<String>,
    pub asn: Option<u32>,
    pub as_org: Option<String>,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub outbound: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ProcItem {
    pub process: String,
    pub pid: Option<u32>,
    pub conns: usize,
    pub bytes_up: u64,
    pub bytes_down: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DomainItem {
    pub domain: String,
    pub conns: usize,
    pub bytes_up: u64,
    pub bytes_down: u64,
}

// Rank orders the severities; the label shown comes from the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeverityRank {
    #[default]
    Info,
    Notice,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Default)]
pub struct AlertItem {
    pub kind: String,
    pub severity: SeverityRank,
    pub title: String,
    pub ts_ms: i64,
    pub key: String,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryItem {
    pub name: String,
    pub proto: u8,
    pub local_port: u16,
    pub remote_port: u16,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub started_ms: i64,
    pub ended_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct StatusItem {
    pub baseline: String,
    pub alerts: u64,
    // The daemon runs another build than this inspector.
    pub stale_build: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InspectData {
    pub conns: Vec<ConnItem>,
    pub procs: Vec<ProcItem>,
    pub domains: Vec<DomainItem>,
    pub alerts: Vec<AlertItem>,
    pub history: Vec<HistoryItem>,
    pub status: Option<StatusItem>,
    pub connected: bool,
    pub disconnect_detail: Option<String>,
}

// 2. tabs ---------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectTab {
    Connections,
    Processes,
    Domains,
    Alerts,
    History,
}

impl InspectTab {
    pub const ALL: [InspectTab; 5] = [
        InspectTab::Connections,
        InspectTab::Processes,
        InspectTab::Domains,
        InspectTab::Alerts,
        InspectTab::History,
    ];

    pub fn label(self) -> Msg {
        match self {
            Self::Connections => Msg::TabConnections,
            Self::Processes => Msg::TabProcesses,
            Self::Domains => Msg::TabDomains,
            Self::Alerts => Msg::TabAlerts,
            Self::History => Msg::TabHistory,
        }
    }

    pub fn table_card(self) -> CardId {
        match self {
            Self::Connections => CardId::ConnTable,
            Self::Processes => CardId::ProcTable,
            Self::Domains => CardId::DomainTable,
            Self::Alerts => CardId::AlertTable,
            Self::History => CardId::HistoryTable,
        }
    }

    pub fn detail_card(self) -> CardId {
        match self {
            Self::Connections => CardId::ConnDetail,
            Self::Processes => CardId::ProcDetail,
            Self::Domains => CardId::DomainDetail,
            Self::Alerts => CardId::AlertDetail,
            Self::History => CardId::HistoryDetail,
        }
    }

    pub fn group(self) -> SplitGroup {
        match self {
            Self::Connections => SplitGroup::InspectConn,
            Self::Processes => SplitGroup::InspectProc,
            Self::Domains => SplitGroup::InspectDomain,
            Self::Alerts => SplitGroup::InspectAlert,
            Self::History => SplitGroup::InspectHistory,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL[index % Self::ALL.len()]
    }
}

// What the host must do after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    None,
    SaveState,
    Refresh,
    LoadHistory,
    // A tab switch that committed a moved drag and opened the history tab.
    SaveStateAndLoadHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

// How the inspector loop ended: a plain quit, or a force restart on the latest build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectExit {
    Quit,
    Restart,
}

// Why a force restart would fail, found before the daemon is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartBlock {
    NotMonitorRun,
    RelativePath,
    ExeOwner,
    Privilege,
    StateAccess,
    NewExePrivilege,
    PreflightUnknown,
    ExeName,
}

impl RestartBlock {
    pub fn toast(self) -> Msg {
        match self {
            Self::NotMonitorRun => Msg::ToastRestartNotMonitorRun,
            Self::RelativePath => Msg::ToastRestartRelativePath,
            Self::ExeOwner => Msg::ToastRestartExeOwner,
            Self::Privilege => Msg::ToastRestartNeedsPrivilege,
            Self::StateAccess => Msg::ToastRestartNeedsStateAccess,
            Self::NewExePrivilege => Msg::ToastRestartNewExePrivilege,
            Self::PreflightUnknown => Msg::ToastRestartPreflightUnknown,
            Self::ExeName => Msg::ToastRestartExeName,
        }
    }
}

// Daemon build against its exe on disk: current, restartable, or blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartState {
    #[default]
    Current,
    Ready,
    Blocked(RestartBlock),
}

// Disk build identifier, settled once two reads in a row agree.
#[derive(Debug, Default)]
pub struct DiskBuildWatch {
    settled: String,
    last: String,
}

impl DiskBuildWatch {
    // True when this read settles a new identifier.
    pub fn observe(&mut self, disk: &str) -> bool {
        let settles = disk == self.last && disk != self.settled;
        if settles {
            self.settled = disk.to_string();
        }
        self.last = disk.to_string();
        settles
    }

    // 테스트 전용 조회 (unix 비테스트 빌드의 dead_code 방지)
    #[cfg(test)]
    pub fn settled(&self) -> &str {
        &self.settled
    }
}

// Answer of the new exe's `monitor preflight` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightOutcome {
    Passed,
    NoPrivilege,
    Unknown,
}

// A failed or unknown preflight answer is checked again after this long.
pub const PREFLIGHT_RETRY: Duration = Duration::from_secs(10);

// The new exe's own capture privilege check for one disk build identifier.
#[derive(Debug, Default)]
pub struct PreflightCache {
    build: String,
    outcome: Option<PreflightOutcome>,
    checked_at: Option<Instant>,
}

impl PreflightCache {
    // A pass for the stored known build; any other answer until it expires.
    pub fn lookup(&self, build: &str, now: Instant) -> Option<PreflightOutcome> {
        if build.is_empty() || build != self.build {
            return None;
        }
        let outcome = self.outcome?;
        if outcome == PreflightOutcome::Passed {
            return Some(outcome);
        }
        let fresh = self
            .checked_at
            .is_some_and(|checked| now.saturating_duration_since(checked) < PREFLIGHT_RETRY);
        fresh.then_some(outcome)
    }

    pub fn store(&mut self, build: &str, outcome: PreflightOutcome, now: Instant) {
        self.build = build.to_string();
        self.outcome = Some(outcome);
        self.checked_at = Some(now);
    }
}

// 3. state --------------------------------------------------------------------
pub struct Inspect {
    pub ui: Ui,
    pub data: InspectData,
    pub tab: InspectTab,
    pub mode: Mode,
    pub search: String,
    pub should_quit: bool,
    // Time of the last loop turn, for the stale marker.
    pub clock: Instant,
    pub last_update: Option<Instant>,
    // Why a restart cannot run now; `None` when it can.
    pub restart_block: Option<RestartBlock>,
    pub restart_requested: bool,
}

// A daemon silent for this long reads as stale in the app bar.
pub const STALE_AFTER: Duration = Duration::from_secs(5);

// A daemon is on another build only when its build and its exe's build on
// disk now are both known, they differ, and it was started with `monitor run`
// (so it can be started again).
pub fn daemon_is_stale(daemon_build: &str, disk_build: &str, run_args: &[String]) -> bool {
    !daemon_build.is_empty()
        && !disk_build.is_empty()
        && daemon_build != disk_build
        && run_args.len() >= 2
        && run_args[0] == "monitor"
        && run_args[1] == "run"
}

// The daemon's own `monitor run` arguments in order, plus `--state-dir` when
// it had none, plus `--daemonize` when missing; every path must be absolute.
pub fn daemon_restart_args(
    run_args: &[String],
    state_dir: &str,
) -> Result<Vec<String>, RestartBlock> {
    // 1. a `monitor run` command
    if run_args.len() < 2 || run_args[0] != "monitor" || run_args[1] != "run" {
        return Err(RestartBlock::NotMonitorRun);
    }

    // 2. absolute state folder and path option values
    if !state_dir.starts_with('/') {
        return Err(RestartBlock::RelativePath);
    }
    let path_flags = ["--state-dir", "--geoip-country-db", "--geoip-asn-db"];
    let mut has_state_dir = false;
    for (index, arg) in run_args.iter().enumerate() {
        let (flag, path) = match arg.split_once('=') {
            Some((flag, path)) => (flag, Some(path)),
            None => (arg.as_str(), run_args.get(index + 1).map(String::as_str)),
        };
        if !path_flags.contains(&flag) {
            continue;
        }
        has_state_dir |= flag == "--state-dir";
        if !path.is_some_and(|path| path.starts_with('/')) {
            return Err(RestartBlock::RelativePath);
        }
    }

    // 3. missing state folder and daemonize flag
    let mut restart_args = run_args.to_vec();
    if !has_state_dir {
        restart_args.push("--state-dir".to_string());
        restart_args.push(state_dir.to_string());
    }
    if !run_args.iter().any(|arg| arg == "--daemonize") {
        restart_args.push("--daemonize".to_string());
    }
    Ok(restart_args)
}

// A shell command line: a word that is empty or holds a character outside
// letters, digits, and `-_./=:,@%+` goes in single quotes, `'` as `'\''`.
pub fn shell_quote_args(program: &str, args: &[String]) -> String {
    let mut words = Vec::with_capacity(args.len() + 1);
    for word in std::iter::once(program).chain(args.iter().map(String::as_str)) {
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|letter| letter.is_ascii_alphanumeric() || "-_./=:,@%+".contains(letter));
        let quoted = if plain {
            word.to_string()
        } else {
            format!("'{}'", word.replace('\'', r"'\''"))
        };
        words.push(quoted);
    }
    words.join(" ")
}

// Exit code of `monitor preflight`: 0 passes, 1 lacks privilege, anything
// else (an old build, a signal, no run) is unknown.
pub fn preflight_outcome(exit_code: Option<i32>) -> PreflightOutcome {
    match exit_code {
        Some(0) => PreflightOutcome::Passed,
        Some(1) => PreflightOutcome::NoPrivilege,
        _ => PreflightOutcome::Unknown,
    }
}

impl Inspect {
    pub fn new(state_file: Option<PathBuf>) -> Self {
        let mut ui = Ui::new(state_file);
        ui.default_focus = InspectTab::Connections.table_card();
        Self {
            ui,
            data: InspectData {
                connected: true,
                ..InspectData::default()
            },
            tab: InspectTab::Connections,
            mode: Mode::Normal,
            search: String::new(),
            should_quit: false,
            clock: Instant::now(),
            last_update: None,
            restart_block: None,
            restart_requested: false,
        }
    }

    // Replaces the snapshot; the table selection follows the same item by id in the views.
    pub fn set_data(&mut self, data: InspectData, now: Instant) {
        self.data = data;
        self.last_update = Some(now);
    }

    // Connections matching the search text (case-insensitive substring over
    // process, destination, address, organization, and country).
    pub fn matching_conns(&self) -> Vec<&ConnItem> {
        if self.search.is_empty() {
            return self.data.conns.iter().collect();
        }
        let query = self.search.to_lowercase();
        self.data
            .conns
            .iter()
            .filter(|conn| {
                conn.process.to_lowercase().contains(&query)
                    || conn.dest_name.to_lowercase().contains(&query)
                    || conn.remote_ip.contains(&query)
                    || conn
                        .as_org
                        .as_deref()
                        .is_some_and(|org| org.to_lowercase().contains(&query))
                    || conn
                        .country
                        .as_deref()
                        .is_some_and(|country| country.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn is_stale(&self) -> bool {
        self.data.connected
            && self
                .last_update
                .is_some_and(|last| self.clock.saturating_duration_since(last) >= STALE_AFTER)
    }

    pub fn on_tick(&mut self, now: Instant) -> bool {
        let was_stale = self.is_stale();
        self.clock = now;
        let mut dirty = self.ui.tick(now);
        dirty |= self.ui.expire_toast(now);
        dirty || was_stale != self.is_stale()
    }

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

    // 4. tabs -----------------------------------------------------------------
    // Clears any card move, commits a drag that had moved (its release may
    // never arrive once the tab changes, so it is saved now), resets the focus
    // for the new tab, and asks for the history when that tab opens.
    pub fn set_tab(&mut self, tab: InspectTab) -> Command {
        self.ui.cancel_card_move();
        let drag_moved = self.ui.commit_drags();
        self.tab = tab;
        self.ui.context = tab.index();
        self.ui.default_focus = tab.table_card();
        match (drag_moved, tab == InspectTab::History) {
            (true, true) => Command::SaveStateAndLoadHistory,
            (true, false) => Command::SaveState,
            (false, true) => Command::LoadHistory,
            (false, false) => Command::None,
        }
    }

    // 5. actions --------------------------------------------------------------
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
                // Order, sizes, and column widths of the current tab only.
                let tab = self.tab;
                self.ui.reset_layout(&[tab.group()], &[tab.table_card()]);
                Command::SaveState
            }
            UiAction::Refresh => {
                let text = i18n::text(self.ui.lang, Msg::ToastRefreshRequested);
                self.ui.show_toast(text, ToastLevel::Info);
                Command::Refresh
            }
            UiAction::OpenSearch => {
                self.mode = Mode::Search;
                Command::None
            }
            UiAction::ForceRestart => {
                let mismatch = self.data.connected
                    && self
                        .data
                        .status
                        .as_ref()
                        .is_some_and(|status| status.stale_build);
                if !mismatch {
                    return Command::None;
                }
                if let Some(block) = self.restart_block {
                    let text = i18n::text(self.ui.lang, block.toast());
                    self.ui.show_toast(text, ToastLevel::Error);
                    return Command::None;
                }
                self.restart_requested = true;
                self.should_quit = true;
                Command::None
            }
            UiAction::TogglePause
            | UiAction::OpenFilter
            | UiAction::OpenColumns
            | UiAction::SaveLog
            | UiAction::CycleLevel => Command::None,
        }
    }

    // 6. keys -----------------------------------------------------------------
    pub fn handle_key(&mut self, key: KeyEvent) -> Command {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Command::None;
        }
        if self.ui.show_help {
            self.ui.show_help = false;
            return Command::None;
        }
        if self.ui.card_move.is_some() || self.ui.card_press.is_some() {
            if key.code == KeyCode::Esc {
                self.ui.cancel_card_move();
            }
            return Command::None;
        }
        if self.mode == Mode::Search {
            match key.code {
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Esc => {
                    self.search.clear();
                    self.mode = Mode::Normal;
                }
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Char(letter) => self.search.push(letter),
                _ => {}
            }
            return Command::None;
        }
        let card = self.ui.focused_card();
        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        let slot = self.tab.index();
        let count = InspectTab::ALL.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.perform_action(UiAction::Quit),
            KeyCode::Char('?') => self.perform_action(UiAction::ShowHelp),
            KeyCode::Char('!') => self.perform_action(UiAction::ForceRestart),
            KeyCode::BackTab => self.set_tab(InspectTab::from_index(slot + count - 1)),
            KeyCode::Tab if shifted => self.set_tab(InspectTab::from_index(slot + count - 1)),
            KeyCode::Tab => self.set_tab(InspectTab::from_index(slot + 1)),
            KeyCode::Left if !shifted => self.set_tab(InspectTab::from_index(slot + count - 1)),
            KeyCode::Right if !shifted => self.set_tab(InspectTab::from_index(slot + 1)),
            KeyCode::Char(digit @ '1'..='5') => {
                self.set_tab(InspectTab::from_index(digit as usize - '1' as usize))
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right if shifted => {
                if self.ui.shift_card(card, key.code) {
                    Command::SaveState
                } else {
                    Command::None
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.step(card, CursorMotion::Lines(-1)),
            KeyCode::Down | KeyCode::Char('j') => self.step(card, CursorMotion::Lines(1)),
            KeyCode::PageUp => self.step(card, CursorMotion::Pages(-1)),
            KeyCode::PageDown => self.step(card, CursorMotion::Pages(1)),
            KeyCode::Home => self.step(card, CursorMotion::Home),
            KeyCode::End => self.step(card, CursorMotion::End),
            KeyCode::Enter => {
                // Keyboard path to the detail card and back.
                let next = if card == self.tab.table_card() {
                    self.tab.detail_card()
                } else {
                    self.tab.table_card()
                };
                self.ui.set_focus(next);
                Command::None
            }
            KeyCode::Char(letter @ ('s' | 'S')) => {
                let target = if card_columns(card).is_empty() {
                    self.tab.table_card()
                } else {
                    card
                };
                let change = if letter == 's' {
                    SortChange::NextColumn
                } else {
                    SortChange::ReverseDirection
                };
                self.ui.change_sort(target, change);
                Command::None
            }
            KeyCode::Char('/') => self.perform_action(UiAction::OpenSearch),
            KeyCode::Char('r') => self.perform_action(UiAction::Refresh),
            KeyCode::Char('l') => self.perform_action(UiAction::ResetLayout),
            KeyCode::Char('L') => self.perform_action(UiAction::ToggleLanguage),
            _ => Command::None,
        }
    }

    fn step(&mut self, card: CardId, motion: CursorMotion) -> Command {
        self.ui.move_cursor(card, motion);
        Command::None
    }

    // 7. mouse ----------------------------------------------------------------
    pub fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) -> Command {
        match self.ui.handle_mouse(mouse, now) {
            MouseOutcome::None | MouseOutcome::Scrolled(_) => Command::None,
            MouseOutcome::SaveState => Command::SaveState,
            MouseOutcome::Action(action) => self.perform_action(action),
            MouseOutcome::Tab(index) => self.set_tab(InspectTab::from_index(index)),
            MouseOutcome::Header { card, column } => {
                self.ui.change_sort(card, SortChange::SelectColumn(column));
                Command::None
            }
            MouseOutcome::Row { card, index } => {
                self.ui.select_row(card, index);
                Command::None
            }
        }
    }

    pub fn handle_resize(&mut self) -> Command {
        if self.ui.handle_resize() {
            Command::SaveState
        } else {
            Command::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::i18n::Lang;
    use crate::tui::test_support::key;

    fn inspect() -> Inspect {
        Inspect::new(Some(PathBuf::from("tui-test.toml")))
    }

    fn conn(id: u64, process: &str, dest: &str) -> ConnItem {
        ConnItem {
            id,
            process: process.into(),
            dest_name: dest.into(),
            remote_ip: "1.2.3.4".into(),
            country: Some("US".into()),
            ..ConnItem::default()
        }
    }

    #[test]
    fn digits_tab_and_arrows_switch_tabs_with_wraparound() {
        let mut ui = inspect();
        ui.handle_key(key(KeyCode::Char('4')));
        assert_eq!(ui.tab, InspectTab::Alerts);
        ui.handle_key(key(KeyCode::Tab));
        assert_eq!(ui.tab, InspectTab::History);
        ui.handle_key(key(KeyCode::Tab));
        assert_eq!(ui.tab, InspectTab::Connections);
        ui.handle_key(key(KeyCode::BackTab));
        assert_eq!(ui.tab, InspectTab::History);
        ui.handle_key(key(KeyCode::Left));
        assert_eq!(ui.tab, InspectTab::Alerts);
        ui.handle_key(key(KeyCode::Right));
        assert_eq!(ui.tab, InspectTab::History);
    }

    #[test]
    fn opening_history_requests_a_load_and_each_tab_has_its_own_default_focus() {
        let mut ui = inspect();
        assert_eq!(ui.handle_key(key(KeyCode::Char('5'))), Command::LoadHistory);
        assert_eq!(ui.ui.focused_card(), CardId::HistoryTable);
        ui.handle_key(key(KeyCode::Char('2')));
        assert_eq!(ui.ui.focused_card(), CardId::ProcTable);
        ui.ui.set_focus(CardId::ProcDetail);
        ui.handle_key(key(KeyCode::Char('3')));
        assert_eq!(ui.ui.focused_card(), CardId::DomainTable);
        ui.handle_key(key(KeyCode::Char('2')));
        assert_eq!(ui.ui.focused_card(), CardId::ProcDetail, "focus is per tab");
    }

    #[test]
    fn enter_moves_focus_between_table_and_detail() {
        let mut ui = inspect();
        ui.handle_key(key(KeyCode::Enter));
        assert_eq!(ui.ui.focused_card(), CardId::ConnDetail);
        ui.handle_key(key(KeyCode::Enter));
        assert_eq!(ui.ui.focused_card(), CardId::ConnTable);
    }

    #[test]
    fn search_filters_connections_case_insensitively_and_escape_clears_it() {
        let mut ui = inspect();
        ui.data.conns = vec![conn(1, "curl", "example.com"), conn(2, "ssh", "github.com")];
        ui.handle_key(key(KeyCode::Char('/')));
        assert_eq!(ui.mode, Mode::Search);
        for letter in "GIT".chars() {
            ui.handle_key(key(KeyCode::Char(letter)));
        }
        assert_eq!(ui.matching_conns().len(), 1);
        ui.handle_key(key(KeyCode::Enter));
        assert_eq!(ui.mode, Mode::Normal);
        assert_eq!(ui.matching_conns()[0].process, "ssh");
        ui.handle_key(key(KeyCode::Char('/')));
        ui.handle_key(key(KeyCode::Esc));
        assert!(ui.search.is_empty());
        assert_eq!(ui.matching_conns().len(), 2);
        ui.search = "us".into();
        assert_eq!(ui.matching_conns().len(), 2, "country matches");
    }

    #[test]
    fn language_refresh_reset_and_quit_keys_map_to_commands() {
        let mut ui = inspect();
        assert_eq!(ui.handle_key(key(KeyCode::Char('L'))), Command::SaveState);
        assert_eq!(ui.ui.lang, Lang::Ko);
        assert_eq!(ui.handle_key(key(KeyCode::Char('r'))), Command::Refresh);
        assert_eq!(ui.handle_key(key(KeyCode::Char('l'))), Command::SaveState);
        assert_eq!(ui.ui.lang, Lang::Ko, "reset keeps the language");
        ui.handle_key(key(KeyCode::Char('q')));
        assert!(ui.should_quit);
    }

    #[test]
    fn staleness_needs_a_connected_daemon_that_stopped_updating() {
        let mut ui = inspect();
        let start = Instant::now();
        ui.set_data(
            InspectData {
                connected: true,
                ..InspectData::default()
            },
            start,
        );
        assert!(!ui.is_stale());
        assert!(ui.on_tick(start + STALE_AFTER));
        assert!(ui.is_stale());
        ui.data.connected = false;
        assert!(!ui.is_stale());
    }

    #[test]
    fn a_tab_switch_commits_a_moved_drag_and_still_loads_the_history() {
        let mut ui = inspect();
        ui.ui.column_drag = Some(crate::tui::cards::ColumnDrag {
            card: CardId::ConnTable,
            column: 1,
            start_x: 10,
            start_width: 8,
            moved: true,
        });
        assert_eq!(ui.set_tab(InspectTab::Processes), Command::SaveState);
        assert!(!ui.ui.dragging());

        ui.ui.split_drag = Some(crate::tui::layout::SplitDrag {
            group: SplitGroup::InspectProc,
            index: 0,
            grip: 0,
            moved: true,
        });
        assert_eq!(
            ui.set_tab(InspectTab::History),
            Command::SaveStateAndLoadHistory
        );
        assert!(!ui.ui.dragging());

        // An undragged press leaves nothing to save.
        ui.ui.scroll_drag = Some(crate::tui::cards::ScrollDrag {
            card: CardId::ConnTable,
            grip: 0,
        });
        assert_eq!(ui.set_tab(InspectTab::Connections), Command::None);
        assert!(!ui.ui.dragging());
    }
}
