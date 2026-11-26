//! Log-level style output level of the capture TUI: debug, info, warn, and
//! error, plus the mapping from a packet's threat, anomaly, and retransmission
//! marks to its level.

use network_monitor_core::analysis::{AnomalySeverity, ThreatKind, analyze};
use network_monitor_core::decode::DecodedPacket;

// 1. output level -------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum OutputLevel {
    #[default]
    Debug,
    Info,
    Warn,
    Error,
}

impl OutputLevel {
    pub const ALL: [OutputLevel; 4] = [
        OutputLevel::Debug,
        OutputLevel::Info,
        OutputLevel::Warn,
        OutputLevel::Error,
    ];

    // Stable code used by config.toml, tui.toml, and the list card hint.
    pub fn code(self) -> &'static str {
        match self {
            OutputLevel::Debug => "debug",
            OutputLevel::Info => "info",
            OutputLevel::Warn => "warn",
            OutputLevel::Error => "error",
        }
    }

    // Case and surrounding whitespace are ignored; an unknown code is None.
    pub fn from_code(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(OutputLevel::Debug),
            "info" => Some(OutputLevel::Info),
            "warn" => Some(OutputLevel::Warn),
            "error" => Some(OutputLevel::Error),
            _ => None,
        }
    }

    // Debug -> Info -> Warn -> Error -> Debug.
    pub fn next(self) -> Self {
        match self {
            OutputLevel::Debug => OutputLevel::Info,
            OutputLevel::Info => OutputLevel::Warn,
            OutputLevel::Warn => OutputLevel::Error,
            OutputLevel::Error => OutputLevel::Debug,
        }
    }
}

// 2. packet level -------------------------------------------------------------
// error = port scan or host sweep; warn = rare external destination, warning
// anomaly, or retransmission; info = info anomaly; debug = everything else.
pub fn level_of(
    threat: Option<ThreatKind>,
    anomaly: Option<AnomalySeverity>,
    retransmission: bool,
) -> OutputLevel {
    if matches!(threat, Some(ThreatKind::PortScan | ThreatKind::HostSweep)) {
        return OutputLevel::Error;
    }
    if threat == Some(ThreatKind::RareExternal)
        || anomaly == Some(AnomalySeverity::Warning)
        || retransmission
    {
        return OutputLevel::Warn;
    }
    if anomaly == Some(AnomalySeverity::Info) {
        return OutputLevel::Info;
    }
    OutputLevel::Debug
}

// Same `analyze` pass as the packet list rows, then `level_of`.
pub fn packet_level(packet: &DecodedPacket) -> OutputLevel {
    let anomaly = analyze(packet);
    level_of(
        packet.threat.as_ref().map(|threat| threat.kind),
        anomaly.map(|found| found.severity),
        packet.retransmission,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_threats_are_error_whatever_else_is_marked() {
        for kind in [ThreatKind::PortScan, ThreatKind::HostSweep] {
            assert_eq!(level_of(Some(kind), None, false), OutputLevel::Error);
            assert_eq!(
                level_of(Some(kind), Some(AnomalySeverity::Info), true),
                OutputLevel::Error
            );
        }
    }

    #[test]
    fn rare_destination_warning_anomaly_and_retransmission_are_warn() {
        assert_eq!(
            level_of(Some(ThreatKind::RareExternal), None, false),
            OutputLevel::Warn
        );
        assert_eq!(
            level_of(None, Some(AnomalySeverity::Warning), false),
            OutputLevel::Warn
        );
        assert_eq!(level_of(None, None, true), OutputLevel::Warn);
        assert_eq!(
            level_of(None, Some(AnomalySeverity::Info), true),
            OutputLevel::Warn
        );
    }

    #[test]
    fn info_anomaly_is_info_and_unmarked_packets_are_debug() {
        assert_eq!(
            level_of(None, Some(AnomalySeverity::Info), false),
            OutputLevel::Info
        );
        assert_eq!(level_of(None, None, false), OutputLevel::Debug);
    }

    #[test]
    fn from_code_ignores_case_and_whitespace_and_rejects_unknown_codes() {
        for level in OutputLevel::ALL {
            assert_eq!(OutputLevel::from_code(level.code()), Some(level));
        }
        assert_eq!(OutputLevel::from_code(" WARN "), Some(OutputLevel::Warn));
        assert_eq!(OutputLevel::from_code("Error\n"), Some(OutputLevel::Error));
        assert_eq!(OutputLevel::from_code("loud"), None);
        assert_eq!(OutputLevel::from_code(""), None);
    }

    #[test]
    fn next_cycles_through_every_level_and_wraps_to_debug() {
        assert_eq!(OutputLevel::Debug.next(), OutputLevel::Info);
        assert_eq!(OutputLevel::Info.next(), OutputLevel::Warn);
        assert_eq!(OutputLevel::Warn.next(), OutputLevel::Error);
        assert_eq!(OutputLevel::Error.next(), OutputLevel::Debug);
        assert_eq!(OutputLevel::default(), OutputLevel::Debug);
    }
}
