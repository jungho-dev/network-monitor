//! Stateful live threat heuristics over the decoded packet stream.
//!
//! Unlike the stateless [`super::anomaly`] checks, this detector keeps a short
//! sliding history per source address so reconnaissance patterns — port scans
//! and host sweeps ("someone is scanning me") — become visible across packets.
//! It also flags a connection attempt or cleartext data to an uncommon port on
//! a public host, once per destination within [`RARE_REPEAT_MS`]. Traffic into
//! a local address, replies from a common service port, empty segments, and
//! encrypted payloads (TLS or QUIC layer, or a TLS record header) never count.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::decode::{DecodedPacket, Layer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatKind {
    /// One source sending connection attempts to many distinct ports on a host.
    PortScan,
    /// One source touching many distinct hosts in a short window.
    HostSweep,
    /// A connection attempt or cleartext data to an uncommon port on a public
    /// (non-private) host.
    RareExternal,
}

impl ThreatKind {
    /// Short tag shown in the packet list / log and matched by the filter.
    pub fn label(self) -> &'static str {
        match self {
            ThreatKind::PortScan => "PORT-SCAN",
            ThreatKind::HostSweep => "HOST-SWEEP",
            ThreatKind::RareExternal => "RARE-DEST",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreatAnnotation {
    pub kind: ThreatKind,
    pub detail: String,
}

/// Sliding window for the recon detectors.
const SCAN_WINDOW_MS: i64 = 10_000;
/// Distinct dst ports from one source to one host that trip a port-scan.
const PORT_SCAN_MIN_PORTS: usize = 15;
/// Distinct dst hosts from one source that trip a host-sweep.
const HOST_SWEEP_MIN_HOSTS: usize = 12;
/// A rare-destination tag repeats for the same (src, dst, dst port) at most
/// once per this interval, so one session does not tag every packet.
pub const RARE_REPEAT_MS: i64 = 60_000;
/// Lowest port treated as a client's ephemeral port (IANA and Linux ranges
/// start at 49152 and 32768): a reply from a common service port goes there.
const EPHEMERAL_PORT_MIN: u16 = 32_768;
/// Remembered rare destinations before expired entries are pruned.
const RARE_SEEN_MAX: usize = 4096;

/// Common service ports that never count as a "rare" external destination.
const COMMON_PORTS: &[u16] = &[
    20, 21, 22, 25, 53, 80, 110, 123, 143, 443, 465, 587, 853, 993, 995, 3389, 8080, 8443,
];

/// Per-source ring of recent connection-initiation events.
#[derive(Default)]
struct SrcWindow {
    /// Kept within the window: (ts_ms, dst, dst_port).
    events: VecDeque<(i64, IpAddr, u16)>,
}

impl SrcWindow {
    fn record(&mut self, ts: i64, dst: IpAddr, port: u16, cutoff: i64) {
        self.events.push_back((ts, dst, port));
        while let Some(&(t, _, _)) = self.events.front() {
            if t < cutoff {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// `(dst, distinct_port_count)` for the host this source probed most.
    fn busiest_dst(&self) -> Option<(IpAddr, usize)> {
        let mut per_dst: HashMap<IpAddr, HashSet<u16>> = HashMap::new();
        for &(_, dst, port) in &self.events {
            per_dst.entry(dst).or_default().insert(port);
        }
        per_dst
            .into_iter()
            .map(|(dst, ports)| (dst, ports.len()))
            .max_by_key(|&(_, n)| n)
    }

    fn distinct_hosts(&self) -> usize {
        self.events
            .iter()
            .map(|&(_, dst, _)| dst)
            .collect::<HashSet<_>>()
            .len()
    }
}

/// Tracks per-source recon activity across the live packet stream. One instance
/// lives for the whole capture session and is fed every packet in arrival order.
#[derive(Default)]
pub struct ThreatTracker {
    sources: HashMap<IpAddr, SrcWindow>,
    /// Addresses of this host: traffic into them is never a rare destination.
    local: HashSet<IpAddr>,
    /// Last tag time (ms) per (src, dst, dst port) rare destination.
    rare_seen: HashMap<(IpAddr, IpAddr, u16), i64>,
}

impl ThreatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The host's own addresses (empty for a file or an unknown host, which
    /// leaves only the server-reply rule to tell the direction).
    pub fn set_local_ips(&mut self, ips: &[IpAddr]) {
        self.local = ips.iter().copied().collect();
    }

    /// Inspect one packet and return a threat annotation if it participates in a
    /// recon pattern or contacts a rare external destination. Recon detectors
    /// only advance on connection-initiation signals (pure SYN / ICMP echo) so
    /// established conversations do not look like scans.
    pub fn observe(&mut self, pkt: &DecodedPacket) -> Option<ThreatAnnotation> {
        let conn = extract_conn(pkt)?;
        let now = pkt.timestamp.timestamp_millis();

        if conn.syn_only || conn.icmp_echo {
            let win = self.sources.entry(conn.src).or_default();
            win.record(now, conn.dst, conn.dport, now - SCAN_WINDOW_MS);

            if let Some((dst, ports)) = win.busiest_dst()
                && ports >= PORT_SCAN_MIN_PORTS
            {
                return Some(ThreatAnnotation {
                    kind: ThreatKind::PortScan,
                    detail: format!(
                        "{} probed {ports} ports on {dst} within {}s",
                        conn.src,
                        SCAN_WINDOW_MS / 1000
                    ),
                });
            }
            let hosts = win.distinct_hosts();
            if hosts >= HOST_SWEEP_MIN_HOSTS {
                return Some(ThreatAnnotation {
                    kind: ThreatKind::HostSweep,
                    detail: format!(
                        "{} contacted {hosts} hosts within {}s",
                        conn.src,
                        SCAN_WINDOW_MS / 1000
                    ),
                });
            }
        }

        // rare destination: a connection attempt (pure SYN) or cleartext data
        // to an uncommon port on a public host, once per window. Never traffic
        // into a local address, a server reply (common source port, no SYN,
        // ephemeral destination port; a spoofed source port alone still tags),
        // an empty ACK/FIN/RST, or an encrypted payload (TLS or QUIC layer, or
        // a payload that opens with a TLS record header).
        let encrypted =
            has_encrypted_layer(pkt) || matches!(conn.payload, [20..=23, 0x03, 0..=0x04, _, _, ..]);
        let cleartext_data = !conn.payload.is_empty() && !encrypted;
        let server_reply = COMMON_PORTS.contains(&conn.sport)
            && !conn.syn_only
            && conn.dport >= EPHEMERAL_PORT_MIN;
        if !(conn.syn_only || cleartext_data)
            || conn.dport == 0
            || self.local.contains(&conn.dst)
            || !is_public(conn.dst)
            || COMMON_PORTS.contains(&conn.dport)
            || server_reply
        {
            return None;
        }
        let key = (conn.src, conn.dst, conn.dport);
        if self
            .rare_seen
            .get(&key)
            .is_some_and(|&last| now - last < RARE_REPEAT_MS)
        {
            return None;
        }
        if self.rare_seen.len() >= RARE_SEEN_MAX {
            self.rare_seen
                .retain(|_, last| now - *last < RARE_REPEAT_MS);
        }
        if self.rare_seen.len() >= RARE_SEEN_MAX {
            self.rare_seen.clear();
        }
        self.rare_seen.insert(key, now);
        let what = if conn.syn_only {
            "connection attempt"
        } else {
            "cleartext"
        };
        Some(ThreatAnnotation {
            kind: ThreatKind::RareExternal,
            detail: format!("{what} to {}:{} (uncommon port)", conn.dst, conn.dport),
        })
    }
}

struct Conn<'a> {
    src: IpAddr,
    dst: IpAddr,
    sport: u16,
    dport: u16,
    syn_only: bool,
    icmp_echo: bool,
    /// Transport payload bytes captured (may be cut short by the snap length).
    payload: &'a [u8],
}

fn extract_conn(pkt: &DecodedPacket) -> Option<Conn<'_>> {
    let mut src = None;
    let mut dst = None;
    let mut sport = 0u16;
    let mut dport = 0u16;
    let mut syn_only = false;
    let mut icmp_echo = false;
    // (start, length) of the transport payload inside `pkt.data`
    let mut payload_span = (0usize, 0usize);
    for l in &pkt.layers {
        match l {
            Layer::Ipv4(ip) => {
                src = Some(IpAddr::V4(ip.src_ip));
                dst = Some(IpAddr::V4(ip.dst_ip));
            }
            Layer::Ipv6(ip) => {
                src = Some(IpAddr::V6(ip.src_ip));
                dst = Some(IpAddr::V6(ip.dst_ip));
            }
            Layer::Tcp(t) => {
                sport = t.src_port;
                dport = t.dst_port;
                syn_only = t.flags.syn && !t.flags.ack;
                payload_span = (t.header_range.1, t.payload_len);
            }
            Layer::Udp(u) => {
                sport = u.src_port;
                dport = u.dst_port;
                payload_span = (u.header_range.1, usize::from(u.length).saturating_sub(8));
            }
            Layer::Icmp(ic) => icmp_echo = ic.icmp_type == 8,
            Layer::Icmpv6(ic) => icmp_echo = ic.icmp_type == 128,
            _ => {}
        }
    }
    let (start, len) = payload_span;
    let end = start.saturating_add(len).min(pkt.data.len());
    Some(Conn {
        src: src?,
        dst: dst?,
        sport,
        dport,
        syn_only,
        icmp_echo,
        payload: pkt.data.get(start..end).unwrap_or_default(),
    })
}

/// True when the decoder found an encrypted application layer.
fn has_encrypted_layer(pkt: &DecodedPacket) -> bool {
    pkt.layers.iter().any(|l| {
        matches!(
            l,
            Layer::TlsClientHello(_) | Layer::TlsHandshake(_) | Layer::Quic(_)
        )
    })
}

/// Conservative "routable public address" test (no unstable `is_global`).
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64))
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::*;
    use chrono::DateTime;
    use std::net::Ipv4Addr;

    const ACK: u8 = 0x10;
    const SYN: u8 = 0x02;
    const HTTP: &[u8] = b"GET / HTTP/1.1\r\n";

    fn syn(src: Ipv4Addr, dst: Ipv4Addr, dport: u16, ts_ms: i64) -> DecodedPacket {
        tcp(src, 40000, dst, dport, SYN, &[], ts_ms)
    }

    // IPv4 + TCP headers (40 zero bytes) followed by `payload`.
    fn tcp(
        src: Ipv4Addr,
        sport: u16,
        dst: Ipv4Addr,
        dport: u16,
        flags: u8,
        payload: &[u8],
        ts_ms: i64,
    ) -> DecodedPacket {
        let mut data = vec![0u8; 40];
        data.extend_from_slice(payload);
        DecodedPacket {
            number: 0,
            timestamp: DateTime::from_timestamp_millis(ts_ms).unwrap(),
            wire_len: 60,
            data,
            layers: vec![
                Layer::Ipv4(Ipv4Header {
                    version: 4,
                    ihl: 5,
                    dscp: 0,
                    ecn: 0,
                    total_length: 40,
                    identification: 0,
                    flags: 0,
                    fragment_offset: 0,
                    ttl: 64,
                    protocol: 6,
                    checksum: 0,
                    src_ip: src,
                    dst_ip: dst,
                    header_range: (0, 20),
                }),
                Layer::Tcp(TcpHeader {
                    src_port: sport,
                    dst_port: dport,
                    seq_num: 0,
                    ack_num: 0,
                    data_offset: 5,
                    flags: TcpFlags::from_bits(flags),
                    window_size: 1024,
                    checksum: 0,
                    urgent_pointer: 0,
                    payload_len: payload.len(),
                    header_range: (20, 40),
                }),
            ],
            summary: PacketSummary {
                source: src.to_string(),
                destination: dst.to_string(),
                protocol: "TCP".into(),
                length: 60,
                info: String::new(),
                color_hint: ColorHint::Tcp,
            },
            process: None,
            retransmission: false,
            threat: None,
        }
    }

    #[test]
    fn detects_port_scan() {
        let mut t = ThreatTracker::new();
        let attacker = Ipv4Addr::new(192, 168, 1, 66);
        let victim = Ipv4Addr::new(192, 168, 1, 10);
        let mut hit = None;
        for port in 1..=PORT_SCAN_MIN_PORTS as u16 {
            hit = t.observe(&syn(attacker, victim, port, 1_000 + port as i64));
        }
        assert!(matches!(
            hit,
            Some(ThreatAnnotation {
                kind: ThreatKind::PortScan,
                ..
            })
        ));
    }

    #[test]
    fn normal_handshake_is_quiet() {
        let mut t = ThreatTracker::new();
        let me = Ipv4Addr::new(192, 168, 1, 5);
        let server = Ipv4Addr::new(192, 168, 1, 1);
        assert!(t.observe(&syn(me, server, 80, 1_000)).is_none());
        assert!(t.observe(&syn(me, server, 443, 1_100)).is_none());
    }

    #[test]
    fn rare_dest_tags_once_per_destination_within_the_window_and_again_after_it() {
        let mut t = ThreatTracker::new();
        let me = Ipv4Addr::new(10, 0, 0, 5);
        let far = Ipv4Addr::new(8, 8, 4, 4);
        let first = t.observe(&tcp(me, 50000, far, 9999, ACK, HTTP, 1_000));
        assert!(matches!(
            first,
            Some(ThreatAnnotation {
                kind: ThreatKind::RareExternal,
                ..
            })
        ));
        for step in 1..50 {
            let ts = 1_000 + step * 100;
            assert!(
                t.observe(&tcp(me, 50000, far, 9999, ACK, HTTP, ts))
                    .is_none()
            );
        }
        let other_port = t.observe(&tcp(me, 50000, far, 7777, ACK, HTTP, 6_000));
        assert!(
            other_port.is_some(),
            "a different port is a new destination"
        );
        let later = t.observe(&tcp(
            me,
            50000,
            far,
            9999,
            ACK,
            HTTP,
            1_000 + RARE_REPEAT_MS,
        ));
        assert!(later.is_some(), "the tag repeats after the window");
    }

    #[test]
    fn rare_dest_ignores_replies_from_a_common_service_port() {
        // The screenshot case: an HTTPS server answering a public client
        // address on its ephemeral port.
        let mut t = ThreatTracker::new();
        let server = Ipv4Addr::new(160, 79, 104, 10);
        let client = Ipv4Addr::new(61, 105, 97, 216);
        for step in 0..20 {
            let reply = tcp(server, 443, client, 57872, ACK, HTTP, 1_000 + step);
            assert!(t.observe(&reply).is_none());
        }
    }

    #[test]
    fn rare_dest_still_tags_a_common_source_port_used_as_a_disguise() {
        let mut t = ThreatTracker::new();
        let me = Ipv4Addr::new(10, 0, 0, 5);
        let far = Ipv4Addr::new(8, 8, 4, 4);
        let syn_from_53 = t.observe(&tcp(me, 53, far, 50000, SYN, &[], 1_000));
        assert!(syn_from_53.is_some(), "a SYN is never a server reply");
        assert!(
            syn_from_53
                .unwrap()
                .detail
                .starts_with("connection attempt")
        );
        let data_from_443 = t.observe(&tcp(me, 443, far, 4444, ACK, HTTP, 2_000));
        assert!(data_from_443.is_some(), "4444 is not an ephemeral port");
        assert!(data_from_443.unwrap().detail.starts_with("cleartext"));
    }

    #[test]
    fn rare_dest_ignores_traffic_into_a_local_address_on_any_port() {
        // A host with a public address: a remote peer reaching our low port.
        let mut t = ThreatTracker::new();
        let me = Ipv4Addr::new(61, 105, 97, 216);
        let far = Ipv4Addr::new(8, 8, 4, 4);
        t.set_local_ips(&[IpAddr::V4(me)]);
        assert!(
            t.observe(&tcp(far, 50000, me, 9999, ACK, HTTP, 1_000))
                .is_none()
        );
        assert!(
            t.observe(&tcp(far, 50000, me, 9999, SYN, &[], 2_000))
                .is_none()
        );
        let outbound = t.observe(&tcp(me, 50000, far, 9999, ACK, HTTP, 3_000));
        assert!(
            outbound.is_some(),
            "outbound from the local address still tags"
        );
    }

    #[test]
    fn rare_dest_ignores_tls_records_and_empty_segments() {
        let mut t = ThreatTracker::new();
        let me = Ipv4Addr::new(10, 0, 0, 5);
        let far = Ipv4Addr::new(8, 8, 4, 4);
        // application_data record, TLS 1.2 version bytes, length 32
        let tls = [0x17, 0x03, 0x03, 0x00, 0x20, 0xAA, 0xBB];
        assert!(
            t.observe(&tcp(me, 50000, far, 9999, ACK, &tls, 1_000))
                .is_none()
        );
        assert!(
            t.observe(&tcp(me, 50000, far, 9999, ACK, &[], 2_000))
                .is_none()
        );
        let fin_ack = 0x11;
        assert!(
            t.observe(&tcp(me, 50000, far, 9999, fin_ack, &[], 3_000))
                .is_none()
        );
        let plain = t.observe(&tcp(me, 50000, far, 9999, ACK, HTTP, 4_000));
        assert!(plain.is_some(), "cleartext data after them still tags");
    }
}
