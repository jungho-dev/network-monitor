//! Packet list rows, the protocol detail tree, and the hex dump. Packet data,
//! addresses, protocol names, and field labels are shown as captured and are
//! never translated; only chrome text comes from the catalog.

use std::net::IpAddr;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use network_monitor_core::analysis::{ThreatKind, analyze};
use network_monitor_core::decode::{
    ColorHint, DecodedPacket, Direction, DnsRdata, Layer, TlsHandshakeMessage,
};

use crate::tui::cards::CardId;
use crate::tui::i18n::{self, Msg};
use crate::tui::state::App;
use crate::tui::theme;
use crate::tui::views::common::{
    CardTable, TableRow, TextCard, empty_pair, render_card_table, render_text_card,
    table_visible_rows,
};

// 1. endpoints ----------------------------------------------------------------
// Remote side of a packet as (ip, port): inbound packets use the source,
// everything else the destination. (None, None) for non-IP packets.
pub fn remote_endpoint(
    packet: &DecodedPacket,
    local_ips: &[IpAddr],
) -> (Option<IpAddr>, Option<u16>) {
    let Some((src, dst)) = packet.ip_pair() else {
        return (None, None);
    };
    let inbound = matches!(packet.direction(local_ips), Direction::In);
    let remote_ip = if inbound { src } else { dst };
    (Some(remote_ip), remote_port(packet, !inbound))
}

// Transport port on the remote side: the destination port when the local host
// sends, the source port when it receives.
fn remote_port(packet: &DecodedPacket, to_dst: bool) -> Option<u16> {
    for layer in &packet.layers {
        match layer {
            Layer::Tcp(tcp) => return Some(if to_dst { tcp.dst_port } else { tcp.src_port }),
            Layer::Udp(udp) => return Some(if to_dst { udp.dst_port } else { udp.src_port }),
            _ => {}
        }
    }
    None
}

// Drops the leading `<sport> → <dport> ` token of a TCP/UDP info string (the
// ports live in the IP column); the original summary stays untouched.
pub fn strip_leading_ports(info: &str) -> &str {
    let Some(arrow) = info.find(" → ") else {
        return info;
    };
    let after = &info[arrow + " → ".len()..];
    match after.find(' ') {
        Some(space) => after[space + 1..].trim_start(),
        None => info,
    }
}

// 2. packet list --------------------------------------------------------------
// One display row: cells for every column plus the row style (selection,
// threat, search, anomaly, retransmission, then direction tint).
fn packet_row(app: &App, visible_idx: usize, display_row: usize) -> Option<TableRow> {
    let packet = app.visible_packet(visible_idx)?;
    let selected = visible_idx == app.selected;
    let anomaly = analyze(packet);
    let direction = packet.direction(&app.local_ips);
    let matches_search = app.search_re.as_ref().is_some_and(|re| {
        re.is_match(&packet.summary.info)
            || re.is_match(&packet.summary.source)
            || re.is_match(&packet.summary.destination)
    });
    let zebra = if display_row.is_multiple_of(2) {
        theme::BG_PANEL
    } else {
        theme::BG_ZEBRA
    };
    let style = if selected {
        Style::default().bg(zebra)
    } else if let Some(threat) = &packet.threat {
        match threat.kind {
            ThreatKind::RareExternal => Style::default()
                .fg(theme::FG_ON_ACCENT)
                .bg(theme::COLOR_ANOMALY)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .fg(theme::FG_STRONG)
                .bg(theme::COLOR_CRIT)
                .add_modifier(Modifier::BOLD),
        }
    } else if matches_search {
        theme::STYLE_SEARCH_ROW
    } else if anomaly.is_some() {
        Style::default()
            .fg(theme::COLOR_ANOMALY)
            .bg(zebra)
            .add_modifier(Modifier::BOLD)
    } else if packet.retransmission {
        Style::default().fg(theme::COLOR_CRIT).bg(zebra)
    } else {
        let fg = match direction {
            Direction::In => theme::DIR_IN,
            Direction::Out => theme::DIR_OUT,
            Direction::Unknown => theme::proto_color(packet.summary.color_hint),
        };
        Style::default().fg(fg).bg(zebra)
    };

    let time = packet.timestamp.format("%H:%M:%S%.3f").to_string();
    let time_cell = if app.bookmarks.contains(&packet.number) {
        vec![
            Span::styled("*", Style::default().fg(theme::BOOKMARK)),
            Span::raw(time),
        ]
    } else {
        vec![Span::raw(time)]
    };
    let dir_text = match direction {
        Direction::In => "↓ IN",
        Direction::Out => "↑ OUT",
        Direction::Unknown => "·",
    };
    let process = packet
        .process
        .as_ref()
        .map_or("-".to_string(), |info| info.name.clone());
    let (remote_ip, remote_pt) = remote_endpoint(packet, &app.local_ips);
    let host = remote_ip
        .and_then(|ip| app.names.resolve(ip))
        .map(|entry| entry.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let address = match (remote_ip, remote_pt) {
        (Some(ip), Some(port)) => format!("{ip}:{port}"),
        (Some(ip), None) => ip.to_string(),
        _ => "-".to_string(),
    };
    let base_info = if matches!(packet.summary.color_hint, ColorHint::Tcp | ColorHint::Udp) {
        strip_leading_ports(&packet.summary.info)
    } else {
        &packet.summary.info
    };
    let info = match (&packet.threat, &anomaly) {
        (Some(threat), _) => format!(
            "! [{}] {} - {}",
            threat.kind.label(),
            base_info,
            threat.detail
        ),
        (None, Some(found)) => format!("! {} [{}]", base_info, found.detail),
        (None, None) => base_info.to_string(),
    };
    // The core writes `→` into some summaries; the list draws the in-budget form.
    let info = info.replace('→', "->");
    Some(TableRow {
        id: packet.number,
        cells: vec![
            time_cell,
            vec![Span::styled(
                dir_text,
                Style::default().add_modifier(Modifier::BOLD),
            )],
            vec![Span::raw(process)],
            vec![Span::raw(host)],
            vec![Span::raw(address)],
            vec![Span::raw(packet.summary.protocol.clone())],
            vec![Span::raw(packet.summary.length.to_string())],
            vec![Span::raw(info)],
        ],
        keys: Vec::new(),
        style,
    })
}

// Draws only the rows in view (virtual scrolling over the whole ring); the
// sort order and LIVE follow decide which rows those are.
pub fn render_packet_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.ui.lang;
    let total = app.visible_count();
    let visible = table_visible_rows(area);
    app.ui.clamp_view(CardId::PacketList, total, visible);
    if total > 0 {
        let max_scroll = total.saturating_sub(visible);
        let view = &mut app.ui.views[CardId::PacketList as usize];
        if app.follow {
            view.selected = total - 1;
            view.scroll = max_scroll;
        } else {
            let display = if app.ui.sorts[CardId::PacketList as usize].is_some() {
                app.order
                    .iter()
                    .position(|candidate| *candidate == app.selected)
                    .unwrap_or(app.selected)
            } else {
                app.selected
            };
            app.ui.views[CardId::PacketList as usize].selected = display.min(total - 1);
        }
    }
    let scroll = app.ui.views[CardId::PacketList as usize].scroll;
    let end = (scroll + visible).min(total);
    let rows: Vec<TableRow> = (scroll..end)
        .filter_map(|display_row| {
            let visible_idx = app.display_to_visible(display_row);
            packet_row(app, visible_idx, display_row)
        })
        .collect();
    let mut shown_columns = [false; crate::tui::cards::MAX_TABLE_COLUMNS];
    for (index, on) in app.col_visible.iter().enumerate() {
        shown_columns[index] = *on;
    }
    if shown_columns.iter().all(|on| !on) {
        shown_columns = [true; crate::tui::cards::MAX_TABLE_COLUMNS];
    }
    let hint = match app.dir_filter {
        Some(Direction::In) => i18n::text(lang, Msg::FilterInOnly),
        Some(Direction::Out) => i18n::text(lang, Msg::FilterOutOnly),
        _ => "",
    };
    let empty = if app.packets.is_empty() && app.active_filter.is_none() {
        if app.paused {
            empty_pair(lang, Msg::EmptyPaused, Msg::EmptyPausedHint)
        } else {
            empty_pair(lang, Msg::EmptyWaiting, Msg::EmptyWaitingHint)
        }
    } else {
        empty_pair(lang, Msg::EmptyNoMatch, Msg::EmptyNoMatchHint)
    };
    let table = CardTable {
        card: CardId::PacketList,
        title: i18n::title_count(i18n::text(lang, Msg::TitlePackets), total),
        hint: hint.to_string(),
        rows,
        total,
        offset: scroll,
        windowed: true,
        shown_columns,
        empty,
    };
    render_card_table(frame, &mut app.ui, area, table);
}

// 3. detail tree --------------------------------------------------------------
// Resets the detail and hex scroll when the selected packet changes.
fn sync_selected_packet(app: &mut App) {
    let number = app.selected_packet().map(|packet| packet.number);
    if number != app.last_selected_number {
        app.last_selected_number = number;
        app.ui.views[CardId::PacketDetail as usize].scroll = 0;
        app.ui.views[CardId::HexDump as usize].scroll = 0;
    }
}

pub fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    sync_selected_packet(app);
    let lang = app.ui.lang;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(packet) = app.selected_packet() {
        if packet.retransmission {
            lines.push(Line::from(Span::styled(
                format!("! {}", i18n::text(lang, Msg::PaneRetransmission)),
                Style::default()
                    .fg(theme::COLOR_CRIT)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        for layer in &packet.layers {
            lines.extend(detail_lines(layer));
        }
    }
    let spec = TextCard {
        card: CardId::PacketDetail,
        title: i18n::text(lang, Msg::TitleDetails).to_string(),
        hint: String::new(),
        lines,
        empty: Some((
            i18n::text(lang, Msg::EmptyNoSelection).to_string(),
            String::new(),
        )),
    };
    render_text_card(frame, &mut app.ui, area, spec);
}

// Rows of one layer: (is_header, text). Field labels are protocol vocabulary.
fn layer_rows(layer: &Layer) -> Vec<(bool, String)> {
    let mut rows: Vec<(bool, String)> = Vec::new();
    match layer {
        Layer::Ethernet(eth) => {
            rows.push((
                true,
                format!("> Ethernet II, Src: {}, Dst: {}", eth.src_mac, eth.dst_mac),
            ));
            rows.push((false, format!("    Source: {}", eth.src_mac)));
            rows.push((false, format!("    Destination: {}", eth.dst_mac)));
            rows.push((false, format!("    Type: 0x{:04x}", eth.ethertype)));
        }
        Layer::Arp(arp) => {
            let operation = match arp.operation {
                1 => "Request",
                2 => "Reply",
                _ => "Unknown",
            };
            rows.push((true, format!("> ARP ({operation})")));
            rows.push((false, format!("    Sender MAC: {}", arp.sender_mac)));
            rows.push((false, format!("    Sender IP: {}", arp.sender_ip)));
            rows.push((false, format!("    Target MAC: {}", arp.target_mac)));
            rows.push((false, format!("    Target IP: {}", arp.target_ip)));
        }
        Layer::Ipv4(ip) => {
            rows.push((
                true,
                format!(
                    "> Internet Protocol Version 4, Src: {}, Dst: {}",
                    ip.src_ip, ip.dst_ip
                ),
            ));
            rows.push((false, format!("    Version: {}", ip.version)));
            rows.push((
                false,
                format!("    Header Length: {} bytes ({})", ip.ihl * 4, ip.ihl),
            ));
            rows.push((false, format!("    Total Length: {}", ip.total_length)));
            rows.push((false, format!("    TTL: {}", ip.ttl)));
            rows.push((
                false,
                format!(
                    "    Protocol: {} ({})",
                    protocol_name(ip.protocol),
                    ip.protocol
                ),
            ));
            rows.push((false, format!("    Source: {}", ip.src_ip)));
            rows.push((false, format!("    Destination: {}", ip.dst_ip)));
        }
        Layer::Ipv6(ip) => {
            rows.push((
                true,
                format!(
                    "> Internet Protocol Version 6, Src: {}, Dst: {}",
                    ip.src_ip, ip.dst_ip
                ),
            ));
            rows.push((
                false,
                format!(
                    "    Next Header: {} ({})",
                    protocol_name(ip.next_header),
                    ip.next_header
                ),
            ));
            rows.push((false, format!("    Hop Limit: {}", ip.hop_limit)));
            rows.push((false, format!("    Payload Length: {}", ip.payload_length)));
        }
        Layer::Tcp(tcp) => {
            rows.push((
                true,
                format!(
                    "> TCP, Src Port: {}, Dst Port: {}",
                    tcp.src_port, tcp.dst_port
                ),
            ));
            rows.push((false, format!("    Source Port: {}", tcp.src_port)));
            rows.push((false, format!("    Destination Port: {}", tcp.dst_port)));
            rows.push((false, format!("    Sequence Number: {}", tcp.seq_num)));
            rows.push((false, format!("    Acknowledgment Number: {}", tcp.ack_num)));
            rows.push((false, format!("    Flags: {}", tcp.flags.display())));
            rows.push((false, format!("    Window Size: {}", tcp.window_size)));
            rows.push((false, format!("    Payload Length: {}", tcp.payload_len)));
        }
        Layer::Udp(udp) => {
            rows.push((
                true,
                format!(
                    "> UDP, Src Port: {}, Dst Port: {}",
                    udp.src_port, udp.dst_port
                ),
            ));
            rows.push((false, format!("    Length: {}", udp.length)));
        }
        Layer::Icmp(icmp) => {
            rows.push((
                true,
                format!("> ICMP, Type: {}, Code: {}", icmp.icmp_type, icmp.code),
            ));
        }
        Layer::Icmpv6(icmp) => {
            rows.push((
                true,
                format!("> ICMPv6, Type: {}, Code: {}", icmp.icmp_type, icmp.code),
            ));
        }
        Layer::Dns(dns) => {
            let kind = if dns.is_response { "Response" } else { "Query" };
            rows.push((true, format!("> DNS {kind} (0x{:04x})", dns.transaction_id)));
            for question in &dns.questions {
                rows.push((
                    false,
                    format!(
                        "    Question: {} (type={})",
                        question.qname,
                        dns_type_name(question.qtype)
                    ),
                ));
            }
            for answer in &dns.answers {
                let data = match &answer.rdata {
                    DnsRdata::A(ip) => format!("A {ip}"),
                    DnsRdata::Aaaa(ip) => format!("AAAA {ip}"),
                    DnsRdata::Cname(name) => format!("CNAME {name}"),
                    DnsRdata::Ns(name) => format!("NS {name}"),
                    DnsRdata::Ptr(name) => format!("PTR {name}"),
                    DnsRdata::Mx {
                        preference,
                        exchange,
                    } => format!("MX {preference} {exchange}"),
                    DnsRdata::Txt(parts) => format!("TXT {}", parts.join(" ")),
                    DnsRdata::Srv { target, port, .. } => format!("SRV {target}:{port}"),
                    DnsRdata::Soa { mname, .. } => format!("SOA {mname}"),
                    DnsRdata::Unknown { rtype, .. } => format!("type{rtype}"),
                };
                rows.push((false, format!("    Answer: {} -> {}", answer.name, data)));
            }
        }
        Layer::TlsClientHello(tls) => {
            rows.push((true, "> TLS Client Hello".to_string()));
            if let Some(sni) = &tls.sni {
                rows.push((false, format!("    Server Name: {sni}")));
            }
            if !tls.alpn.is_empty() {
                rows.push((false, format!("    ALPN: {}", tls.alpn.join(", "))));
            }
            if let Some(ja3) = &tls.ja3 {
                rows.push((false, format!("    JA3: {ja3}")));
            }
            if let Some(ja4) = &tls.ja4 {
                rows.push((false, format!("    JA4: {ja4}")));
            }
        }
        Layer::TlsHandshake(handshake) => {
            rows.push((true, "> TLS Handshake".to_string()));
            for message in &handshake.messages {
                let text = match message {
                    TlsHandshakeMessage::ServerHello {
                        version,
                        cipher_suite,
                        alpn,
                    } => format!(
                        "    Server Hello (version=0x{version:04x}, cipher=0x{cipher_suite:04x}, alpn={})",
                        alpn.join(",")
                    ),
                    TlsHandshakeMessage::Certificate { cert_count } => {
                        format!("    Certificate ({cert_count} certs)")
                    }
                    TlsHandshakeMessage::Other { msg_type } => {
                        format!("    Handshake type {msg_type}")
                    }
                };
                rows.push((false, text));
            }
        }
        Layer::Http(http) => {
            let title = if http.is_request {
                format!(
                    "> HTTP {} {}",
                    http.method.as_deref().unwrap_or("?"),
                    http.uri.as_deref().unwrap_or("")
                )
            } else {
                format!(
                    "> HTTP {} response",
                    http.status_code
                        .map(|code| code.to_string())
                        .unwrap_or_default()
                )
            };
            rows.push((true, title));
            if let Some(host) = &http.host {
                rows.push((false, format!("    Host: {host}")));
            }
            for (key, value) in &http.query_params {
                rows.push((false, format!("    Query · {key}: {value}")));
            }
            if !http.body_params.is_empty() {
                rows.push((true, "    Body Parameters:".to_string()));
                for (key, value) in &http.body_params {
                    rows.push((false, format!("        {key}: {value}")));
                }
            }
        }
        Layer::Http2(http2) => {
            rows.push((true, format!("> HTTP/2 ({} frames)", http2.frames.len())));
            for frame in &http2.frames {
                for (key, value) in &frame.headers {
                    rows.push((false, format!("    {key}: {value}")));
                }
            }
        }
        Layer::Quic(quic) => {
            rows.push((true, format!("> QUIC {:?}", quic.packet_type)));
            if let Some(version) = quic.version {
                rows.push((false, format!("    Version: 0x{version:08x}")));
            }
        }
        Layer::Payload { offset, len } => {
            rows.push((false, format!("> Data ({len} bytes at offset {offset})")));
        }
    }
    rows
}

fn detail_lines(layer: &Layer) -> Vec<Line<'static>> {
    let header_style = Style::default()
        .fg(theme::DETAIL_HEADER)
        .add_modifier(Modifier::BOLD);
    layer_rows(layer)
        .into_iter()
        .map(|(is_header, text)| {
            Line::from(Span::styled(
                text,
                if is_header {
                    header_style
                } else {
                    theme::STYLE_DIM
                },
            ))
        })
        .collect()
}

fn protocol_name(proto: u8) -> &'static str {
    match proto {
        1 => "ICMP",
        6 => "TCP",
        17 => "UDP",
        58 => "ICMPv6",
        _ => "Unknown",
    }
}

fn dns_type_name(qtype: u16) -> &'static str {
    match qtype {
        1 => "A",
        2 => "NS",
        5 => "CNAME",
        6 => "SOA",
        15 => "MX",
        16 => "TXT",
        28 => "AAAA",
        255 => "ANY",
        _ => "?",
    }
}

// 4. hex dump -----------------------------------------------------------------
pub fn render_hex(frame: &mut Frame, app: &mut App, area: Rect) {
    sync_selected_packet(app);
    let lang = app.ui.lang;
    let lines = app
        .selected_packet()
        .map(|packet| format_hex_dump(&packet.data))
        .unwrap_or_default();
    let spec = TextCard {
        card: CardId::HexDump,
        title: i18n::text(lang, Msg::TitleHex).to_string(),
        hint: String::new(),
        lines,
        empty: Some((
            i18n::text(lang, Msg::EmptyNoSelection).to_string(),
            String::new(),
        )),
    };
    render_text_card(frame, &mut app.ui, area, spec);
}

// 16 bytes per line: offset, hex with a gap after eight, then ASCII.
pub fn format_hex_dump(data: &[u8]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, chunk) in data.chunks(16).enumerate() {
        let mut parts = vec![Span::styled(
            format!("{:04x}   ", index * 16),
            theme::STYLE_DIM,
        )];
        let mut hex = String::new();
        for (position, byte) in chunk.iter().enumerate() {
            hex.push_str(&format!("{byte:02x} "));
            if position == 7 {
                hex.push(' ');
            }
        }
        let padding = if chunk.len() <= 8 {
            (16 - chunk.len()) * 3 + 1
        } else {
            (16 - chunk.len()) * 3
        };
        hex.push_str(&" ".repeat(padding));
        parts.push(Span::styled(hex, Style::default().fg(theme::ACCENT_TCP)));
        parts.push(Span::raw("  "));
        let ascii: String = chunk
            .iter()
            .map(|&byte| {
                if (0x20..=0x7E).contains(&byte) {
                    byte as char
                } else {
                    '.'
                }
            })
            .collect();
        parts.push(Span::styled(ascii, Style::default().fg(theme::ACCENT_UDP)));
        lines.push(Line::from(parts));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tcp_udp_port_prefix() {
        assert_eq!(
            strip_leading_ports("443 → 50000 [SYN] Seq=0 Len=0"),
            "[SYN] Seq=0 Len=0"
        );
        assert_eq!(strip_leading_ports("12345 → 53 Len=40"), "Len=40");
    }

    #[test]
    fn leaves_info_without_port_prefix() {
        assert_eq!(
            strip_leading_ports("DNS Q example.com"),
            "DNS Q example.com"
        );
        assert_eq!(strip_leading_ports("Echo Request"), "Echo Request");
    }

    #[test]
    fn hex_dump_pads_partial_lines_and_masks_non_ascii() {
        let lines = format_hex_dump(b"ABCDEFGHIJKLMNOPQ\x01");
        assert_eq!(lines.len(), 2);
        let first: String = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(first.starts_with("0000   41 42 43"), "{first}");
        assert!(first.ends_with("ABCDEFGHIJKLMNOP"), "{first}");
        let second: String = lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(second.starts_with("0010   51 01"), "{second}");
        assert!(second.ends_with("Q."), "{second}");
    }
}
