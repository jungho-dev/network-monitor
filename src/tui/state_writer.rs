//! State-file writer thread: commit events queue a whole-file save, a burst for
//! one path is coalesced to the newest text, a failed write is reported through
//! a callback (the host turns it into a typed message and an error toast), and
//! the exit path flushes what is queued with a bounded wait.

use std::io;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, SendError, Sender, unbounded};

use super::layout_store::write_state_atomic;

// Longest the exit path waits for the writer before it gives up on it.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

// One save request: the state file path and the full text the UI thread built.
pub struct StateSave {
    pub path: PathBuf,
    pub text: String,
}

// How the exit flush ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateFinish {
    Done,
    // The last write failed; the text is the raw error.
    Failed(String),
    TimedOut,
}

impl StateFinish {
    // The one stderr line for a flush that did not end cleanly.
    pub fn warning_text(&self, timeout: Duration) -> Option<String> {
        match self {
            Self::Done => None,
            Self::Failed(detail) => Some(format!("network-monitor: UI state not saved: {detail}")),
            Self::TimedOut => Some(format!(
                "network-monitor: UI state still being written after {}s; the last change may be lost",
                timeout.as_secs()
            )),
        }
    }
}

// 1. writer handle ------------------------------------------------------------
// Owned by the host; Drop flushes with the same bounded wait.
pub struct StateWriter {
    requests: Option<Sender<StateSave>>,
    outcome: Receiver<Option<String>>,
    worker: Option<JoinHandle<()>>,
}

impl StateWriter {
    // Queues one save; a writer thread that already ended hands the request back.
    pub fn queue_state_save(&self, save: StateSave) -> Result<(), SendError<StateSave>> {
        match &self.requests {
            Some(requests) => requests.send(save),
            None => Err(SendError(save)),
        }
    }

    // Closes the queue so the thread writes what is left, then waits at most
    // `timeout`; a thread still busy is detached. Safe to call again (Done).
    pub fn finish_state_saves(&mut self, timeout: Duration) -> StateFinish {
        self.requests = None;
        let Some(worker) = self.worker.take() else {
            return StateFinish::Done;
        };
        match self.outcome.recv_timeout(timeout) {
            Ok(None) => {
                let _ = worker.join();
                StateFinish::Done
            }
            Ok(Some(detail)) => {
                let _ = worker.join();
                StateFinish::Failed(detail)
            }
            Err(RecvTimeoutError::Timeout) => StateFinish::TimedOut,
            Err(RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                StateFinish::Failed("the state writer thread ended unexpectedly".to_string())
            }
        }
    }

    // The exit path: flush with JOIN_TIMEOUT and print one stderr line when it
    // failed or timed out (the terminal is already restored).
    pub fn finish_with_warning(&mut self) {
        let finish = self.finish_state_saves(JOIN_TIMEOUT);
        if let Some(text) = finish.warning_text(JOIN_TIMEOUT) {
            eprintln!("{text}");
        }
    }
}

impl Drop for StateWriter {
    fn drop(&mut self) {
        let _ = self.finish_state_saves(JOIN_TIMEOUT);
    }
}

// 2. thread start -------------------------------------------------------------
// `report` receives the raw error of every failed write on the writer thread.
pub fn spawn_state_writer(report: impl Fn(String) + Send + 'static) -> io::Result<StateWriter> {
    spawn_state_writer_with(write_state_atomic, report)
}

fn spawn_state_writer_with(
    write: impl Fn(&Path, &str) -> io::Result<()> + Send + 'static,
    report: impl Fn(String) + Send + 'static,
) -> io::Result<StateWriter> {
    let (requests, inbox) = unbounded::<StateSave>();
    let (done, outcome) = unbounded::<Option<String>>();
    let worker = thread::Builder::new()
        .name("tui-state-writer".to_string())
        .spawn(move || {
            let unresolved = run_state_writer(&inbox, write, report);
            let _ = done.send(unresolved);
        })?;
    Ok(StateWriter {
        requests: Some(requests),
        outcome,
        worker: Some(worker),
    })
}

// 3. write loop ---------------------------------------------------------------
// A request pulls in everything already queued, writes only the newest text
// per path (first-seen order), reports each failure and keeps going, and ends
// when every sender is gone. Returns the error of the last write when it failed.
fn run_state_writer(
    inbox: &Receiver<StateSave>,
    write: impl Fn(&Path, &str) -> io::Result<()>,
    report: impl Fn(String),
) -> Option<String> {
    let mut unresolved: Option<String> = None;
    while let Ok(first) = inbox.recv() {
        // 1. gather the queued requests
        let mut pending: Vec<StateSave> = vec![first];
        while let Ok(next) = inbox.try_recv() {
            match pending.iter_mut().find(|queued| queued.path == next.path) {
                Some(queued) => queued.text = next.text,
                None => pending.push(next),
            }
        }

        // 2. write them (a success clears an earlier failure)
        for save in &pending {
            match write(&save.path, &save.text) {
                Ok(()) => unresolved = None,
                Err(error) => {
                    let detail = error.to_string();
                    report(detail.clone());
                    unresolved = Some(detail);
                }
            }
        }
    }
    unresolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::io::ErrorKind;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    fn save(path: &Path, text: &str) -> StateSave {
        StateSave {
            path: path.to_path_buf(),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_burst_for_one_path_writes_only_the_latest_in_first_seen_order() {
        let (requests, inbox) = unbounded::<StateSave>();
        let first = PathBuf::from("first.toml");
        let second = PathBuf::from("second.toml");
        for (path, text) in [
            (&first, "a"),
            (&first, "b"),
            (&second, "x"),
            (&first, "c"),
            (&second, "y"),
        ] {
            requests.send(save(path, text)).unwrap();
        }
        drop(requests);

        let writes: RefCell<Vec<(PathBuf, String)>> = RefCell::new(Vec::new());
        let reports: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let unresolved = run_state_writer(
            &inbox,
            |path, text| {
                writes
                    .borrow_mut()
                    .push((path.to_path_buf(), text.to_string()));
                Ok(())
            },
            |detail| reports.borrow_mut().push(detail),
        );
        assert_eq!(
            *writes.borrow(),
            [(first, "c".to_string()), (second, "y".to_string())]
        );
        assert!(reports.borrow().is_empty());
        assert_eq!(unresolved, None);
    }

    #[test]
    fn a_failed_write_is_reported_and_a_later_success_clears_it() {
        let (requests, inbox) = unbounded::<StateSave>();
        requests.send(save(Path::new("state.toml"), "one")).unwrap();
        drop(requests);
        let reports: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let unresolved = run_state_writer(
            &inbox,
            |_, _| Err(io::Error::new(ErrorKind::PermissionDenied, "access denied")),
            |detail| reports.borrow_mut().push(detail),
        );
        assert_eq!(*reports.borrow(), ["access denied"]);
        assert_eq!(unresolved.as_deref(), Some("access denied"));

        let (requests, inbox) = unbounded::<StateSave>();
        let calls = RefCell::new(0usize);
        requests.send(save(Path::new("a"), "1")).unwrap();
        requests.send(save(Path::new("b"), "2")).unwrap();
        drop(requests);
        let unresolved = run_state_writer(
            &inbox,
            |_, _| {
                *calls.borrow_mut() += 1;
                if *calls.borrow() == 1 {
                    Err(io::Error::other("disk full"))
                } else {
                    Ok(())
                }
            },
            |_| {},
        );
        assert_eq!(*calls.borrow(), 2);
        assert_eq!(unresolved, None);
    }

    #[test]
    fn dropping_the_writer_flushes_queued_requests_before_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tui.toml");
        let writer = spawn_state_writer(|_| {}).unwrap();
        writer
            .queue_state_save(save(&path, "version = 1\n"))
            .unwrap();
        writer
            .queue_state_save(save(&path, "version = 1\nlanguage = \"ko\"\n"))
            .unwrap();
        drop(writer);

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "version = 1\nlanguage = \"ko\"\n"
        );
        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["tui.toml".to_string()]);
    }

    #[test]
    fn a_failed_save_is_reported_through_the_callback_and_the_final_flush_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // The target is a folder, so the rename over it fails.
        let path = dir.path().join("tui.toml");
        fs::create_dir_all(&path).unwrap();
        let (report_tx, report_rx) = mpsc::channel::<String>();
        let mut writer = spawn_state_writer(move |detail| {
            let _ = report_tx.send(detail);
        })
        .unwrap();
        writer
            .queue_state_save(save(&path, "version = 1\n"))
            .unwrap();

        let finish = writer.finish_state_saves(Duration::from_secs(5));
        let detail = report_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!detail.is_empty());
        assert_eq!(finish, StateFinish::Failed(detail));
        let leftovers: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        // After the flush a request comes back, and a second finish is clean.
        let lost = writer.queue_state_save(save(&path, "late")).unwrap_err();
        assert_eq!(lost.0.text, "late");
        assert_eq!(
            writer.finish_state_saves(Duration::from_secs(1)),
            StateFinish::Done
        );
    }

    #[test]
    fn a_writer_stuck_in_a_write_times_out_at_the_bound_and_is_detached() {
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let worker_gate = Arc::clone(&gate);
        let mut writer = spawn_state_writer_with(
            move |_, _| {
                let _wait = worker_gate.lock().unwrap();
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        writer.queue_state_save(save(Path::new("x"), "t")).unwrap();

        let started = std::time::Instant::now();
        let finish = writer.finish_state_saves(Duration::from_millis(150));
        assert_eq!(finish, StateFinish::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            writer.finish_state_saves(Duration::from_millis(10)),
            StateFinish::Done
        );
        drop(held);
    }

    #[test]
    fn the_warning_line_names_a_failure_or_a_timeout_and_a_clean_flush_is_silent() {
        let bound = Duration::from_secs(3);
        assert_eq!(StateFinish::Done.warning_text(bound), None);
        let failed = StateFinish::Failed("disk full".into()).warning_text(bound);
        assert_eq!(
            failed.as_deref(),
            Some("network-monitor: UI state not saved: disk full")
        );
        let timed_out = StateFinish::TimedOut.warning_text(bound).unwrap();
        assert!(timed_out.contains("3s") && timed_out.contains("may be lost"));
    }
}
