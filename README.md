# network-monitor

## Overview

1. **Languages**
    - Korean version: [README-ko.md](README-ko.md)
    - English is the default language of these documents.
2. **What it is**
    - A terminal packet analyzer and a local egress monitor in one binary, `network-monitor`.
    - The packet analyzer captures from a live interface or reads a PCAP/PCAPNG file, decodes the traffic, and shows it in a keyboard-driven terminal UI (TUI).
    - The egress monitor is a background daemon that answers "what is my machine talking to, and is any of it new?": it ties each flow to a process, resolves destination names passively, enriches them with offline country/ASN data, learns a baseline, and raises alerts.
    - It has no network client dependency: GeoIP/ASN data comes from local files, and nothing is sent to any service.
3. **Highlights**
    - Live capture with BPF filters, PCAP/PCAPNG read and write, JSON-lines output for scripts, and a content diff of two capture files.
    - Decoders for Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS, TLS (SNI, JA3/JA4), HTTP/1.1, cleartext HTTP/2, and QUIC long headers.
    - TCP stream reassembly with a follow-stream view, RTT estimation, flow table, top talkers, protocol distribution, and a connection timeline.
    - Display filters with `and` / `or` / `not`, saved named filters, regex search, bookmarks, and inbound/outbound views.
    - Process attribution for each flow on Linux, macOS, and Windows.
    - Egress monitor with five alert signals: new process to destination, new process phoning home, new country or ASN, outbound volume spike, and program modification.
4. **Platforms**
    - Packet analyzer: Linux, macOS, and Windows.
    - Egress monitor (`monitor`, `inspect`): Unix only (Linux, macOS); on Windows these commands exit with an error.
    - The project is developed macOS-first, and Windows process attribution is lightly tested.
5. **Documents**
    - Source structure and internal flow: [ARCHITECTURE.md](ARCHITECTURE.md) (Korean: [ARCHITECTURE-ko.md](ARCHITECTURE-ko.md))
    - Example configuration: [docs/config.example.toml](docs/config.example.toml)
    - License: [LICENSE.md](LICENSE.md) (Apache-2.0)
6. **Badges**
    - [![Crates.io](https://img.shields.io/crates/v/network-monitor.svg)](https://crates.io/crates/network-monitor) [![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE.md)

---

## Quick Start

### 1. Requirements

1. **Rust**
    - Rust 1.88 or later to build (`rust-version` in `Cargo.toml`, edition 2024).
    - A C compiler is needed on every OS because SQLite is bundled (`rusqlite` with the `bundled` feature).
2. **Windows**
    - Runtime: the [Npcap](https://npcap.com/) runtime for x64; capture from an Administrator prompt.
    - Build: the Npcap SDK too. The `pcap` crate links `wpcap`, so point the `LIB` environment variable (or `LIBPCAP_LIBDIR`) at the SDK `Lib\x64` folder.
    - Optional: a resource compiler (`<arch>-w64-mingw32-windres`, `llvm-windres`, or `windres`). `build.rs` generates the resource script and uses it to embed the app icon, the English version info, and an `asInvoker` manifest into the exe; a tool that writes an object for another CPU architecture is skipped, and without any tool the build prints a warning and still succeeds.
3. **Linux**
    - The libpcap development package to build: `libpcap-dev` on Debian/Ubuntu, `libpcap-devel` on Fedora.
    - Capture needs root or the `CAP_NET_RAW` capability: run with `sudo`, or grant it once with `setcap` (see Troubleshooting).
4. **macOS**
    - The system libpcap is used; install the command-line tools (`xcode-select --install`) to build.
    - The app itself does not check privileges on macOS; opening the capture device is up to the OS, and `sudo` is the simple route.
5. **No privileges needed**
    - `read` and `diff` work on files without elevated rights; `list-interfaces` only lists devices.

### 2. Install

1. **From crates.io**
    - On Windows, point `LIB` at the Npcap SDK `Lib\x64` folder first.

```bash
cargo install network-monitor
```

```powershell
$env:LIB = "<Npcap SDK folder>\Lib\x64;$env:LIB"
cargo install network-monitor
```

2. **From source**
    - Clone the repository and build with plain cargo; on Windows set `LIB` the same way first.
    - The binary is `target/release/network-monitor` (`network-monitor.exe` on Windows).

```bash
git clone https://github.com/jungho-dev/network-monitor.git
cd network-monitor
cargo build --release
```

3. **Workspace**
    - The repository root is the `network-monitor` package: the CLI binary with the two TUIs.
    - `crates/network-monitor-core` is a workspace member: the reusable engine library `network-monitor-core`.
    - Both crates publish this README as their crates.io readme.

### 3. Configure

1. **Optional config file**
    - The tool runs without one. A config file sets the default interface and saved filters.
    - Path: `~/.config/network-monitor/config.toml` on every OS; on Windows that is `%USERPROFILE%\.config\network-monitor\config.toml`.
    - The home directory is the first non-empty of `USERPROFILE` and `HOME`; the path does not follow `$XDG_CONFIG_HOME`.
    - The same folder holds every other per-user file (daemon state and saved capture logs); see Configuration.
2. **Minimal example**
    - Create the folder and the file yourself; nothing writes it for you.
    - [docs/config.example.toml](docs/config.example.toml) lists every key with its default; copy it to the path above and edit it.

```toml
[capture]
default_interface = "en0"        # used when -i is omitted

[filters]                        # recall in the filter bar as :name
https = "tcp.port == 443"
dns   = "udp.port == 53"
```

```markdown
default_interface: the interface for capture when -i is omitted
filters: each key is a saved display filter; type :https in the filter bar to apply it
all keys are listed in the Configuration section
```

### 4. First run

1. **List interfaces**
    - Pick the interface name you want to watch.

```bash
network-monitor list-interfaces
```

2. **Start the live TUI**
    - Without `-i` it uses `capture.default_interface`, otherwise it auto-selects the active interface (up, running, connected, non-loopback, real IPv4 address) and prints the choice to stderr.

```bash
sudo network-monitor
sudo network-monitor capture -i en0
```

```markdown
On Windows run the same commands without sudo from an Administrator prompt
-i names the interface; -f adds a BPF capture filter such as "tcp port 443"
```

3. **Read the screen**
    - From top to bottom: the app bar (interface or file, capture state, counters), the tab bar (Packets, Flows, Stats, Alerts, Stream, Settings), the tab body, and the footer chips with a hint row.
    - The Packets tab holds the filter bar, the packet list (Time, Dir, Process, Host, IP, Proto, Len, Info) with the detail card beside it, the hex card, and the throughput strip.
    - Press `1` to `6` or `Tab` to switch tabs, and `v` to raise the output level (`debug`, `info`, `warn`, `error`) so only warnings and errors stay in the list.
    - The app bar shows `LIVE`, `BROWSE`, or `PAUSED`, the packet and dropped counters, and a freshness dot.
    - Press `/` to filter, `Space` to pause, `w` to save the visible packets as a log file, `L` to switch the display language, `?` for every key, and `q` to quit. Full key list: Keybindings; screen behavior: Terminal UI.
4. **Try it without privileges**
    - Open a saved capture file; no elevated rights are needed.

```bash
network-monitor read capture.pcap
```

### 5. Egress monitor

1. **Start the daemon**
    - Unix only. `monitor run` needs an explicit `-i` and the same capture privileges as `capture` (root on Linux); it runs in the foreground until `Ctrl-C` or `monitor stop`.
    - `--demo` shortens the learning window to 5 seconds so the alert signals fire quickly; without it the baseline learns for 7 days and raises no alerts meanwhile.

```bash
sudo network-monitor monitor run -i en0 --demo
```

2. **Attach the inspector**
    - In a second terminal; it connects over a local Unix socket.

```bash
network-monitor inspect
network-monitor monitor status
network-monitor monitor stop
```

3. **Same state directory**
    - The daemon, `inspect`, and `monitor status/stop` find each other through the state directory (`network-monitor.sock` inside it).
    - Under `sudo` the default state directory can differ from your own; pass the same `--state-dir <dir>` to every command to be sure.
4. **Optional offline country/ASN data**
    - Download country and ASN databases in MaxMind DB (`.mmdb`) format, for example DB-IP Lite, and pass them with `--geoip-country-db` and `--geoip-asn-db`.
    - Data from db-ip.com is CC-BY-4.0 and needs attribution: © [db-ip.com](https://db-ip.com). MaxMind GeoLite2 files also work; none is bundled or fetched automatically.
    - Without a database (or when a file fails to open) country and ASN stay unknown and the monitor keeps running.

```bash
sudo network-monitor monitor run -i en0 \
  --geoip-country-db /path/to/dbip-country-lite.mmdb \
  --geoip-asn-db /path/to/dbip-asn-lite.mmdb
```

---

## Features

1. **Packet analyzer**
    - Live capture from any interface with a BPF filter, and PCAP or PCAPNG save (`-w`, `--pcapng`).
    - Decoders: Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS (questions and answers), TLS (SNI plus JA3/JA4 fingerprints), HTTP/1.1, cleartext HTTP/2 (HPACK), QUIC long-header detection.
    - TCP stream reassembly with RTT estimation; `f` opens the stream of the selected TCP packet in the Stream tab.
    - Six tabs in the default order (`1` to `6`): Packets (packet list, detail tree, hex dump, filter row, throughput strip), Flows (flows over the connection timeline), Stats (top talkers beside the protocol distribution), Alerts (threat alert table over bookmarks), Stream (follow-stream of the TCP stream opened with `f`), and Settings (output level and language); the tab order can be changed.
    - Regex search (`Ctrl-F`), bookmarks (`m`), inbound/outbound views (`i`, `o`), per-column visibility (`c`), and sortable tables with draggable columns (`s`, `S`, header click).
    - Mouse, display language (`L`), card arrangement, and a saved layout: see Terminal UI.
    - Cleartext warnings in the packet list: HTTP Basic credentials, an FTP `PASS`, and cleartext HTTP on an unusual port.
    - Live recon heuristics that tag packets and match the `threat` filter: `PORT-SCAN` (15 or more distinct ports on one host within 10 seconds), `HOST-SWEEP` (12 or more distinct hosts within 10 seconds), and `RARE-DEST` (a connection attempt or cleartext data to an uncommon port on a public host, tagged once per source, destination, and destination port within 60 seconds; never on traffic into a local address, on replies from a common service port to an ephemeral port, on segments without data, or on encrypted payloads such as TLS records and QUIC).
    - Output level (`v`, the Settings tab, or `[display] output_level`): `debug` (the default, shows everything), `info`, `warn`, or `error`. `error` is `PORT-SCAN` and `HOST-SWEEP`; `warn` is `RARE-DEST`, warning-severity anomalies, and retransmissions; `info` is info-severity anomalies; `debug` is everything else. Items below the level are hidden from the packet list, the alerts table, and the `w` log export; pcap `-w` output, `--json`, the daemon, and the Flows and Stats tabs are not filtered. Warn rows use a muted foreground and error rows a red foreground with no background fill.
    - JSON-lines output (`--json`) with optional packet count (`-c`) and duration (`-G`) limits.
2. **Egress monitor**
    - A background daemon captures continuously, maps each flow's 5-tuple to a process through the OS socket table, and resolves destination names passively from DNS answers and TLS SNI (no decryption).
    - Country and ASN come from local `.mmdb` files; state is stored in a local SQLite database.
    - Desktop notifications use `osascript` on macOS and `notify-send` on Linux; `--no-notify` limits output to stderr logging.
3. **Five alert signals**
    - New process to destination: a process contacts a domain or organization it never has (severity notice).
    - New process phoning home: a binary's first outbound connection (warning).
    - New country or ASN: first contact with a host in a new country or network (notice).
    - Volume spike: outbound bytes per 60-second interval far above the process's baseline, at least 5 MiB and at least 4 standard deviations above the mean after 8 samples (warning, or critical at 10 times the mean).
    - Program modification: the binary identity of a process changed since baseline and it then connects out (critical).
    - Binary identity is the code signature on macOS (a changed team or signing identity, or a valid-to-invalid regression, is flagged; a re-signed update with the same identity is not) and the SHA-256 of the executable elsewhere.
    - Alerts stay silent during the learning window, and a repeated alert is suppressed for one hour.
4. **Inspector**
    - A second TUI over the daemon socket with five views: Connections, Processes, Domains, Alerts, and History.
    - `inspect --json` prints one snapshot (status, live connections, up to 200 recent alerts) and exits.
5. **Monitor only**
    - The tool observes and alerts; it never blocks or firewalls traffic.

---

## Commands

1. **Global**
    - `network-monitor` with no subcommand starts the live capture TUI with default options.
    - `--help` and `--version` are provided by clap.
    - Every subcommand also accepts `--help`.
2. **`list-interfaces`**
    - Prints each interface name, description, and addresses.
3. **`capture`**
    - Live capture opens the interface in promiscuous mode.
    - `-i, --interface <name>`: interface; defaults to `capture.default_interface`, then auto-selection.
    - `-f, --filter <bpf>`: BPF capture filter applied by libpcap.
    - `-w, --write <file>`: save packets to a pcap file while the TUI runs (every received packet, not only the displayed ones).
    - `--pcapng`: write PCAPNG instead of classic pcap (with `-w`).
    - `--json`: print JSON lines instead of the TUI; `-w` is not used in this mode.
    - `-s, --snaplen <n>`: maximum bytes per packet, default 65535.
    - `--buffer-size <n>`: packets kept in memory, default 100000, at least 1.
    - `-c, --count <n>`: stop after n packets; takes effect with `--json`.
    - `-G, --duration <seconds>`: stop after n seconds; takes effect with `--json`.
4. **`read <file>`**
    - Opens a pcap or PCAPNG file in the TUI.
    - `-f, --filter <bpf>`: BPF filter applied while reading the file.
    - `--json`: print JSON lines.
    - `--buffer-size <n>`: packets kept in memory, default 100000.
5. **`diff <file_a> <file_b>`**
    - Compares two capture files by packet content and prints the common count plus up to 20 packets only in A and only in B.
6. **`monitor run`** (Unix only)
    - `-i, --interface <name>`: required.
    - `-f, --filter <bpf>`, `--snaplen <n>` (default 65535).
    - `--state-dir <dir>`: runtime state directory (database, socket, pid file, log).
    - `--geoip-country-db <file>`, `--geoip-asn-db <file>`: offline `.mmdb` databases.
    - `--demo`: 5-second learning window.
    - `--daemonize`: detach into the background (double fork); output goes to `monitor.log` in the state directory.
    - `--no-notify`: disable desktop notifications and log only.
7. **`monitor status`** and **`monitor stop`** (Unix only)
    - `monitor status [--state-dir <dir>] [--json]`: baseline state, pid, interface, uptime, and process, destination, and alert counts.
    - The status also carries `build_id` (size and modification time of the daemon's exe at startup), `exe_path` (that exe's path), and `run_args` (the daemon's own arguments); `inspect` compares `build_id` with the file now at `exe_path`, and a daemon from an older build without these fields is never flagged.
    - `monitor stop [--state-dir <dir>]`: asks the running daemon to shut down cleanly.
8. **`inspect`** (Unix only)
    - `--state-dir <dir>`: state directory used to find the daemon socket.
    - `--socket <path>`: use this socket path directly.
    - `--json`: print status, live connections, and recent alerts as JSON and exit.

```bash
sudo network-monitor capture -i eth0 -f "tcp port 443"
sudo network-monitor capture -i en0 -w out.pcapng --pcapng
sudo network-monitor capture -i en0 --json -c 100
network-monitor read capture.pcap -f "port 53"
network-monitor diff a.pcap b.pcap
```

---

## Display Filters

1. **Where**
    - Press `/` in the TUI, type a filter, and press `Enter`; an empty filter clears it.
    - `:name` applies a saved filter from `config.toml`.
    - This is separate from the BPF filter given with `-f`, which libpcap applies before decoding.
2. **Protocol atoms**
    - `eth`, `arp`, `ip`, `ipv4`, `ipv6`, `tcp`, `udp`, `icmp`, `icmpv6`, `dns`, `tls`.
    - `threat` (or `alert`) matches packets flagged by the live recon heuristics.
3. **Fields**
    - `ip.src`, `ip.dst`: IPv4 or IPv6 address.
    - `tcp.port`, `udp.port`: matches either the source or the destination port; `tcp.srcport`, `tcp.dstport`, `udp.srcport`, `udp.dstport` match one side.
    - `dns.qname`: the first DNS question name; `tls.sni`: the server name of a TLS ClientHello.
    - `icmp.type`, `icmp.code`, `arp.sender.ip`, `arp.target.ip`.
4. **Operators**
    - Comparison: `==` and `!=` only (no ordering operators). `tcp.port != 80` means neither side uses port 80.
    - Containment: `<field> contains <text>` is a case-sensitive substring test on a string field such as `tls.sni` or `dns.qname`.
    - Boolean: `and` / `&&`, `or` / `||`, `not` / `!`, and parentheses; `and` binds tighter than `or`.
5. **Free text fallback**
    - Input that is not a complete structured filter becomes a case-insensitive text match on the packet summary (info, source, destination, protocol, length). A typo such as `tpc.port == 443` therefore filters by text instead of reporting an error.
6. **Examples**

```markdown
tcp.port == 443 and ip.src == 10.0.0.1
(tcp || udp) && not dns
tls.sni contains example.com
dns.qname contains example
icmp.type == 8
threat
google
```

7. **Relation to Wireshark**
    - A few field names (`ip.src`, `tcp.port`) follow Wireshark's naming, but this is a much smaller language, not Wireshark's display-filter syntax.

---

## Configuration

1. **Precedence**
    - Command-line options, then `config.toml`, then built-in behavior (auto-selected interface).
2. **Keys in `config.toml`**
    - File: `~/.config/network-monitor/config.toml`; a commented copy of every key is [docs/config.example.toml](docs/config.example.toml).
    - `[capture] default_interface`: interface used when `-i` is omitted.
    - `[display] color_scheme`: read and defaults to `"dark"`, but not applied yet.
    - `[display] output_level`: minimum output level of the capture TUI, `"debug"` (default), `"info"`, `"warn"`, or `"error"`; an unknown value prints one warning on stderr and falls back to `debug`. The `v` key and the Settings tab override it, and the choice is saved in `tui.toml`.
    - `[filters]`: table of `name = "filter"`; recall one in the filter bar as `:name`.
    - A file that fails to parse is ignored with a message on stderr (`network-monitor: ignoring invalid <path>: ...`) and defaults are used.
3. **One folder for every per-user file**
    - Folder: `~/.config/network-monitor/` on every OS (`%USERPROFILE%\.config\network-monitor\` on Windows), under the first non-empty of `USERPROFILE` and `HOME`.
    - Without a home directory there is no shared fallback folder: the TUIs print one stderr line and turn state and log saving off, and the `monitor` commands need `--state-dir`.
    - Files inside: `config.toml`, `tui.toml` (TUI state), `network-monitor.sock` (IPC socket), `network-monitor.db` (SQLite database), `network-monitor.pid`, `monitor.log` (daemon output after `--daemonize`), and the saved `network-monitor-<YYYYMMDD-HHMMSS>.log` files.
    - `--state-dir <dir>` moves the daemon files (socket, database, pid file, `monitor.log`) to `<dir>`; `config.toml` and saved capture logs stay in the folder above.
    - The database holds process names, executable paths, destination names and IPs, and alerts: keep the folder private and do not share it.
4. **Saved capture log**
    - `w` in the TUI writes the visible packets to `network-monitor-<YYYYMMDD-HHMMSS>.log` (UTC time) in `~/.config/network-monitor/` on a background thread; a toast shows the full path when it finishes.
    - `w` replaces the old `s` key, which now sorts the focused table.
    - A process running as root (effective user ID 0) refuses to write through a symbolic link owned by another user and shows an error toast.
5. **Debug tracing**
    - `NETWORK_MONITOR_DEBUG=1` (any non-empty value other than `0`) prints effective parameters and IPC traffic to stderr.

---

## Keybindings

1. **Capture TUI: move**
    - `j` / `Down`, `k` / `Up`: move the selected row of the focused card; the detail and hex cards scroll instead.
    - `g` / `Home`: first row; `G` / `End`: last row and back to `LIVE`.
    - `PageUp` / `PageDown`: one screen of rows.
    - Moving up or jumping to the top leaves `LIVE` for `BROWSE`; `G` or landing on the last row returns to `LIVE`.
2. **Capture TUI: sort**
    - `s`: sort by the next column of the focused table; the cycle ends in capture order.
    - `S`: flip the direction of the active column, then reset to capture order.
    - Both follow the same three steps as a header click; the active sort is named in the card title (`Sort Len↓`).
3. **Capture TUI: filter and search**
    - `/`: filter bar (`Enter` apply, `Esc` cancel, `:name` recalls a saved filter).
    - `Ctrl-F`: regex search over info, source, and destination (`Enter` runs it, `Esc` clears it); `n` jumps to the next match.
    - `i` / `o`: show inbound-only / outbound-only packets; press again to clear.
4. **Capture TUI: views**
    - `1` to `6` open the tabs; `Tab` / `Right` next tab, `Shift-Tab` / `Left` previous tab; `Enter` focuses the next card (on Settings it changes the selected setting).
    - `t` / `P` Stats, `F` / `T` Flows, `'` Alerts, `f` opens the stream of the selected TCP packet in the Stream tab; `Esc` goes back to Packets.
    - `<` / `>` move the active tab left or right; dragging a tab on the tab bar reorders it (`Esc` cancels).
    - `d` toggles the detail card, `x` the hex card, `z` both.
    - `c` opens the column editor: `1` to `8` toggle Time, Dir, Process, Host, IP, Proto, Len, Info; `Esc`, `Enter`, or `c` closes it.
5. **Capture TUI: actions**
    - `Space`: pause or resume.
    - `m`: bookmark the selected packet.
    - `w`: save the visible packets to a log file.
    - `v`: cycle the output level (`debug`, `info`, `warn`, `error`).
    - `Shift` with an arrow key: move the focused card (see Terminal UI).
    - `l`: reset card order, sizes, and column widths; `L`: switch the display language.
    - `?`: help; `q` or `Ctrl-C`: quit; `Esc`: back to Packets, and quit on Packets.
6. **Inspector**
    - `Tab` / `Right` next view, `Shift-Tab` / `Left` previous view, `1` to `5` jump to Connections, Processes, Domains, Alerts, History.
    - `j` / `Down`, `k` / `Up`, `PageUp` / `PageDown`, `Home` / `End`: move the selected row.
    - `Enter`: move the focus between the table card and its detail card.
    - `s` / `S`: sort the focused table, the same cycle as the capture TUI.
    - `/`: search the Connections view (`Enter` keeps it, `Esc` clears it).
    - `r`: refresh status and connections.
    - `Shift` with an up or down arrow: move the focused card; `l`: reset the current view's layout; `L`: switch the display language.
    - `?`: help; `q`, `Esc`, or `Ctrl-C`: quit.
    - `!`: force restart, only while the daemon's own exe was replaced by another build on disk (checked every 2 seconds): stops the daemon, starts it again from that path with its own `monitor run` arguments plus `--daemonize`, and reruns `inspect`. It first checks capture privilege (of this `inspect` and of the new exe, by running it as a hidden `monitor preflight` command), write access to the state directory, absolute paths in the daemon's arguments, and the exe's owner; a failed check only shows an error toast and leaves the daemon running.
    - While the daemon's exe on disk is another build, the app bar shows `● Build mismatch` and a warn-colored `[!] Force restart` chip leads the footer; clicking it does what `!` does.

---

## Terminal UI

1. **Look**
    - One style for both TUIs: an app bar (product badge, interface or file, capture state, freshness dot), cards, clickable footer chips with help and quit pinned right, and a hint row.
    - The capture screen stacks the app bar, the tab bar, the body of the active tab, the throughput strip (Packets tab only, when the screen is at least 24 rows tall), and two footer rows (chips and hints); a `[v] <level>` chip sits before the language chip.
    - A card has a rounded border; the card under the pointer gets a brighter border, and the focused card gets a thick border with an inverted title.
    - Tables show a scrollbar in its own gray gutter column (`░` track, `█` thumb) and ` a-b / total ` on the bottom border; the selected row is a colored band.
    - The hint row shows the keys that apply to what is under the pointer, then to the focused card; a toast stays 4 seconds and only an error toast is a red fill.
    - `?` opens the help overlay (double border, dimmed backdrop, keyboard and mouse sections); any key or click closes it. Double lines appear only in overlays.
    - Besides Korean text, only ASCII, box drawing, `·`, `…`, `●`/`○`, and `↑`/`↓` are drawn, so cell widths hold in every terminal font: the packet detail tree uses `>`, the follow-stream view marks client data `>>` and server data `<<`, and arrows in packet summaries are drawn as `->`.
    - A terminal smaller than 60x16 shows one warning line and nothing else.
2. **Mouse**
    - Click a card to focus it and a row to select it; the wheel scrolls the card under the pointer.
    - Click a column header to sort: numbers and times go descending first, text ascending first, and the third click returns to capture order.
    - Drag the edge between two column headers to resize a column; double-click the edge to return to automatic width.
    - Drag the boundary between cards to resize them; drag the scrollbar thumb, or click the track to page.
    - Footer chips, tabs, and the language chip are clickable and run the action of their key.
3. **Arrangement**
    - Hold the left button on a card title for 400 ms, then drag to reorder the cards of that group; the order previews live, releasing commits, and `Esc` cancels.
    - The card being moved has a light double-dashed border in the accent color.
    - A released button that the terminal never reports is treated as released at the next mouse move or press; this ends a card move, a boundary or column-edge drag, and a scrollbar drag, and a layout change is saved. Switching the inspector view ends a drag the same way.
    - `Shift` with an arrow key moves the focused card one place; `l` restores order, sizes, and column widths of the current screen and keeps the language.
    - Slot sizes stay with the slot: moving a card swaps positions, not sizes.
4. **Display language**
    - Every screen text comes from one English and Korean catalog; English is the default.
    - `L` (chip `[L] EN` or `[L] KO`) switches the language at once, without a restart, and the choice is saved.
    - Packet data, addresses, protocol names, process names, and filter text are never translated; the detail card keeps protocol field labels as written in the protocol.
5. **State file**
    - Path: `~/.config/network-monitor/tui.toml`, next to `config.toml`; both TUIs read and write it.
    - Content: `version`, `language`, `output_level`, `capture_tab_order`, card orders, split ratios, and column widths; only values that differ from the default are written (`output_level` only after a choice), and it is saved only when a change is committed (a drop, a drag release, a reset, a language switch, an output level change, a tab move).
    - Saves run on a background writer thread: a burst of changes is coalesced to the newest state, a failed save shows an error toast, and on exit the program waits at most 5 seconds for the writer and prints one stderr warning if the last save failed or is still running.
    - The file is replaced through a temporary file in the same folder, so an interrupted save leaves the old file; a symbolic link (a chain of up to 16 links) is kept and its final target is updated, and as root a link owned by another user is refused.
    - Temporary files an interrupted save left behind are removed at start only when they are at least 60 seconds old, so a save of another running instance is not disturbed.
    - An unreadable or other-version file is copied to `tui.toml.bak` before the first save overwrites it; invalid entries in a hand-edited file are skipped, and column widths are capped at 512 cells.
    - Delete the file to return to the default layout and English.

---

## Troubleshooting

1. **`Capture needs root or the CAP_NET_RAW capability` on Linux**
    - Run with `sudo`, or grant the capability once so a normal user can capture: `sudo setcap cap_net_raw,cap_net_admin=eip "$(command -v network-monitor)"`.
    - The check accepts an effective user ID of 0 or `CAP_NET_RAW` in the process's effective capability set; rebuilding or reinstalling the binary drops the capability, so run `setcap` again.
2. **`sudo: network-monitor: command not found`**
    - `sudo` may not include `~/.cargo/bin` in its `PATH`; run `sudo env "PATH=$PATH" network-monitor` or use the full path.
3. **No interfaces or no packets**
    - `list-interfaces` prints `No interfaces found. You may need elevated privileges.` when the device list is empty.
    - On Windows install the Npcap x64 runtime and use an Administrator prompt.
    - Pick the interface explicitly with `-i <name>`; the automatic choice can differ from the one that carries your traffic.
4. **Build fails to find libpcap**
    - Linux: install `libpcap-dev` (Debian/Ubuntu) or `libpcap-devel` (Fedora); the build uses `pkg-config` unless `LIBPCAP_LIBDIR` is set.
    - Windows: the Npcap SDK is not on the linker path; set `LIB` (or `LIBPCAP_LIBDIR`) to the SDK `Lib\x64` folder, not the x86 `Lib` folder.
5. **`the egress monitor daemon is only supported on Unix`**
    - `monitor` and `inspect` are not available on Windows; use `capture` and `read` there.
6. **`cannot connect to daemon at <path>`**
    - The daemon is not running, or `inspect` looks in a different state directory; start `monitor run` and pass the same `--state-dir` to both.
7. **No alerts appear**
    - The baseline is still learning: 7 days by default, 5 seconds with `--demo`. Check `network-monitor monitor status` (`learning` or `active`).
    - Only outbound connections that the OS socket table can attribute to a process raise per-process signals; run the daemon as root for full attribution across users.
8. **Country and ASN show as unknown**
    - No `.mmdb` file was passed, or it failed to open; the monitor does not report the failure.
9. **A filter does not behave as expected**
    - Anything that is not a complete structured expression becomes a text search; compare with the Display Filters section.
10. **Config values are ignored**
    - Check stderr for `network-monitor: ignoring invalid <path>` and confirm the file path for your OS in Configure.
11. **Layout or language is not restored**
    - A toast names `tui.toml` when it could not be read; the file is copied to `tui.toml.bak`, defaults are used, and the next commit writes a new file.
    - Invalid entries are skipped; delete `~/.config/network-monitor/tui.toml` to start fresh.

---

## Limitations

1. **Monitor only**
    - No blocking or firewalling, by design.
2. **Protocols**
    - QUIC Initial decryption is not implemented; DNS-derived names cover the same destinations.
    - HPACK Huffman-coded literals in HTTP/2 are not expanded (HTTP/2 is usually TLS-encrypted anyway); only prior-knowledge cleartext h2c is decoded.
3. **Platforms**
    - The egress monitor daemon and inspector are Unix only.
    - Windows process attribution is lightly tested.
4. **Not applied yet**
    - `display.color_scheme` is parsed but does not change the theme (the one dark palette is always used), and key bindings cannot be remapped.
5. **Memory**
    - Packets live in a bounded ring (`--buffer-size`, default 100000); old packets are dropped from the view. Write a pcap with `-w` to keep everything.

---

## Development

1. **Checks**
    - `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo build --workspace`.
    - The `monitor`, `ipc`, and `inspector` modules compile and run on Unix targets only.
2. **Structure**
    - Crate and module layout: [ARCHITECTURE.md](ARCHITECTURE.md).

---

## License

1. **License**
    - Apache-2.0, see [LICENSE.md](LICENSE.md).
    - GeoIP data, if you download it, keeps its own license (DB-IP Lite: CC-BY-4.0, © db-ip.com).
