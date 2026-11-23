# network-monitor Architecture

## Overview

1. **Scope**
    - The architecture of this repository as implemented in the current source tree: the workspace layout, the capture pipeline, the decoder chain, the egress monitor, the terminal UIs, and the build notes.
    - `network-monitor` is a Cargo workspace whose root package is the single CLI binary that hosts two terminal frontends, with the reusable engine crate as its one workspace member.
2. **Documents**
    - Installation, commands, and keys: [README.md](README.md)
    - Korean version: [ARCHITECTURE-ko.md](ARCHITECTURE-ko.md)

---

## 1. Design Principles

1. **Decoupled stages**
    - The capture, decode, and UI stages run on separate threads over bounded crossbeam channels, so a slow terminal does not stall capture and a packet burst does not stall the UI.
    - Each stage applies backpressure through the bounded channel instead of queueing without limit.
2. **Untrusted wire data**
    - Packets are decoded without panicking: every decoder validates length before indexing.
3. **Offline and local-first**
    - There is no network client dependency; GeoIP and ASN enrichment reads local `.mmdb` files only.
    - Live capture opens the interface in promiscuous mode (`start_live_capture`).
4. **Observation only**
    - The egress monitor observes and alerts; it never blocks traffic.
5. **Shared store**
    - One `Store` (SQLite through `rusqlite`, bundled) sits behind a mutex shared by the correlate/detect worker (writes) and the IPC server (reads).

---

## 2. Workspace Layout

1. **Crates**
    - `network-monitor` (repository root): the binary package (`network-monitor`). CLI dispatch (clap), the capture pipeline wiring, and the two ratatui frontends (capture TUI, inspector). Its `[workspace]` table makes the root the workspace root.
    - `network-monitor-core` (`crates/network-monitor-core`, the only other workspace member): library (`network_monitor_core`). Capture, decoders, flow tracking, filters, process attribution, name/GeoIP/identity enrichment, the SQLite store, the alert engine, the monitor daemon, and Unix-socket IPC.
    - The root `Cargo.toml` keeps the shared `[workspace.package]` and `[workspace.dependencies]` tables, so the core crate inherits its version and dependency versions; the binary depends on the core crate by path plus version for crates.io.
    - Edition 2024, `rust-version` 1.88 (the code uses let chains), version 0.1.2, Apache-2.0; both crates publish the root `README.md` as their crates.io readme.
2. **Platform gating**
    - In the core crate, `inspector`, `ipc`, and `monitor` are `#[cfg(unix)]`; `libc` is a Unix-only dependency in both crates.
    - In the binary, `inspect` and the `monitor` and `inspect` command bodies are `#[cfg(unix)]`; on other targets those commands return an error.
    - `process` has one implementation per OS (`linux.rs`, `macos.rs`, `windows.rs`) and returns nothing on other targets.
    - Windows builds the capture, decode, analysis, and TUI surface without the daemon.
3. **Tree**

```markdown
network-monitor/
├─ Cargo.toml, Cargo.lock
├─ build.rs
├─ LICENSE.md
├─ README.md, README-ko.md
├─ ARCHITECTURE.md, ARCHITECTURE-ko.md
├─ .github/workflows/ci.yml, release.yml
├─ assets/icon/ (network-monitor.webp, network-monitor-small.webp, network-monitor.ico, network-monitor.rc, build_icon.py)
├─ docs/config.example.toml
├─ src/
│  ├─ main.rs
│  ├─ core/ (cli.rs, config.rs, permissions.rs)
│  ├─ inspect/ (mod.rs, state.rs, views.rs, glue.rs, tests.rs)
│  └─ tui/ (mod.rs, state.rs, theme.rs, i18n.rs, layout.rs, layout_store.rs, state_writer.rs, link_guard.rs, interaction.rs, format.rs, cards.rs, columns.rs, sort.rs, stats.rs, capture_io.rs, terminal.rs, views/)
└─ crates/network-monitor-core/
   ├─ Cargo.toml
   └─ src/
      ├─ lib.rs, app_dir.rs, debug.rs, error.rs, identity.rs
      ├─ alert/ (mod.rs, detectors.rs, baseline.rs)
      ├─ analysis/ (mod.rs, anomaly.rs, threat.rs)
      ├─ capture/ (mod.rs, live.rs, file.rs)
      ├─ decode/ (mod.rs, ethernet.rs, arp.rs, ipv4.rs, ipv6.rs, tcp.rs, udp.rs, icmp.rs, icmpv6.rs, dns.rs, tls.rs, ja.rs, http.rs, http2.rs, quic.rs)
      ├─ diff/, notify/, storage/ (mod.rs, ring.rs)
      ├─ enrich/ (mod.rs, geoip.rs, names.rs)
      ├─ filter/ (mod.rs, ast.rs, parser.rs, eval.rs)
      ├─ flow/ (mod.rs, tracker.rs, reassembly.rs, dns_tracker.rs, stats.rs)
      ├─ output/ (mod.rs, json.rs, log_writer.rs, pcap_writer.rs, pcapng_writer.rs)
      ├─ process/ (mod.rs, linux.rs, macos.rs, windows.rs)
      ├─ store/ (mod.rs, models.rs, schema.rs)
      └─ Unix only: inspector/, ipc/ (client, server, protocol), monitor/ (correlate, daemonize, paths)
```

```markdown
src/main.rs: command dispatch, decode thread, JSON output loop, daemon and inspector entry points
src/core/cli.rs: clap definitions for every command and option
src/core/config.rs: optional config.toml (default interface, color_scheme accepted but unused, saved filters)
crates/network-monitor-core/src/app_dir.rs: the single per-user folder ~/.config/network-monitor/ (config, daemon state, saved capture logs)
src/core/permissions.rs: capture privilege check (Linux: root required; Windows: Npcap and Administrator note)
src/tui/: the terminal UI kit shared by both TUIs and the capture screen (see Terminal UI)
src/inspect/: inspector TUI: plain-row state and views on every platform, daemon socket glue on Unix only
build.rs: Windows resource embedding and, for gnullvm targets, libunwind.dll copy
assets/icon: the two WebP masters, the generated ICO, the resource script, and the ICO regeneration script
docs/config.example.toml: every config.toml key with its default value
crates/network-monitor-core: the engine library, a workspace member
```

---

## 3. Capture Pipeline

1. **Live path**
    - Three threads over bounded crossbeam channels (capacity 10 000 each).

```markdown
pcap interface
  |
  v
[capture thread]  capture::live::start_live_capture
  |  RawPacket  (bounded channel, cap 10_000)
  v
[decode thread]   decode_thread
  |   decode::decode_packet  -> flow::tracker::update  -> process attribution
  |  DecodedPacket  (bounded channel, cap 10_000)
  v
[UI thread]       tui::run_tui   (or json_output_loop when --json)
     PacketRing (bounded) + ratatui render loop (redraw on change, 1 s idle)
```

2. **Stages**
    - The capture thread translates libpcap records into `RawPacket` and stops on a shared `AtomicBool`.
    - The decode thread runs the layered decoder chain, updates flow state (reassembly, RTT, retransmission), and attaches the owning process.
    - The UI thread drains decoded packets into a bounded `storage::ring::PacketRing` and redraws when state changed or after a 1 s idle interval, waiting at most 50 ms for input.
    - The pcap writer (`-w`) and the log export run on their own threads; the UI thread only queues packet copies.
3. **JSON mode**
    - `--json` swaps the TUI for `json_output_loop`, which writes JSON lines and stops at the optional count (`-c`) or duration (`-G`) limit.
    - `-w` and these limits belong to different modes: `-w` is passed to the TUI only, and `-c` and `-G` to the JSON loop only.
4. **Offline path**
    - `read` replaces the live capture thread with `capture::file::start_file_capture`; the BPF filter is applied to the file by libpcap, and the rest of the pipeline is identical.
5. **Direction**
    - The TUI classifies packets as IN or OUT from the interface's local IP addresses; with none (reading a file), it falls back to a private/public address heuristic.

---

## 4. Decoder Chain

1. **Chain**
    - `decode::decode_packet` walks a `DecodeResult` / `NextDecode` chain from the link layer up.
    - Each protocol decoder returns `Option`, validates every length before slicing, and uses saturating arithmetic so a malformed or truncated packet stops the chain cleanly instead of panicking.
2. **Supported layers**
    - Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS (questions and answers), TLS (SNI plus JA3/JA4 fingerprints), HTTP/1.1, cleartext HTTP/2 (HPACK), and QUIC long-header detection.
    - HPACK Huffman literals are not expanded, and QUIC Initial decryption is not implemented.
3. **Summary**
    - `compute_summary` derives the human-readable summary and the color hint used by the TUI.

---

## 5. Module Responsibilities

1. **Capture and decode**
    - `capture`: libpcap live and offline capture, the shared `capture_loop`, and the `Linktype` mapping.
    - `decode`: per-protocol decoders, the `DecodedPacket` and `Layer` model, and summary computation.
    - `flow`: `FlowTracker` (an LRU cache keyed by a normalized 5-tuple `FlowKey`), TCP reassembly, DNS query/response pairing, and RTT estimation.
    - `filter`: nom-based display-filter parser, the `FilterExpr` AST, and per-packet `eval_filter`.
    - `storage`: `PacketRing`, the bounded ring that backs the TUI scrollback.
2. **Analysis and attribution**
    - `analysis`: conservative cleartext anomaly checks (`anomaly`) and the stateful recon tracker (`threat`: port scan, host sweep, rare destination).
    - `process`: OS socket-table lookup that maps a 5-tuple to a PID, name, and executable path (Linux, macOS, Windows).
    - `enrich`: passive name resolution (DNS answers plus TLS SNI) and offline GeoIP/ASN lookup behind an `Enricher` trait.
    - `identity`: binary identity (macOS code signature, otherwise SHA-256 of the executable) with an LRU cache keyed by path, mtime, and size.
3. **Egress monitor (Unix)**
    - `store`: SQLite data-access layer with parameterized queries; schema and models live alongside.
    - `alert`: the five-signal alert engine and the EWMA volume baseline.
    - `monitor`: the always-on daemon (capture, the correlate/detect worker, `run`/`status`/`stop`, daemonize, runtime paths).
    - `ipc`: Unix-socket request/response protocol, server, and client used by `inspect`.
    - `inspector`: the reducer that turns IPC events into inspector view state.
    - `notify`: best-effort desktop notifications (macOS `osascript`, Linux `notify-send`) plus stderr logging.
4. **Output and support**
    - `output`: JSON lines, the plain-text log writer, and pcap/pcapng writers behind a `PacketSink` trait.
    - `diff`: content-based pcap diff.
    - `error`: the shared `NetworkMonitorError` enum.
    - `debug`: opt-in tracing enabled by `NETWORK_MONITOR_DEBUG`.

---

## 6. Egress Monitor

1. **Threads**
    - `monitor run` starts a capture thread and an `ipc-server` thread, and runs the correlate/detect worker on the calling thread.
    - The capture-to-worker channel is bounded (10 000).
2. **Per flow**
    - The worker correlates the 5-tuple to a PID, identity, and destination name, enriches it with offline country/ASN, and builds a `FlowEvent`.
    - `AlertEngine::evaluate_full` runs all detectors for the event inside one SQLite transaction.
    - A flow without process attribution is still recorded as a destination, but raises no per-process signal.
3. **Baseline**
    - The first event starts a learning window (7 days by default, 5 seconds with `--demo`).
    - During the window the baseline is populated and no alert is emitted; afterwards novel observations fire.
4. **Signals**
    - New process to destination: a new organization, registrable domain, or IP label for a process (notice).
    - New process phoning home: first outbound connection of an executable (warning).
    - New country / ASN: first country or ASN seen for a process (notice).
    - Volume / exfil spike: outbound bytes per 60-second interval, EWMA baseline with alpha 0.3; fires at z of 4.0 or more, at least 5 MiB, and at least 8 samples (warning, or critical at 10 times the mean).
    - Program modification: identity changed since baseline, then a connection out (critical). A re-signed update with the same team and signing identity is flagged only when `flag_signed_updates` is set, which the daemon leaves off.
5. **Cooldown**
    - An alert with the same `dedup_key` is dropped while the previous one is younger than the cooldown (1 hour by default).
6. **IPC**
    - The daemon listens on a Unix socket in the state directory and removes it on clean shutdown.
    - Requests used by the CLI: `Status`, `LiveConnections`, `RecentAlerts`, `RecentConnections`, `Subscribe`, and `Stop`; a subscribed client receives pushed events.
    - `monitor status`, `monitor stop`, `inspect`, and `inspect --json` are thin clients of this protocol.
7. **Daemonize**
    - `--daemonize` double-forks, calls `setsid`, and redirects stdout and stderr to `monitor.log` in the state directory.
8. **Signals and shutdown**
    - `SIGINT` and `SIGTERM` set the shared stop flag; `monitor stop` asks over IPC.

---

## 7. Terminal UI

1. **Module layout**
    - `src/tui/` is shared by both screens: `theme.rs` (Indexed-color palette), `i18n.rs` (en/ko catalog), `layout.rs` (split ratios and card order), `layout_store.rs` (state file), `state_writer.rs` (state writer thread), `link_guard.rs` (symbolic-link resolution and the root guard), `interaction.rs` (gestures, injected clock), `format.rs` (width-aware text), `cards.rs` and `columns.rs` (card ids, hit areas, column definitions), `sort.rs` (typed sort keys), and `views/` (`chrome.rs` app bar, tabs, footer, help; `common.rs` cards, tables, scrollbar; the capture views).
    - `state.rs` holds the capture state and returns a `Command` from each key or mouse transition; `mod.rs` dispatches I/O commands to background threads.
    - Views are pure render functions over `&mut App` or `&mut Inspect` that only record hit areas; the hit areas are reset at the start of every frame.
2. **Terminal lifecycle**
    - `terminal.rs` enters raw mode, the alternate screen, mouse capture, and a hidden cursor together, and restores them from a `Drop` guard and a panic hook that share one idempotent restore.
    - The shared event loop (`TuiHost`) redraws when state changed or after a 1 s idle interval, drains queued input before drawing, ignores key release events, and shortens its wait to the next timer (a toast expiry or the long-press threshold).
3. **Capture TUI**
    - Layout: app bar, filter bar, packet list with the detail card beside it, the hex card, a throughput strip (only when the screen is at least 24 rows tall), and the footer.
    - The list, detail, and hex panes are cards in two groups: `CaptureRows` (main area over hex, resizable) and `CaptureCols` (list and detail, resizable and reorderable).
    - The packet list draws only the rows in view over the whole ring, and a sort orders the visible indices by a typed key (cached, rebuilt at most every 500 ms while packets arrive).
    - Full-area statistics views (top talkers, protocol distribution, flows, timeline, follow-stream, bookmarks) read a cache rebuilt at most every 500 ms.
    - Input modes: `Normal`, `FilterInput`, `Search`, and `ColumnConfig`; `LIVE` pins the view to the newest packet and manual navigation switches to `BROWSE`.
4. **Inspector TUI**
    - Five tabs (Connections, Processes, Domains, Alerts, History); each tab is a table card over a detail card in one reorderable group.
    - The core reducer is converted into plain rows (`inspect::state`), so state and views compile and test on every platform; only `inspect::glue` (IPC reader and query threads, the host) is `#[cfg(unix)]`.
    - It keeps the last state and shows a disconnected marker when the daemon goes away.
5. **Display language**
    - Every user-visible string is a `Msg` entry with an English and a Korean text in one macro table; English is the default and `L` switches without a restart.
    - Values that carry data (counts, names, paths, errors from the engine) go through format functions and are never translated.
    - Sorting uses typed keys, so switching the language never reorders rows or moves the selection.
6. **Tables**
    - A table card resolves column widths from natural content widths: fixed columns keep their width, text columns shrink widest-first to 3 cells, the right-most column not marked keep hides last, and user-dragged widths shrink last and are capped at 512.
    - Header clicks and `s`/`S` share one sort cycle (numbers and times descending first, text ascending first, third step back to capture order); the selection follows the same item across a re-sort.
    - The scrollbar sits in the gutter column inside the right border in a gray family, never the focus color; ` a-b / total ` is drawn on the bottom border.
7. **Arrangement**
    - A long press on a card title (400 ms, measured by `Ui::tick` with an injected clock) enters move mode; the order previews live, a release commits, `Esc`, a tab switch, or a resize restores the previous order, and a buttonless mouse move or a new press commits a lost release.
    - Boundaries are stored as ratios and clamped to each neighbor's minimum; slot sizes stay with the slot.

---

## 8. State and Files

1. **In-memory state**
    - Capture scrollback: `tui::App`, a `PacketRing` (bounded), lives for the UI session.
    - Flow table: `flow::FlowTracker`, an LRU cache of `FlowKey` to `FlowState`, lives for the capture session.
    - Live connections: the monitor worker, a `Mutex<LiveState>`, lives for the daemon.
2. **Persistent state**
    - Single per-user folder: `~/.config/network-monitor/` (home plus `.config`, so `%USERPROFILE%\.config` on Windows), resolved in one place, `network_monitor_core::app_dir`: `home_dir` takes the first non-empty of `USERPROFILE` and `HOME`, and `app_dir_in` takes the home path as an argument so tests never touch the real profile. There is no fallback folder: without a home directory the TUIs print one stderr line and turn state and log saving off, and `monitor::paths::resolve` fails unless `--state-dir` is given.
    - Config file: `config.toml` in that folder (read only, the program never writes it).
    - Daemon state: `network-monitor.sock`, `network-monitor.db`, `network-monitor.pid`, and `monitor.log` in that folder, or in the directory given with `--state-dir`.
    - Saved capture log: `network-monitor-<YYYYMMDD-HHMMSS>.log` in that folder, written on a background thread by `tui::capture_io::write_log_in` (key `w`); as root, a symbolic link owned by another user anywhere on the path is refused, and without a home directory the export fails with an error toast.
    - TUI state: `tui.toml` in that folder, shared by both TUIs (`tui::layout_store`): `version`, `language`, card orders, split ratios, and column widths, keyed by stable group and card names. Saves go to the state writer thread (`tui::state_writer`): the UI thread builds the whole file text on a commit event and queues it, a burst for one path is coalesced to the newest text, a failed write returns as a typed message and an error toast, and the exit path waits at most 5 seconds for the thread and prints one stderr warning when the last write failed or is still running. The write is atomic (a temporary file created exclusively in the target folder, synced, then renamed) and only on commit events; a symbolic link chain (at most 16 links, `tui::link_guard`) is followed first so the links survive, and a process with effective user ID 0 refuses a link owned by another user. An unreadable or other-version file is copied to `tui.toml.bak` before the first overwrite, `tui.toml.<pid>.tmp` files at least 60 seconds old are removed at start (a younger one may be another instance mid-save), invalid entries are skipped, and column widths are capped at 512. The tests inject the path, the clock, and the owner lookup.

---

## 9. Display Filters

1. **Grammar**
    - Protocol atoms, field comparisons (`==`, `!=`), `contains`, `threat`/`alert`, boolean composition (`and`/`&&`, `or`/`||`, `not`/`!`, parentheses), and a free-text fallback; the field list is in [README.md](README.md).
    - `and` binds tighter than `or`.
2. **Parser**
    - The parser builds the canonical dotted field path once at parse time, so per-packet evaluation does not allocate for the path.
    - Input that is not a complete structured expression becomes `FreeText`, a case-insensitive match over the packet summary; only an empty filter is an error.
3. **Evaluation**
    - `tcp.port` and `udp.port` match either side; `!=` on them requires both sides to differ.

---

## 10. Build Notes

1. **Native dependencies**
    - The `pcap` crate links libpcap: `pkg-config` on Unix, `wpcap` on Windows (set `LIB`, or `LIBPCAP_LIBDIR`, to the Npcap SDK `Lib\x64` folder).
    - SQLite is compiled from source (`rusqlite` with the `bundled` feature), so a C compiler is required.
2. **`build.rs` (root package)**
    - On a Windows target it generates the manifest (`asInvoker`) and the resource script (icon from `CARGO_MANIFEST_DIR/assets/icon/network-monitor.ico`, English `VERSIONINFO` block `040904B0` with translation `0x409`, built from the `CARGO_PKG_*` values) in `OUT_DIR` and compiles them with `windres`; the candidates are the linker folder, then `PATH`, per target architecture (`<arch>-w64-mingw32-windres`, `llvm-windres --target=<arch>-w64-mingw32`, `windres`).
    - The first two bytes of each object (the COFF machine type) must match the target architecture, so an `aarch64-pc-windows-msvc` build never links an x64 object; a mismatching candidate is skipped.
    - When no `windres` works, it prints a warning and builds without the resources.
    - For a `gnullvm` target it copies `libunwind.dll` next to the built executable.
    - The packaged crate ships `build.rs` and `assets/icon/network-monitor.ico`; the `.webp` masters and `build_icon.py` are excluded, so a build needs no image tool.
3. **Icon**
    - `assets/icon/network-monitor.webp` (1024 px tile) feeds the 40 to 256 px entries and `network-monitor-small.webp` (256 px hexagon crop) feeds the 16 to 32 px entries of the ICO.
    - `build_icon.py` regenerates `network-monitor.ico` with ImageMagick; run it only when a master changes.

---

## 11. Validation Strategy

1. **Commands**
    - Use the narrowest commands that cover the changed surface.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --workspace
```

2. **Coverage**
    - The core crate carries unit tests for the decoders, flow tracker, filter, ring, writers, alert engine, and IPC; the binary crate carries `TestBackend` buffer tests of every view in English and Korean at 60x16, 100x30, and 130x40 (exact cells for focus, scrollbar, selection band, toast, and chips; hit areas through real mouse events), clock-injected tests for long press, drag, and lost release, unit tests for the sort cycle, column resolver, split ratios, the state store, the state writer (coalescing, bounded join), and the link guard (chains, loops, root refusal), buffer tests for the glyph budget and for double lines appearing only in overlays, plus config tests (injected home directory and `docs/config.example.toml` parsing).
    - The `monitor`, `ipc`, and `inspector` modules compile and run on Unix targets only; `notify` compiles everywhere but sends OS notifications on macOS and Linux only.
