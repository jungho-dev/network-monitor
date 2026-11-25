//! Force restart (Unix): checks every precondition first, then stops a daemon
//! whose exe on disk holds a newer build, starts it again from that exe with
//! its own launch arguments, and replaces this inspector with its own exe.

use std::fs::OpenOptions;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use network_monitor_core::build_info::{EXE_PATH, read_build_id};
use network_monitor_core::ipc::protocol::DaemonStatus;
use network_monitor_core::monitor::{monitor_status, monitor_stop};

use crate::core::permissions::check_capture_permissions;
use crate::inspect::state::{PreflightCache, PreflightOutcome, RestartBlock, RestartState};
use crate::inspect::state::{daemon_is_stale, daemon_restart_args};
use crate::inspect::state::{preflight_outcome, shell_quote_args};
use crate::tui::i18n::{self, Lang};

// Waits for the old daemon to release its socket and for the new one to answer.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const READY_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

// The daemon's own exe and the arguments that start it again.
pub struct RestartPlan {
    pub daemon_exe: PathBuf,
    pub run_args: Vec<String>,
}

// 1. plan ---------------------------------------------------------------------
// `None` when the daemon's exe on disk still holds its build; an error names
// the first precondition a restart would fail on.
pub fn plan_restart(
    status: &DaemonStatus,
    socket: &Path,
    preflight: &mut PreflightCache,
) -> Result<Option<RestartPlan>, RestartBlock> {
    // 1. a newer build of the daemon's own exe on disk
    let disk_build = read_build_id(Path::new(&status.exe_path));
    if !daemon_is_stale(&status.build_id, &disk_build, &status.run_args) {
        return Ok(None);
    }

    // 2. absolute exe and state folder, then the restart arguments
    let Some(state_dir) = socket.parent().and_then(Path::to_str) else {
        return Err(RestartBlock::RelativePath);
    };
    if !status.exe_path.starts_with('/') {
        return Err(RestartBlock::RelativePath);
    }
    let run_args = daemon_restart_args(&status.run_args, state_dir)?;

    // 3. daemon exe owned by root or by the owner of this inspector's exe
    let daemon_exe = PathBuf::from(&status.exe_path);
    let daemon_meta = std::fs::metadata(&daemon_exe);
    let own_meta = EXE_PATH.as_deref().map(std::fs::metadata);
    let (Ok(daemon_meta), Some(Ok(own_meta))) = (daemon_meta, own_meta) else {
        return Err(RestartBlock::ExeOwner);
    };
    if daemon_meta.uid() != 0 && daemon_meta.uid() != own_meta.uid() {
        return Err(RestartBlock::ExeOwner);
    }

    // 4. capture privilege
    if check_capture_permissions().is_err() {
        return Err(RestartBlock::Privilege);
    }
    #[cfg(target_os = "macos")]
    {
        let bpf = std::fs::File::open("/dev/bpf0");
        if bpf.is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied) {
            return Err(RestartBlock::Privilege);
        }
    }

    // 5. capture privilege of the new exe itself, run only under this
    // inspector's exe name, bounded in time, and cached per disk build
    let own_name = EXE_PATH.as_deref().and_then(Path::file_name);
    if own_name.is_none() || daemon_exe.file_name() != own_name {
        return Err(RestartBlock::ExeName);
    }
    let cached = preflight.lookup(&disk_build, Instant::now());
    let mut child = if cached.is_none() {
        Command::new(&daemon_exe)
            .args(["monitor", "preflight"])
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    } else {
        None
    };
    let check_started = Instant::now();
    let mut exit_code = None;
    while let Some(running) = child.as_mut() {
        match running.try_wait() {
            Ok(Some(exit)) => {
                exit_code = exit.code();
                break;
            }
            Ok(None) if check_started.elapsed() < PREFLIGHT_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            // past the limit or a failed poll: the answer stays unknown
            // whether or not the kill and reap succeed
            Ok(None) | Err(_) => {
                let _ = running.kill();
                let _ = running.wait();
                break;
            }
        }
    }
    let outcome = match cached {
        Some(outcome) => outcome,
        None => {
            let checked = preflight_outcome(exit_code);
            preflight.store(&disk_build, checked, Instant::now());
            checked
        }
    };
    match outcome {
        PreflightOutcome::Passed => {}
        PreflightOutcome::NoPrivilege => return Err(RestartBlock::NewExePrivilege),
        PreflightOutcome::Unknown => return Err(RestartBlock::PreflightUnknown),
    }

    // 6. write access to the state folder and its existing files
    let state_path = Path::new(state_dir);
    let probe = state_path.join(format!(".restart-probe-{}", std::process::id()));
    let probe_open = OpenOptions::new().write(true).create_new(true).open(&probe);
    if probe_open.is_err() {
        return Err(RestartBlock::StateAccess);
    }
    drop(probe_open);
    if std::fs::remove_file(&probe).is_err() {
        return Err(RestartBlock::StateAccess);
    }
    let state_files = [
        "network-monitor.db",
        "network-monitor.db-wal",
        "network-monitor.db-shm",
        "monitor.log",
    ];
    for name in state_files {
        let path = state_path.join(name);
        if path.exists() && OpenOptions::new().append(true).open(&path).is_err() {
            return Err(RestartBlock::StateAccess);
        }
    }

    Ok(Some(RestartPlan {
        daemon_exe,
        run_args,
    }))
}

pub fn restart_state(
    status: &DaemonStatus,
    socket: &Path,
    preflight: &mut PreflightCache,
) -> RestartState {
    match plan_restart(status, socket, preflight) {
        Ok(None) => RestartState::Current,
        Ok(Some(_)) => RestartState::Ready,
        Err(block) => RestartState::Blocked(block),
    }
}

// 2. restart ------------------------------------------------------------------
pub fn restart_daemon_and_inspector(socket: &Path) -> Result<()> {
    // 1. this inspector's exe and the daemon's current status
    let exe_path = EXE_PATH
        .as_deref()
        .ok_or_else(|| anyhow!("cannot resolve the path of this executable"))?;
    let status = monitor_status(socket)
        .with_context(|| format!("querying the daemon at {} failed", socket.display()))?;

    // 2. every precondition holds before the daemon is touched
    let mut preflight = PreflightCache::default();
    let plan = match plan_restart(&status, socket, &mut preflight) {
        Ok(plan) => plan,
        Err(block) => bail!(
            "{}; the daemon was left running",
            i18n::text(Lang::En, block.toast())
        ),
    };

    // 3. the daemon stops, then starts again from its own exe
    if let Some(plan) = plan {
        let manual = shell_quote_args(&plan.daemon_exe.to_string_lossy(), &plan.run_args);
        monitor_stop(socket).context("stopping the daemon failed")?;
        let stop_started = Instant::now();
        while socket.exists() {
            if stop_started.elapsed() >= STOP_TIMEOUT {
                bail!(
                    "the daemon did not release {} within {}s; if the daemon stops later, start it with: {manual}",
                    socket.display(),
                    STOP_TIMEOUT.as_secs()
                );
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        let log_path = socket.with_file_name("monitor.log");
        let launched = Command::new(&plan.daemon_exe)
            .args(&plan.run_args)
            .stdin(Stdio::null())
            .status();
        let started = match launched {
            Ok(exit) if exit.success() => true,
            Ok(exit) => {
                eprintln!("network-monitor: daemon start exited with {exit}");
                false
            }
            Err(error) => {
                eprintln!("network-monitor: daemon start failed: {error}");
                false
            }
        };
        let ready_started = Instant::now();
        let mut ready = false;
        while started && ready_started.elapsed() < READY_TIMEOUT {
            if let Ok(fresh) = monitor_status(socket)
                && fresh.pid != status.pid
            {
                ready = true;
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        if !ready {
            bail!(
                "the daemon did not come back on the latest build; start it by hand: {manual} (log: {})",
                log_path.display()
            );
        }
    }

    // 4. this inspector is replaced by its own exe with the same arguments
    let error = Command::new(exe_path)
        .args(std::env::args_os().skip(1))
        .exec();
    Err(anyhow!(error).context(format!(
        "rerunning the inspector from {} failed",
        exe_path.display()
    )))
}
