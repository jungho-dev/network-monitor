//! The per-user folder `~/.config/network-monitor/`: `config.toml` plus every
//! state file (IPC socket, database, pid file, `monitor.log`, saved capture logs).

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Folder name under `~/.config` (the project name).
pub const APP_DIR_NAME: &str = "network-monitor";

/// The home directory: a non-empty `USERPROFILE` first, then a non-empty
/// `HOME`, or `None` when neither is set. The values are injected so tests
/// never read the real environment.
pub fn home_dir_from(user_profile: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    [user_profile, home]
        .into_iter()
        .flatten()
        .find(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// The home directory of the running process (`USERPROFILE`, then `HOME`).
pub fn home_dir() -> Option<PathBuf> {
    home_dir_from(env::var_os("USERPROFILE"), env::var_os("HOME"))
}

/// `<home>/.config/network-monitor`. `home` is injected so tests never touch
/// the real profile.
pub fn app_dir_in(home: &Path) -> PathBuf {
    home.join(".config").join(APP_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_dir_sits_under_dot_config_of_the_injected_home() {
        let home = Path::new("injected-home");
        assert_eq!(
            app_dir_in(home),
            home.join(".config").join("network-monitor")
        );
    }

    #[test]
    fn userprofile_wins_over_home_and_empty_values_are_skipped() {
        let profile = Some(OsString::from("profile-dir"));
        let home = Some(OsString::from("home-dir"));
        assert_eq!(
            home_dir_from(profile.clone(), home.clone()),
            Some(PathBuf::from("profile-dir"))
        );
        assert_eq!(
            home_dir_from(None, home.clone()),
            Some(PathBuf::from("home-dir"))
        );
        assert_eq!(
            home_dir_from(Some(OsString::new()), home),
            Some(PathBuf::from("home-dir"))
        );
        assert_eq!(
            home_dir_from(profile, None),
            Some(PathBuf::from("profile-dir"))
        );
    }

    #[test]
    fn no_home_resolves_to_none_instead_of_a_shared_tmp_folder() {
        assert_eq!(home_dir_from(None, None), None);
        assert_eq!(
            home_dir_from(Some(OsString::new()), Some(OsString::new())),
            None
        );
    }
}
