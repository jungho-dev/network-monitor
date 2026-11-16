//! Terminal UI shared by the capture screen and the egress inspector: theme,
//! language catalog, card layout, state file, interaction, and views.

pub mod capture_io;
pub mod cards;
pub mod columns;
pub mod format;
pub mod i18n;
pub mod interaction;
pub mod layout;
pub mod layout_store;
pub mod link_guard;
pub mod sort;
pub mod state;
pub mod state_writer;
pub mod stats;
pub mod terminal;
#[cfg(test)]
pub mod test_support;
pub mod theme;
pub mod views;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossbeam_channel::{Receiver, Sender, unbounded};
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;

use network_monitor_core::app_dir::home_dir;
use network_monitor_core::capture::Linktype;
use network_monitor_core::decode::DecodedPacket;
use network_monitor_core::output::PacketSink;
use network_monitor_core::output::pcap_writer::PcapWriter;
use network_monitor_core::output::pcapng_writer::PcapngWriter;

use crate::tui::capture_io::{PacketWriter, write_log_in};
use crate::tui::layout_store::state_path_in;
use crate::tui::state::{App, CaptureSource, Command, UiMessage};
use crate::tui::state_writer::{StateWriter, spawn_state_writer};
use crate::tui::terminal::{TuiHost, enter_terminal, run_event_loop};

// 1. capture host -------------------------------------------------------------
// Drives the capture app for the shared loop: packets and background
// messages are drained in `on_tick`, I/O commands go to threads (state saves
// to the state writer thread, the log export to a one-shot thread).
struct CaptureHost {
    app: App,
    packets: Receiver<DecodedPacket>,
    messages: Receiver<UiMessage>,
    message_tx: Sender<UiMessage>,
    home: Option<PathBuf>,
    state_writer: StateWriter,
}

impl CaptureHost {
    fn dispatch(&mut self, command: Command) {
        match command {
            Command::None => {}
            Command::SaveState => self.app.ui.request_state_save(&self.state_writer),
            Command::SaveLog => {
                let packets = self.app.visible_packets_cloned();
                let local_ips = self.app.local_ips.clone();
                let reply = self.message_tx.clone();
                let home = self.home.clone();
                std::thread::spawn(move || {
                    let outcome = write_log_in(home.as_deref(), &packets, &local_ips);
                    let _ = reply.send(UiMessage::LogSaved(outcome));
                });
            }
        }
    }
}

impl TuiHost for CaptureHost {
    fn draw(&mut self, frame: &mut Frame) {
        views::render_frame(frame, &mut self.app);
    }

    fn on_tick(&mut self, now: Instant) -> bool {
        let before = self.app.total_received;
        if !self.app.paused {
            self.app.drain_packets(&self.packets, now);
        }
        let mut dirty = self.app.total_received != before;
        while let Ok(message) = self.messages.try_recv() {
            self.app.apply_message(message);
            dirty = true;
        }
        dirty | self.app.on_tick(now)
    }

    fn next_wake(&self, now: Instant, base: Duration) -> Duration {
        self.app.next_wake(now, base)
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        let command = self.app.handle_key(key);
        self.dispatch(command);
        true
    }

    fn on_mouse(&mut self, mouse: MouseEvent, now: Instant) -> bool {
        let hover_before = self.app.ui.hover_signature();
        let arranging_before = self.app.ui.arranging();
        let dragging_before = self.app.ui.dragging();
        let command = self.app.handle_mouse(mouse, now);
        self.dispatch(command);
        // A plain move redraws only when a hover target changed or a card move,
        // long press, or drag was in progress (it may commit or cancel).
        mouse.kind != MouseEventKind::Moved
            || self.app.ui.hover_signature() != hover_before
            || arranging_before
            || dragging_before
    }

    fn on_resize(&mut self) {
        let command = self.app.handle_resize();
        self.dispatch(command);
    }

    fn should_quit(&self) -> bool {
        self.app.should_quit
    }
}

// 2. entry point --------------------------------------------------------------
pub fn run_tui(
    rx: Receiver<DecodedPacket>,
    buffer_size: usize,
    save_path: Option<&Path>,
    saved_filters: HashMap<String, String>,
    pcapng: bool,
    local_ips: Vec<IpAddr>,
    source: CaptureSource,
) -> anyhow::Result<()> {
    // 1. state and the per-user state file (no home directory: state and log
    // saving are off, with one stderr line)
    let home = home_dir();
    if home.is_none() {
        eprintln!(
            "network-monitor: no home directory (USERPROFILE and HOME are unset); saving TUI state and logs is off"
        );
    }
    let state_file = state_path_in(home.as_deref());
    let mut app = App::new(buffer_size, state_file, source);
    app.saved_filters = saved_filters;
    app.local_ips = local_ips;
    app.ui.load_state();

    // 2. optional pcap writer on its own thread
    if let Some(path) = save_path {
        let file = std::fs::File::create(path)?;
        let buffered = std::io::BufWriter::new(file);
        let sink: Box<dyn PacketSink + Send> = if pcapng {
            Box::new(PcapngWriter::new(buffered, Linktype::Ethernet, 65535)?)
        } else {
            Box::new(PcapWriter::new(buffered, Linktype::Ethernet, 65535)?)
        };
        app.writer = Some(PacketWriter::spawn(sink));
    }

    // 3. state writer thread, terminal, and loop
    let (message_tx, messages) = unbounded::<UiMessage>();
    let report_tx = message_tx.clone();
    let state_writer = spawn_state_writer(move |detail| {
        let _ = report_tx.send(UiMessage::StateSaveFailed { detail });
    })
    .context("starting the state writer failed")?;
    let mut host = CaptureHost {
        app,
        packets: rx,
        messages,
        message_tx,
        home,
        state_writer,
    };
    let (_guard, mut terminal) = enter_terminal(i18n::PRODUCT_NAME)?;
    let outcome = run_event_loop(&mut terminal, &mut host);
    drop(terminal);
    drop(_guard);
    host.state_writer.finish_with_warning();
    if let Some(writer) = host.app.writer.take() {
        writer.finish()?;
    }
    outcome
}
