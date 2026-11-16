//! Capture-side file I/O kept off the render loop: the pcap/pcapng writer
//! thread and the plain-text packet log export.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use crossbeam_channel::{Sender, unbounded};
use network_monitor_core::app_dir::app_dir_in;
use network_monitor_core::decode::DecodedPacket;
use network_monitor_core::output::PacketSink;
use network_monitor_core::output::log_writer;

use super::link_guard::{RootGuard, resolve_write_target, root_guard};

// 1. writer thread ------------------------------------------------------------
struct WriteJob {
    timestamp: chrono::DateTime<chrono::Utc>,
    data: Vec<u8>,
    wire_len: u32,
}

// Owns the sink on its own thread; the UI thread only queues packet copies.
pub struct PacketWriter {
    jobs: Option<Sender<WriteJob>>,
    handle: Option<JoinHandle<anyhow::Result<()>>>,
}

impl PacketWriter {
    pub fn spawn(mut sink: Box<dyn PacketSink + Send>) -> Self {
        let (jobs, queue) = unbounded::<WriteJob>();
        let handle = std::thread::Builder::new()
            .name("pcap-writer".into())
            .spawn(move || {
                // A failed write is kept and reported once at the end; later writes still try.
                let mut failure: Option<anyhow::Error> = None;
                for job in queue {
                    if let Err(error) = sink.write_packet(job.timestamp, &job.data, job.wire_len)
                        && failure.is_none()
                    {
                        failure = Some(error);
                    }
                }
                sink.flush()?;
                failure.map_or(Ok(()), Err)
            })
            .ok();
        Self {
            jobs: Some(jobs),
            handle,
        }
    }

    pub fn queue_packet(&self, packet: &DecodedPacket) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(WriteJob {
                timestamp: packet.timestamp,
                data: packet.data.clone(),
                wire_len: packet.wire_len,
            });
        }
    }

    // Closes the queue, waits for the thread to flush, and returns its result.
    pub fn finish(mut self) -> anyhow::Result<()> {
        self.jobs.take();
        match self.handle.take() {
            Some(handle) => handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("writer thread panicked"))),
            None => Ok(()),
        }
    }
}

// 2. log export ---------------------------------------------------------------
// Writes the packets to `network-monitor-<time>.log` in the per-user folder
// under `home` (injected so tests never touch the real profile); returns the
// packet count and the file path, or the error text. Without a home directory
// log saving is off; as root, a symbolic link owned by another user anywhere
// on the path is refused.
pub fn write_log_in(
    home: Option<&Path>,
    packets: &[DecodedPacket],
    local_ips: &[IpAddr],
) -> Result<(usize, PathBuf), String> {
    write_log_guarded(home, packets, local_ips, root_guard().as_ref())
}

fn write_log_guarded(
    home: Option<&Path>,
    packets: &[DecodedPacket],
    local_ips: &[IpAddr],
    guard: Option<&RootGuard>,
) -> Result<(usize, PathBuf), String> {
    let Some(home) = home else {
        return Err("no home directory (USERPROFILE and HOME are unset)".to_string());
    };
    let name = format!(
        "network-monitor-{}.log",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = app_dir_in(home).join(name);
    resolve_write_target(&path, guard)
        .and_then(|target| {
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            log_writer::write_packet_log(&target, packets, local_ips)
        })
        .map(|count| (count, path))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct RecordingSink {
        seen: Arc<Mutex<Vec<(u32, usize)>>>,
        flushed: Arc<Mutex<bool>>,
    }

    impl PacketSink for RecordingSink {
        fn write_packet(
            &mut self,
            _timestamp: chrono::DateTime<chrono::Utc>,
            data: &[u8],
            wire_len: u32,
        ) -> anyhow::Result<()> {
            self.seen.lock().unwrap().push((wire_len, data.len()));
            Ok(())
        }

        fn flush(&mut self) -> anyhow::Result<()> {
            *self.flushed.lock().unwrap() = true;
            Ok(())
        }
    }

    fn sample_packet(wire_len: u32) -> DecodedPacket {
        use network_monitor_core::capture::{Linktype, RawPacket};
        network_monitor_core::decode::decode_packet(RawPacket {
            number: 1,
            timestamp: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            wire_len,
            data: vec![0u8; 64],
            linktype: Linktype::Ethernet,
        })
    }

    #[test]
    fn writer_thread_receives_every_queued_packet_and_flushes_on_finish() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let flushed = Arc::new(Mutex::new(false));
        let writer = PacketWriter::spawn(Box::new(RecordingSink {
            seen: seen.clone(),
            flushed: flushed.clone(),
        }));
        for wire_len in [60, 70, 80] {
            writer.queue_packet(&sample_packet(wire_len));
        }
        writer.finish().expect("clean finish");
        assert_eq!(*seen.lock().unwrap(), vec![(60, 64), (70, 64), (80, 64)]);
        assert!(*flushed.lock().unwrap());
    }

    #[test]
    fn log_lands_under_the_injected_home_config_dir_and_empty_input_writes_a_header_only_log() {
        let home = tempfile::tempdir().expect("temp home");
        let (count, path) =
            write_log_in(Some(home.path()), &[sample_packet(64)], &[]).expect("log written");
        assert_eq!(count, 1);
        assert_eq!(path.parent().unwrap(), app_dir_in(home.path()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("network-monitor-") && name.ends_with(".log"));
    }

    #[test]
    fn without_a_home_log_saving_is_off_with_a_typed_reason() {
        let error = write_log_in(None, &[sample_packet(64)], &[]).unwrap_err();
        assert!(error.contains("USERPROFILE"), "{error}");
    }

    fn foreign_owner(_path: &Path) -> std::io::Result<u32> {
        Ok(1234)
    }

    #[test]
    fn root_refuses_a_foreign_linked_app_dir_and_a_plain_user_follows_it() {
        let home = tempfile::tempdir().expect("temp home");
        let elsewhere = tempfile::tempdir().expect("temp target");
        let config = home.path().join(".config");
        std::fs::create_dir_all(&config).unwrap();
        let app_link = config.join("network-monitor");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(elsewhere.path(), &app_link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(elsewhere.path(), &app_link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if !made {
            eprintln!("skipped: the OS refused to create a symlink");
            return;
        }
        let guard = RootGuard {
            owner_of: &foreign_owner,
        };
        let error = write_log_guarded(Some(home.path()), &[sample_packet(64)], &[], Some(&guard))
            .unwrap_err();
        assert!(error.contains("another user"), "{error}");
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);

        let (count, _) =
            write_log_guarded(Some(home.path()), &[sample_packet(64)], &[], None).unwrap();
        assert_eq!(count, 1);
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 1);
    }
}
