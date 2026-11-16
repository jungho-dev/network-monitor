/// Linux capability number of CAP_NET_RAW (raw and packet sockets).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const CAP_NET_RAW_BIT: u32 = 13;

pub fn check_capture_permissions() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let is_root = unsafe { libc::geteuid() } == 0;
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if !is_root && !has_effective_capability(&status, CAP_NET_RAW_BIT) {
            return Err(
                network_monitor_core::error::NetworkMonitorError::Permission(
                    "Capture needs root or the CAP_NET_RAW capability. Run with sudo, or grant it once:\n  \
                 sudo setcap cap_net_raw,cap_net_admin=eip \"$(command -v network-monitor)\""
                        .into(),
                )
                .into(),
            );
        }
    }
    #[cfg(target_os = "windows")]
    {
        eprintln!(
            "Note: Ensure Npcap is installed (https://npcap.com) and run from an Administrator prompt."
        );
    }

    Ok(())
}

/// Reads the `CapEff:` hex mask of a `/proc/<pid>/status` text and tests one capability bit.
/// A missing or malformed line counts as "not granted".
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn has_effective_capability(status: &str, bit: u32) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| bit < 64 && mask & (1u64 << bit) != 0)
}

#[cfg(test)]
mod tests {
    use super::{CAP_NET_RAW_BIT, has_effective_capability};

    #[test]
    fn reads_cap_net_raw_from_capeff() {
        let granted =
            "Name:\tnetwork-monitor\nCapInh:\t0000000000000000\nCapEff:\t0000000000003000\n";
        let denied = "Name:\tnetwork-monitor\nCapEff:\t0000000000000000\n";
        assert!(has_effective_capability(granted, CAP_NET_RAW_BIT));
        assert!(!has_effective_capability(denied, CAP_NET_RAW_BIT));
    }

    #[test]
    fn missing_or_malformed_capeff_is_not_granted() {
        assert!(!has_effective_capability("Name:\tx\n", CAP_NET_RAW_BIT));
        assert!(!has_effective_capability("CapEff:\tzz\n", CAP_NET_RAW_BIT));
        assert!(!has_effective_capability("CapEff:\tffffffffffffffff\n", 64));
    }
}
