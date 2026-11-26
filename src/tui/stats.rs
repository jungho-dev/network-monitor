//! Statistics computed from the packet ring: top talkers, protocol shares,
//! flows, bookmarks, and TCP stream reassembly. Built into a cache on demand
//! so the render path never walks the whole ring.

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;

use chrono::{DateTime, Utc};

use network_monitor_core::analysis::ThreatKind;
use network_monitor_core::decode::{DecodedPacket, Layer};
use network_monitor_core::flow::{ReassemblyBuffer, StreamData};

use super::level::{OutputLevel, packet_level};

// 1. cached rows --------------------------------------------------------------
pub struct TalkerRow {
    pub endpoint: String,
    pub packets: u64,
    pub bytes: u64,
}

pub struct ProtocolRow {
    pub name: String,
    pub count: u64,
}

pub struct FlowRow {
    pub label: String,
    pub proto: &'static str,
    pub packets: u64,
    pub bytes: u64,
    pub first_ms: i64,
    pub last_ms: i64,
}

pub struct BookmarkRow {
    pub number: u64,
    pub time_ms: i64,
    pub endpoints: String,
    pub info: String,
}

pub struct ThreatRow {
    pub time: DateTime<Utc>,
    pub kind: ThreatKind,
    pub number: u64,
    pub detail: String,
}

#[derive(Default)]
pub struct StatsCache {
    pub talkers: Vec<TalkerRow>,
    pub protocols: Vec<ProtocolRow>,
    pub flows: Vec<FlowRow>,
    pub bookmarks: Vec<BookmarkRow>,
    pub threats: Vec<ThreatRow>,
}

// 2. aggregation --------------------------------------------------------------
pub type FiveTuple = (IpAddr, u16, IpAddr, u16, u8, &'static str);

// (src ip, src port, dst ip, dst port, ip protocol, label) of a TCP or UDP packet.
pub fn five_tuple(packet: &DecodedPacket) -> Option<FiveTuple> {
    let mut src = None;
    let mut dst = None;
    let mut proto = 0u8;
    for layer in &packet.layers {
        match layer {
            Layer::Ipv4(ip) => {
                src = Some(IpAddr::V4(ip.src_ip));
                dst = Some(IpAddr::V4(ip.dst_ip));
                proto = ip.protocol;
            }
            Layer::Ipv6(ip) => {
                src = Some(IpAddr::V6(ip.src_ip));
                dst = Some(IpAddr::V6(ip.dst_ip));
                proto = ip.next_header;
            }
            Layer::Tcp(tcp) => {
                return Some((src?, tcp.src_port, dst?, tcp.dst_port, proto, "TCP"));
            }
            Layer::Udp(udp) => {
                return Some((src?, udp.src_port, dst?, udp.dst_port, proto, "UDP"));
            }
            _ => {}
        }
    }
    None
}

// One pass over the packets: talkers, flows, bookmarked rows, and threat rows
// at or above `min_level`; protocol counts come from the running totals the
// app keeps.
pub fn build_stats<'a>(
    packets: impl Iterator<Item = &'a DecodedPacket>,
    bookmarks: &HashSet<u64>,
    proto_counts: &BTreeMap<String, u64>,
    min_level: OutputLevel,
) -> StatsCache {
    let mut talkers: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut flows: BTreeMap<String, FlowRow> = BTreeMap::new();
    let mut marked: Vec<BookmarkRow> = Vec::new();
    let mut threats: Vec<ThreatRow> = Vec::new();
    for packet in packets {
        if let Some(threat) = &packet.threat
            && packet_level(packet) >= min_level
        {
            threats.push(ThreatRow {
                time: packet.timestamp,
                kind: threat.kind,
                number: packet.number,
                detail: threat.detail.clone(),
            });
        }
        for endpoint in [&packet.summary.source, &packet.summary.destination] {
            if endpoint.is_empty() {
                continue;
            }
            let entry = talkers.entry(endpoint.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += u64::from(packet.wire_len);
        }
        if bookmarks.contains(&packet.number) {
            marked.push(BookmarkRow {
                number: packet.number,
                time_ms: packet.timestamp.timestamp_millis(),
                endpoints: format!(
                    "{} -> {}",
                    packet.summary.source, packet.summary.destination
                ),
                info: packet.summary.info.replace('→', "->"),
            });
        }
        let Some((src, src_port, dst, dst_port, _proto, label)) = five_tuple(packet) else {
            continue;
        };
        // Both directions share one key.
        let (first, second) = if (src, src_port) <= (dst, dst_port) {
            (format!("{src}:{src_port}"), format!("{dst}:{dst_port}"))
        } else {
            (format!("{dst}:{dst_port}"), format!("{src}:{src_port}"))
        };
        let timestamp = packet.timestamp.timestamp_millis();
        let flow = flows
            .entry(format!("{first} <-> {second} {label}"))
            .or_insert_with(|| FlowRow {
                label: format!("{first} <-> {second}"),
                proto: label,
                packets: 0,
                bytes: 0,
                first_ms: timestamp,
                last_ms: timestamp,
            });
        flow.packets += 1;
        flow.bytes += u64::from(packet.wire_len);
        flow.first_ms = flow.first_ms.min(timestamp);
        flow.last_ms = flow.last_ms.max(timestamp);
    }
    StatsCache {
        talkers: talkers
            .into_iter()
            .map(|(endpoint, (packets, bytes))| TalkerRow {
                endpoint,
                packets,
                bytes,
            })
            .collect(),
        protocols: proto_counts
            .iter()
            .map(|(name, count)| ProtocolRow {
                name: name.clone(),
                count: *count,
            })
            .collect(),
        flows: flows.into_values().collect(),
        bookmarks: marked,
        threats,
    }
}

// 3. stream reassembly --------------------------------------------------------
// The reassembled flow with its two endpoints (client = lower ip and port).
pub struct StreamView {
    pub client: (IpAddr, u16),
    pub server: (IpAddr, u16),
    pub data: StreamData,
}

// Reassembles the selected packet's TCP flow in both directions.
pub fn build_stream<'a>(
    selected: &DecodedPacket,
    packets: impl Iterator<Item = &'a DecodedPacket>,
) -> Option<StreamView> {
    let (src, src_port, dst, dst_port, proto, _) = five_tuple(selected)?;
    if proto != 6 {
        return None;
    }
    let (client, server) = if (src, src_port) <= (dst, dst_port) {
        ((src, src_port), (dst, dst_port))
    } else {
        ((dst, dst_port), (src, src_port))
    };
    let mut client_to_server = ReassemblyBuffer::new();
    let mut server_to_client = ReassemblyBuffer::new();
    for packet in packets {
        let Some((psrc, psrc_port, pdst, pdst_port, pproto, _)) = five_tuple(packet) else {
            continue;
        };
        if pproto != 6 {
            continue;
        }
        let same = ((psrc, psrc_port), (pdst, pdst_port)) == (client, server)
            || ((pdst, pdst_port), (psrc, psrc_port)) == (client, server);
        if !same {
            continue;
        }
        let Some((seq, payload)) = tcp_payload(packet) else {
            continue;
        };
        if payload.is_empty() {
            continue;
        }
        if (psrc, psrc_port) == client {
            client_to_server.insert_segment(seq, payload);
        } else {
            server_to_client.insert_segment(seq, payload);
        }
    }
    let data = StreamData {
        client_to_server: client_to_server.try_drain().unwrap_or_default(),
        server_to_client: server_to_client.try_drain().unwrap_or_default(),
    };
    Some(StreamView {
        client,
        server,
        data,
    })
}

fn tcp_payload(packet: &DecodedPacket) -> Option<(u32, &[u8])> {
    let mut seq = None;
    for layer in &packet.layers {
        match layer {
            Layer::Tcp(tcp) => seq = Some(tcp.seq_num),
            Layer::Payload { offset, len } => {
                let start = seq?;
                let end = (offset + len).min(packet.data.len());
                return Some((start, packet.data.get(*offset..end)?));
            }
            _ => {}
        }
    }
    None
}

// Bytes shown as text: printable ASCII, newline, tab; CR as space; the rest `.`.
pub fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if byte == b'\n' || byte == b'\t' || (0x20u8..0x7fu8).contains(&byte) {
                char::from_u32(u32::from(byte)).unwrap_or('.')
            } else if byte == b'\r' {
                ' '
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::tcp_packet;

    #[test]
    fn talkers_flows_and_bookmarks_aggregate_both_directions_once() {
        let forward = tcp_packet(1, [10, 0, 0, 1], 40000, [93, 184, 216, 34], 443, 100, b"");
        let reverse = tcp_packet(2, [93, 184, 216, 34], 443, [10, 0, 0, 1], 40000, 60, b"");
        let mut marks = HashSet::new();
        marks.insert(2);
        let stats = build_stats(
            [&forward, &reverse].into_iter(),
            &marks,
            &BTreeMap::new(),
            OutputLevel::Debug,
        );
        assert_eq!(stats.flows.len(), 1);
        assert_eq!(stats.flows[0].packets, 2);
        assert_eq!(stats.flows[0].bytes, 160);
        assert_eq!(stats.flows[0].proto, "TCP");
        let talker = stats
            .talkers
            .iter()
            .find(|row| row.endpoint == "10.0.0.1")
            .expect("talker");
        assert_eq!((talker.packets, talker.bytes), (2, 160));
        assert_eq!(stats.bookmarks.len(), 1);
        assert_eq!(stats.bookmarks[0].number, 2);
    }

    #[test]
    fn stream_reassembly_splits_client_and_server_payloads() {
        let request = tcp_packet(1, [10, 0, 0, 1], 40000, [10, 0, 0, 2], 9000, 80, b"GET /");
        let response = tcp_packet(2, [10, 0, 0, 2], 9000, [10, 0, 0, 1], 40000, 80, b"200 OK");
        let stream = build_stream(&request, [&request, &response].into_iter())
            .expect("a TCP packet builds a stream");
        assert_eq!(stream.client.1, 40000);
        assert_eq!(stream.server.1, 9000);
        assert_eq!(printable(&stream.data.client_to_server), "GET /");
        assert_eq!(printable(&stream.data.server_to_client), "200 OK");
    }

    #[test]
    fn non_tcp_packets_build_no_stream_and_printable_masks_binary() {
        let plain = crate::tui::test_support::blank_packet(1);
        assert!(build_stream(&plain, std::iter::once(&plain)).is_none());
        assert_eq!(printable(b"a\r\n\x01b"), "a \n.b");
    }
}
