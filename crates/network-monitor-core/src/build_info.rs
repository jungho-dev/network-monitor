//! Identity of the running executable: its path, its build identifier
//! (`"<file size>-<modified UNIX ms>"`, taken once at startup so a rebuilt exe
//! on disk does not change it), and the arguments it was launched with.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

/// Path of this executable; `None` when the OS cannot report it.
pub static EXE_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(|| std::env::current_exe().ok());

/// Build identifier of this executable at startup; empty when unreadable.
pub static BUILD_ID: LazyLock<String> =
    LazyLock::new(|| EXE_PATH.as_deref().map(read_build_id).unwrap_or_default());

/// Fixes both values at startup; call it first in `main`.
pub fn init() {
    LazyLock::force(&EXE_PATH);
    LazyLock::force(&BUILD_ID);
}

/// `"<file size>-<modified UNIX ms>"` of the file at `path`; empty when its
/// metadata or modified time cannot be read.
pub fn read_build_id(path: &Path) -> String {
    let Ok(metadata) = std::fs::metadata(path) else {
        return String::new();
    };
    let Ok(modified) = metadata.modified() else {
        return String::new();
    };
    let Ok(elapsed) = modified.duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    format!("{}-{}", metadata.len(), elapsed.as_millis())
}

/// Launch arguments after the program name; empty when any is not UTF-8.
pub fn launch_args() -> Vec<String> {
    let args: Option<Vec<String>> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().ok())
        .collect();
    args.unwrap_or_default()
}

/// This executable's path as text; empty when unknown or not UTF-8.
pub fn exe_path_text() -> String {
    let Some(path) = EXE_PATH.as_deref().and_then(Path::to_str) else {
        return String::new();
    };
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_id_is_the_file_size_and_modified_millis() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("exe");
        std::fs::write(&path, b"12345").expect("write");
        let modified_ms = std::fs::metadata(&path)
            .expect("metadata")
            .modified()
            .expect("modified")
            .duration_since(UNIX_EPOCH)
            .expect("after epoch")
            .as_millis();
        assert_eq!(read_build_id(&path), format!("5-{modified_ms}"));
    }

    #[test]
    fn build_id_of_a_missing_path_is_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(read_build_id(&dir.path().join("missing")), "");
    }
}
