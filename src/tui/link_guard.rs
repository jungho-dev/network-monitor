//! Symbolic links under files the TUI writes (`tui.toml`, saved capture logs):
//! a save follows a link chain to the file it must replace, within a bounded
//! depth, and a process running as root refuses to follow a link that another
//! user owns, so a planted link cannot redirect a root write.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

// Links followed before the chain counts as a loop.
pub const MAX_LINK_DEPTH: usize = 16;

// Owner lookup of a path without following it; injected so tests never need root.
pub type OwnerOf<'a> = &'a dyn Fn(&Path) -> io::Result<u32>;

// Exists only for a process running as root: a link owned by any other user is refused.
pub struct RootGuard<'a> {
    pub owner_of: OwnerOf<'a>,
}

// 1. root detection -----------------------------------------------------------
// Used by the root guard, which only a Unix host builds outside the tests.
#[cfg_attr(not(unix), allow(dead_code))]
fn lstat_owner(path: &Path) -> io::Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(fs::symlink_metadata(path)?.uid())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "owner lookup needs a Unix host",
        ))
    }
}

// The guard for an effective user id: only uid 0 gets one.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn root_guard_for(euid: u32) -> Option<RootGuard<'static>> {
    (euid == 0).then_some(RootGuard {
        owner_of: &lstat_owner,
    })
}

// The guard for this process (always None off Unix).
pub fn root_guard() -> Option<RootGuard<'static>> {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        root_guard_for(unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

// 2. link checks --------------------------------------------------------------
// Every existing component of `path` that is a link must be owned by root.
pub fn check_links_on_path(path: &Path, guard: Option<&RootGuard>) -> io::Result<()> {
    let Some(guard) = guard else {
        return Ok(());
    };
    for prefix in path.ancestors().filter(|part| !part.as_os_str().is_empty()) {
        let is_link = fs::symlink_metadata(prefix).is_ok_and(|meta| meta.file_type().is_symlink());
        if is_link && (guard.owner_of)(prefix)? != 0 {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "refusing to follow a symbolic link owned by another user: {}",
                    prefix.display()
                ),
            ));
        }
    }
    Ok(())
}

// The file a write must replace: the end of a link chain (also a dangling
// one) with directory links resolved, else `path` itself. Errors on a chain
// deeper than MAX_LINK_DEPTH or, for a root guard, on a foreign-owned link.
pub fn resolve_write_target(path: &Path, guard: Option<&RootGuard>) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..=MAX_LINK_DEPTH {
        check_links_on_path(&current, guard)?;
        let is_link =
            fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink());
        if !is_link {
            let resolved_dir = current
                .parent()
                .filter(|dir| !dir.as_os_str().is_empty())
                .and_then(|dir| fs::canonicalize(dir).ok());
            return Ok(match (resolved_dir, current.file_name()) {
                (Some(dir), Some(name)) => dir.join(name),
                _ => current,
            });
        }
        let target = fs::read_link(&current)?;
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(target)
        };
    }
    Err(io::Error::new(
        ErrorKind::InvalidInput,
        format!(
            "symbolic link chain deeper than {MAX_LINK_DEPTH}: {}",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let outcome = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let outcome = std::os::windows::fs::symlink_file(target, link);
        #[cfg(not(any(unix, windows)))]
        let outcome: io::Result<()> = Err(io::Error::new(ErrorKind::Unsupported, "no links"));
        match outcome {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipped: the OS refused to create a symlink ({error})");
                false
            }
        }
    }

    fn foreign_owner(_path: &Path) -> io::Result<u32> {
        Ok(1234)
    }

    fn root_owner(_path: &Path) -> io::Result<u32> {
        Ok(0)
    }

    #[test]
    fn only_uid_zero_gets_a_guard() {
        assert!(root_guard_for(0).is_some());
        assert!(root_guard_for(1000).is_none());
    }

    #[test]
    fn a_plain_path_resolves_to_itself_with_its_directory_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tui.toml");
        let resolved = resolve_write_target(&file, None).unwrap();
        assert_eq!(resolved.file_name(), file.file_name());
        assert_eq!(
            resolved.parent().unwrap(),
            fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn a_multi_level_chain_resolves_to_the_final_target_even_when_it_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("final.toml");
        let middle = dir.path().join("middle.toml");
        let first = dir.path().join("first.toml");
        if !make_link(&target, &middle) || !make_link(&middle, &first) {
            return;
        }
        let resolved = resolve_write_target(&first, None).unwrap();
        assert_eq!(resolved.file_name().unwrap(), "final.toml");
        assert!(!resolved.exists());
        assert!(
            fs::symlink_metadata(&first)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_link_loop_is_refused_after_the_bounded_depth() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.toml");
        let right = dir.path().join("right.toml");
        if !make_link(&right, &left) || !make_link(&left, &right) {
            return;
        }
        let error = resolve_write_target(&left, None).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("deeper than"), "{error}");
    }

    #[test]
    fn a_chain_longer_than_the_bound_is_refused_and_one_at_the_bound_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let end = dir.path().join("end.toml");
        let mut previous = end.clone();
        let mut links = Vec::new();
        for index in 0..=MAX_LINK_DEPTH {
            let link = dir.path().join(format!("link-{index}.toml"));
            if !make_link(&previous, &link) {
                return;
            }
            previous = link.clone();
            links.push(link);
        }
        // links[MAX_LINK_DEPTH] is the head of a chain of MAX_LINK_DEPTH + 1 links.
        let head = links.last().unwrap();
        assert!(resolve_write_target(head, None).is_err());
        let one_shorter = &links[MAX_LINK_DEPTH - 1];
        let resolved = resolve_write_target(one_shorter, None).unwrap();
        assert_eq!(resolved.file_name().unwrap(), "end.toml");
    }

    #[test]
    fn root_refuses_a_link_owned_by_another_user_and_follows_a_root_owned_one() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let link = dir.path().join("link.toml");
        if !make_link(&target, &link) {
            return;
        }
        let foreign = RootGuard {
            owner_of: &foreign_owner,
        };
        let error = resolve_write_target(&link, Some(&foreign)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("another user"), "{error}");

        let own = RootGuard {
            owner_of: &root_owner,
        };
        let resolved = resolve_write_target(&link, Some(&own)).unwrap();
        assert_eq!(resolved.file_name().unwrap(), "target.toml");
    }

    #[test]
    fn root_refuses_a_foreign_directory_link_above_the_file_and_ignores_plain_paths() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("real");
        fs::create_dir_all(&real_dir).unwrap();
        let dir_link = dir.path().join("dir-link");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real_dir, &dir_link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&real_dir, &dir_link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if made {
            let foreign = RootGuard {
                owner_of: &foreign_owner,
            };
            let through_link = dir_link.join("tui.toml");
            assert!(check_links_on_path(&through_link, Some(&foreign)).is_err());
        }
        let foreign = RootGuard {
            owner_of: &foreign_owner,
        };
        let plain = real_dir.join("tui.toml");
        assert!(check_links_on_path(&plain, Some(&foreign)).is_ok());
        assert!(check_links_on_path(&dir_link.join("tui.toml"), None).is_ok());
    }
}
