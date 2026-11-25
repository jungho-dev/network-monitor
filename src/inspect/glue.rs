//! Socket glue (Unix): IPC reader and query threads, conversion of the core
//! reducer into plain rows, and the host that plugs the inspector into the
//! shared event loop. All socket I/O stays off the render loop.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;

use network_monitor_core::alert::Severity;
use network_monitor_core::app_dir::home_dir;
use network_monitor_core::build_info::read_build_id;
use network_monitor_core::inspector::InspectorApp;
use network_monitor_core::ipc::protocol::{DaemonStatus, Event as IpcEvent};
use network_monitor_core::ipc::{IpcClient, Request, Response};
use network_monitor_core::store::models::ConnectionRow;

use crate::inspect::restart::restart_state;
use crate::inspect::state::{
    AlertItem, Command, ConnItem, DiskBuildWatch, DomainItem, HistoryItem, Inspect, InspectData,
    InspectExit, InspectTab, PreflightCache, ProcItem, RestartBlock, RestartState, SeverityRank,
    StatusItem,
};
use crate::inspect::views;
use crate::tui::i18n;
use crate::tui::layout_store::state_path_in;
use crate::tui::state_writer::{StateWriter, spawn_state_writer};
use crate::tui::terminal::{TuiHost, enter_terminal, run_event_loop};

// Rows requested per query, and the least time between data rebuilds.
const QUERY_LIMIT: usize = 200;
const REBUILD_INTERVAL: Duration = Duration::from_millis(100);
const EXE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

// 1. messages -----------------------------------------------------------------
enum Msg {
    Event(IpcEvent),
    Disconnected(String),
    Snapshot(Box<Snapshot>),
    // The state writer thread failed to write `tui.toml` (raw error text).
    StateSaveFailed(String),
    // The force restart state changed.
    Restart(RestartState),
}

// Results of one query round; a field is None when its request was not made or failed.
#[derive(Default)]
struct Snapshot {
    status: Option<DaemonStatus>,
    conns: Option<Vec<network_monitor_core::ipc::protocol::ConnectionDto>>,
    alerts: Option<Vec<network_monitor_core::alert::Alert>>,
    history: Option<Vec<ConnectionRow>>,
}

enum Query {
    Refresh { history: bool },
    LoadHistory,
}

// 2. query thread -------------------------------------------------------------
// Serves refresh and history requests so the UI thread never waits on the
// socket, and watches the daemon's exe on disk for a force restart.
fn query_loop(
    mut client: IpcClient,
    socket: PathBuf,
    queries: Receiver<Query>,
    replies: Sender<Msg>,
) {
    let mut watch = DiskBuildWatch::default();
    let mut preflight = PreflightCache::default();
    let mut latest: Option<DaemonStatus> = None;
    let mut restart = RestartState::Current;
    let mut last_check: Option<Instant> = None;
    loop {
        // 1. a request, or a timeout for the exe check alone
        let received = queries.recv_timeout(EXE_CHECK_INTERVAL);
        if matches!(received, Err(RecvTimeoutError::Disconnected)) {
            break;
        }
        let answered = received.is_ok();
        let (with_live, with_history) = match received {
            Ok(Query::Refresh { history }) => (true, history),
            Ok(Query::LoadHistory) => (false, true),
            Err(_) => (false, false),
        };
        let mut snapshot = Snapshot::default();
        if with_live {
            if let Ok(Response::Status(status)) = client.request(&Request::Status) {
                snapshot.status = Some(status);
            }
            if let Ok(Response::Connections(conns)) = client.request(&Request::LiveConnections) {
                snapshot.conns = Some(conns);
            }
            if let Ok(Response::Alerts(alerts)) =
                client.request(&Request::RecentAlerts { limit: QUERY_LIMIT })
            {
                snapshot.alerts = Some(alerts);
            }
        }
        if with_history
            && let Ok(Response::History(rows)) =
                client.request(&Request::RecentConnections { limit: QUERY_LIMIT })
        {
            snapshot.history = Some(rows);
        }
        let status_arrived = snapshot.status.is_some();
        if status_arrived {
            latest = snapshot.status.clone();
        }
        if answered && replies.send(Msg::Snapshot(Box::new(snapshot))).is_err() {
            break;
        }

        // 2. daemon exe build on disk, every interval, and a retry of a
        // failed or unknown new exe check
        let mut disk_settled = false;
        let mut recheck = false;
        if last_check.is_none_or(|checked| checked.elapsed() >= EXE_CHECK_INTERVAL) {
            last_check = Some(Instant::now());
            recheck = matches!(
                restart,
                RestartState::Blocked(
                    RestartBlock::NewExePrivilege | RestartBlock::PreflightUnknown
                )
            );
            let disk_build = latest
                .as_ref()
                .map(|status| read_build_id(Path::new(&status.exe_path)))
                .unwrap_or_default();
            disk_settled = watch.observe(&disk_build);
        }

        // 3. restart state, sent on change
        if !status_arrived && !disk_settled && !recheck {
            continue;
        }
        let Some(status) = latest.as_ref() else {
            continue;
        };
        let next = restart_state(status, &socket, &mut preflight);
        if next == restart {
            continue;
        }
        restart = next;
        if replies.send(Msg::Restart(next)).is_err() {
            break;
        }
    }
}

// 3. conversion ---------------------------------------------------------------
fn severity_rank(severity: Severity) -> SeverityRank {
    match severity {
        Severity::Info => SeverityRank::Info,
        Severity::Notice => SeverityRank::Notice,
        Severity::Warning => SeverityRank::Warning,
        Severity::Critical => SeverityRank::Critical,
    }
}

// Plain rows from the core reducer and the last history answer.
fn build_data(
    core: &InspectorApp,
    history: &[ConnectionRow],
    restart: RestartState,
) -> InspectData {
    InspectData {
        conns: core
            .connections()
            .into_iter()
            .map(|dto| ConnItem {
                id: dto.id,
                process: dto.process.clone(),
                pid: dto.pid,
                dest_name: dto.dest_name.clone(),
                remote_ip: dto.remote_ip.clone(),
                remote_port: dto.remote_port,
                local_port: dto.local_port,
                protocol: dto.protocol,
                country: dto.country.clone(),
                asn: dto.asn,
                as_org: dto.as_org.clone(),
                bytes_up: dto.bytes_up,
                bytes_down: dto.bytes_down,
                first_seen_ms: dto.first_seen_ms,
                last_seen_ms: dto.last_seen_ms,
                outbound: dto.outbound,
            })
            .collect(),
        procs: core
            .process_aggs()
            .into_iter()
            .map(|agg| ProcItem {
                process: agg.process,
                pid: agg.pid,
                conns: agg.conns,
                bytes_up: agg.bytes_up,
                bytes_down: agg.bytes_down,
            })
            .collect(),
        domains: core
            .domain_aggs()
            .into_iter()
            .map(|agg| DomainItem {
                domain: agg.domain,
                conns: agg.conns,
                bytes_up: agg.bytes_up,
                bytes_down: agg.bytes_down,
            })
            .collect(),
        alerts: core
            .alerts
            .iter()
            .map(|alert| AlertItem {
                kind: alert.kind.label().to_string(),
                severity: severity_rank(alert.severity),
                title: alert.title.clone(),
                ts_ms: alert.ts,
                key: alert.dedup_key.clone(),
            })
            .collect(),
        history: history
            .iter()
            .map(|row| HistoryItem {
                name: row.name.clone().unwrap_or_default(),
                proto: row.proto,
                local_port: row.local_port,
                remote_port: row.remote_port,
                bytes_up: row.bytes_up,
                bytes_down: row.bytes_down,
                started_ms: row.ts_start_ms,
                ended_ms: row.ts_end_ms,
            })
            .collect(),
        status: core.status.as_ref().map(|status| StatusItem {
            baseline: status.baseline.clone(),
            alerts: status.alerts,
            stale_build: restart != RestartState::Current,
        }),
        connected: core.connected,
        disconnect_detail: core.disconnect_msg.clone(),
    }
}

// 4. host ---------------------------------------------------------------------
struct InspectHost {
    inspect: Inspect,
    core: InspectorApp,
    history: Vec<ConnectionRow>,
    restart: RestartState,
    messages: Receiver<Msg>,
    queries: Sender<Query>,
    data_dirty: bool,
    last_build: Option<Instant>,
    state_writer: StateWriter,
}

impl InspectHost {
    fn dispatch(&mut self, command: Command) {
        match command {
            Command::None => {}
            Command::SaveState => self.inspect.ui.request_state_save(&self.state_writer),
            Command::SaveStateAndLoadHistory => {
                self.inspect.ui.request_state_save(&self.state_writer);
                let _ = self.queries.send(Query::LoadHistory);
            }
            Command::Refresh => {
                let history = self.inspect.tab == InspectTab::History;
                let _ = self.queries.send(Query::Refresh { history });
            }
            Command::LoadHistory => {
                let _ = self.queries.send(Query::LoadHistory);
            }
        }
    }

    fn apply_message(&mut self, message: Msg) {
        match message {
            Msg::Event(event) => self.core.apply(event),
            Msg::Disconnected(detail) => self.core.set_disconnected(detail),
            Msg::StateSaveFailed(detail) => self.inspect.ui.state_save_failed(&detail),
            Msg::Restart(restart) => self.restart = restart,
            Msg::Snapshot(snapshot) => {
                let Snapshot {
                    status,
                    conns,
                    alerts,
                    history,
                } = *snapshot;
                if let Some(status) = status {
                    self.core.set_status(status);
                }
                if let Some(conns) = conns {
                    self.core.set_snapshot(conns);
                }
                if let Some(alerts) = alerts {
                    self.core.set_recent_alerts(alerts);
                }
                if let Some(rows) = history {
                    self.history = rows;
                }
            }
        }
        self.data_dirty = true;
    }
}

impl TuiHost for InspectHost {
    fn draw(&mut self, frame: &mut Frame) {
        views::render_frame(frame, &mut self.inspect);
    }

    fn on_tick(&mut self, now: Instant) -> bool {
        while let Ok(message) = self.messages.try_recv() {
            self.apply_message(message);
        }
        let mut dirty = false;
        let due = self
            .last_build
            .is_none_or(|built| now.saturating_duration_since(built) >= REBUILD_INTERVAL);
        if self.data_dirty && due {
            let data = build_data(&self.core, &self.history, self.restart);
            self.inspect.restart_block = match self.restart {
                RestartState::Blocked(block) => Some(block),
                RestartState::Current | RestartState::Ready => None,
            };
            self.inspect.set_data(data, now);
            self.data_dirty = false;
            self.last_build = Some(now);
            dirty = true;
        }
        dirty | self.inspect.on_tick(now)
    }

    fn next_wake(&self, now: Instant, base: Duration) -> Duration {
        self.inspect.next_wake(now, base)
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        let command = self.inspect.handle_key(key);
        self.dispatch(command);
        true
    }

    fn on_mouse(&mut self, mouse: MouseEvent, now: Instant) -> bool {
        let hover_before = self.inspect.ui.hover_signature();
        let arranging_before = self.inspect.ui.arranging();
        let dragging_before = self.inspect.ui.dragging();
        let command = self.inspect.handle_mouse(mouse, now);
        self.dispatch(command);
        mouse.kind != MouseEventKind::Moved
            || self.inspect.ui.hover_signature() != hover_before
            || arranging_before
            || dragging_before
    }

    fn on_resize(&mut self) {
        let command = self.inspect.handle_resize();
        self.dispatch(command);
    }

    fn should_quit(&self) -> bool {
        self.inspect.should_quit
    }
}

// 5. entry point --------------------------------------------------------------
pub fn run_inspector(socket_path: &Path) -> Result<InspectExit> {
    // 1. query connection, then a second one subscribed to the event stream
    let query = IpcClient::connect(socket_path)
        .map_err(|e| anyhow!("cannot connect to daemon at {}: {e}", socket_path.display()))?;
    let mut sub = IpcClient::connect(socket_path)?;
    let _ = sub.request(&Request::Subscribe);

    // 2. reader and query threads report through one channel
    let (message_tx, messages) = unbounded::<Msg>();
    let reader_tx = message_tx.clone();
    let report_tx = message_tx.clone();
    std::thread::spawn(move || {
        loop {
            match sub.next_event() {
                Ok(event) => {
                    if reader_tx.send(Msg::Event(event)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = reader_tx.send(Msg::Disconnected(error.to_string()));
                    break;
                }
            }
        }
    });
    let (query_tx, query_rx) = unbounded::<Query>();
    let query_socket = socket_path.to_path_buf();
    std::thread::spawn(move || query_loop(query, query_socket, query_rx, message_tx));
    let _ = query_tx.send(Query::Refresh { history: false });

    // 3. state, saved layout (no home directory: state saving is off, with one
    // stderr line), state writer thread, terminal, loop
    let home = home_dir();
    if home.is_none() {
        eprintln!(
            "network-monitor: no home directory (USERPROFILE and HOME are unset); saving TUI state is off"
        );
    }
    let mut inspect = Inspect::new(state_path_in(home.as_deref()));
    inspect.ui.load_state();
    let state_writer = spawn_state_writer(move |detail| {
        let _ = report_tx.send(Msg::StateSaveFailed(detail));
    })
    .context("starting the state writer failed")?;
    let mut host = InspectHost {
        inspect,
        core: InspectorApp::new(),
        history: Vec::new(),
        restart: RestartState::Current,
        messages,
        queries: query_tx,
        data_dirty: false,
        last_build: None,
        state_writer,
    };
    let (_guard, mut terminal) = enter_terminal(i18n::PRODUCT_NAME)?;
    let outcome = run_event_loop(&mut terminal, &mut host);
    drop(terminal);
    drop(_guard);
    host.state_writer.finish_with_warning();
    outcome?;
    let exit = if host.inspect.restart_requested {
        InspectExit::Restart
    } else {
        InspectExit::Quit
    };
    Ok(exit)
}
