use crate::app_dir::{app_dir_in, home_dir};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// Filesystem locations for the daemon's runtime state.
pub struct Paths {
    pub state_dir: PathBuf,
    pub socket: PathBuf,
    pub db: PathBuf,
    pub pidfile: PathBuf,
    pub log: PathBuf,
}

/// Resolve runtime paths, defaulting to `~/.config/network-monitor` (the same
/// folder as `config.toml`). Fails when neither `--state-dir` nor a home
/// directory (`USERPROFILE`, then `HOME`) is available.
pub fn resolve(state_dir: Option<PathBuf>) -> Result<Paths> {
    resolve_in(state_dir, home_dir().as_deref())
}

/// `home` is injected so tests never read the real profile.
pub fn resolve_in(state_dir: Option<PathBuf>, home: Option<&Path>) -> Result<Paths> {
    let dir = match (state_dir, home) {
        (Some(dir), _) => dir,
        (None, Some(home)) => app_dir_in(home),
        (None, None) => {
            bail!("no home directory (USERPROFILE and HOME are unset); pass --state-dir")
        }
    };
    Ok(Paths {
        socket: dir.join("network-monitor.sock"),
        db: dir.join("network-monitor.db"),
        pidfile: dir.join("network-monitor.pid"),
        log: dir.join("monitor.log"),
        state_dir: dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_paths_live_under_dot_config_of_the_injected_home() {
        let home = tempfile::tempdir().expect("temp home");
        let paths = resolve_in(None, Some(home.path())).expect("paths");
        let dir = home.path().join(".config").join("network-monitor");
        assert_eq!(paths.state_dir, dir);
        assert_eq!(paths.socket, dir.join("network-monitor.sock"));
        assert_eq!(paths.db, dir.join("network-monitor.db"));
        assert_eq!(paths.pidfile, dir.join("network-monitor.pid"));
        assert_eq!(paths.log, dir.join("monitor.log"));
    }

    #[test]
    fn state_dir_override_wins_over_the_home() {
        let home = tempfile::tempdir().expect("temp home");
        let custom = home.path().join("custom");
        let paths = resolve_in(Some(custom.clone()), Some(home.path())).expect("paths");
        assert_eq!(paths.state_dir, custom);
        assert_eq!(paths.db, custom.join("network-monitor.db"));
    }

    #[test]
    fn state_dir_override_works_without_a_home() {
        let custom = PathBuf::from("custom-state");
        let paths = resolve_in(Some(custom.clone()), None).expect("paths");
        assert_eq!(paths.socket, custom.join("network-monitor.sock"));
    }

    #[test]
    fn no_home_and_no_state_dir_is_an_error_not_a_shared_tmp_folder() {
        let error = resolve_in(None, None).err().expect("an error");
        let text = error.to_string();
        assert!(
            text.contains("USERPROFILE") && text.contains("--state-dir"),
            "{text}"
        );
    }
}
