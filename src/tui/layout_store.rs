//! TUI state file `tui.toml` in the per-user folder next to `config.toml`:
//! display language, card orders, split ratios, and column widths, restored
//! at start and saved atomically on commit events.
//!
//! The file may be a symbolic link into a synced folder: a save resolves the
//! link chain first (bounded depth) and replaces the target, so the link itself
//! is never replaced; as root, a link owned by another user is refused.
//! Saves run on the state writer thread (`state_writer`), never on the UI thread.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use network_monitor_core::app_dir::app_dir_in;
use serde::{Deserialize, Serialize};

use super::cards::{CARD_COUNT, CardId, MAX_COLUMN_WIDTH, MAX_TABLE_COLUMNS};
use super::i18n::{self, Lang, StateIssue};
use super::interaction::{ToastLevel, Ui};
use super::layout::{CardLayout, SplitGroup};
use super::link_guard::{RootGuard, resolve_write_target, root_guard};
use super::state_writer::{StateSave, StateWriter};

pub const TUI_STATE_FILE_NAME: &str = "tui.toml";
const STATE_VERSION: i64 = 1;
const BACKUP_SUFFIX: &str = ".bak";
const TEMP_SUFFIX: &str = ".tmp";
// A temp file younger than this may belong to a save in progress (another
// running instance shares the file), so startup cleanup leaves it alone.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60);

// 1. location -----------------------------------------------------------------
// `<home>/.config/network-monitor/tui.toml` through the shared app dir; None
// without a home directory (state saving is then off).
pub fn state_path_in(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| app_dir_in(home).join(TUI_STATE_FILE_NAME))
}

// 2. file format --------------------------------------------------------------
// Language is a top-level string (an unknown value is skipped alone); groups
// and cards use stable names; values are integer arrays checked entry by entry.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    version: i64,
    #[serde(default)]
    language: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    orders: BTreeMap<String, Vec<i64>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ratios: BTreeMap<String, Vec<i64>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    column_widths: BTreeMap<String, Vec<i64>>,
}

enum StateRead {
    Missing,
    Loaded(StateFile),
    Broken(StateIssue),
}

// 3. current state -> file ----------------------------------------------------
// Only entries that differ from the defaults are written.
fn build_state_file(
    layout: &CardLayout,
    column_widths: &[[Option<u16>; MAX_TABLE_COLUMNS]; CARD_COUNT],
    lang: Lang,
) -> StateFile {
    let mut file = StateFile {
        version: STATE_VERSION,
        language: Some(toml::Value::String(lang.code().to_string())),
        ..StateFile::default()
    };
    for group in SplitGroup::ALL {
        if let Some(order) = layout.custom_order(group) {
            let values = order.iter().map(|card| i64::from(*card)).collect();
            file.orders.insert(group.group_key().to_string(), values);
        }
        if let Some(ratios) = layout.custom_ratios(group) {
            let values = ratios.iter().map(|ratio| i64::from(*ratio)).collect();
            file.ratios.insert(group.group_key().to_string(), values);
        }
    }
    for card in CardId::ALL {
        let Some((key, columns)) = card.table_key() else {
            continue;
        };
        let widths = &column_widths[card as usize][..columns];
        if widths.iter().all(Option::is_none) {
            continue;
        }
        let values = widths
            .iter()
            .map(|width| i64::from(width.unwrap_or(0)))
            .collect();
        file.column_widths.insert(key.to_string(), values);
    }
    file
}

// 4. file -> current state ----------------------------------------------------
// Skips entries with an unknown name, wrong length, out-of-range value, or a
// width above the cap; returns how many were skipped.
fn apply_state_file(
    file: &StateFile,
    layout: &mut CardLayout,
    column_widths: &mut [[Option<u16>; MAX_TABLE_COLUMNS]; CARD_COUNT],
) -> usize {
    let mut skipped = 0usize;
    for (key, values) in &file.orders {
        let group = SplitGroup::ALL
            .iter()
            .find(|group| group.group_key() == key.as_str());
        match group {
            Some(group) if layout.restore_order(*group, values) => {}
            _ => skipped += 1,
        }
    }
    for (key, values) in &file.ratios {
        let group = SplitGroup::ALL
            .iter()
            .find(|group| group.group_key() == key.as_str());
        match group {
            Some(group) if layout.restore_ratios(*group, values) => {}
            _ => skipped += 1,
        }
    }
    for (key, values) in &file.column_widths {
        let card = CardId::ALL.iter().find(|card| {
            card.table_key()
                .is_some_and(|(name, _)| name == key.as_str())
        });
        let Some(card) = card else {
            skipped += 1;
            continue;
        };
        let columns = card.table_key().map_or(0, |(_, columns)| columns);
        let parsed: Option<Vec<Option<u16>>> = values
            .iter()
            .map(|value| match u16::try_from(*value) {
                Ok(0) => Some(None),
                Ok(width) if width <= MAX_COLUMN_WIDTH => Some(Some(width)),
                _ => None,
            })
            .collect();
        match parsed {
            Some(widths) if widths.len() == columns => {
                let slot = &mut column_widths[*card as usize];
                *slot = [None; MAX_TABLE_COLUMNS];
                slot[..columns].copy_from_slice(&widths);
            }
            _ => skipped += 1,
        }
    }
    skipped
}

// 5. file access --------------------------------------------------------------
fn read_state_file(path: &Path) -> StateRead {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return StateRead::Missing,
        Err(error) => return StateRead::Broken(StateIssue::Detail(error.to_string())),
    };
    match toml::from_str::<StateFile>(&text) {
        Ok(file) if file.version == STATE_VERSION => StateRead::Loaded(file),
        Ok(file) => StateRead::Broken(StateIssue::UnsupportedVersion(file.version)),
        Err(error) => StateRead::Broken(StateIssue::Detail(error.to_string())),
    }
}

// Copies an unreadable original to `tui.toml.bak` before the first save
// overwrites it (an older .bak is replaced, failures are ignored, a root
// process never writes through a foreign-owned link).
fn backup_state_file(path: &Path) {
    let Some(file_name) = path.file_name() else {
        return;
    };
    let mut backup_name = file_name.to_os_string();
    backup_name.push(BACKUP_SUFFIX);
    let backup = path.with_file_name(backup_name);
    let Ok(target) = resolve_write_target(&backup, root_guard().as_ref()) else {
        return;
    };
    let _ = fs::copy(path, target);
}

// Removes `tui.toml.<pid>.tmp` files an interrupted save left behind, next to
// the file a save writes: files only, failures ignored, and only those last
// modified at least STALE_TEMP_AGE before `now`, so a save another running
// instance is writing right now survives.
fn remove_stale_temp_files(path: &Path, now: SystemTime, guard: Option<&RootGuard>) {
    let Ok(target) = resolve_write_target(path, guard) else {
        return;
    };
    let (Some(dir), Some(file_name)) = (target.parent(), target.file_name()) else {
        return;
    };
    let prefix = format!("{}.", file_name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_suffix(TEMP_SUFFIX))
        else {
            continue;
        };
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        let is_stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age >= STALE_TEMP_AGE)
            });
        if is_file && is_stale && !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

// A new temp file that never follows a link planted at its name (O_EXCL); a
// leftover of the same name is replaced.
fn create_temp_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    match options.open(path) {
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            options.open(path)
        }
        other => other,
    }
}

// Temp file in the target's folder, synced, then renamed over the target; an
// interrupted write leaves the old file intact and a failure removes the temp.
// Runs on the state writer thread.
pub fn write_state_atomic(path: &Path, text: &str) -> io::Result<()> {
    write_state_atomic_with(path, text, root_guard().as_ref())
}

fn write_state_atomic_with(path: &Path, text: &str, guard: Option<&RootGuard>) -> io::Result<()> {
    let target = resolve_write_target(path, guard)?;
    let Some(dir) = target.parent() else {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "no parent directory",
        ));
    };
    fs::create_dir_all(dir)?;
    let file_name = target.file_name().map_or_else(
        || TUI_STATE_FILE_NAME.to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temp_path = dir.join(format!("{file_name}.{}{TEMP_SUFFIX}", std::process::id()));
    let written = create_temp_file(&temp_path).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temp_path, &target) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

impl Ui {
    // Restores once at start: a missing file keeps English and the default
    // layout; an unreadable or other-version file is backed up and ignored; a
    // bad entry is skipped alone. One toast reports a problem.
    pub fn load_state(&mut self) {
        let Some(state_file) = self.state_file.clone() else {
            return;
        };
        remove_stale_temp_files(&state_file, SystemTime::now(), root_guard().as_ref());
        match read_state_file(&state_file) {
            StateRead::Missing => {}
            StateRead::Broken(issue) => {
                backup_state_file(&state_file);
                let text = i18n::state_unreadable(self.lang, TUI_STATE_FILE_NAME, &issue);
                self.show_toast(&text, ToastLevel::Info);
            }
            StateRead::Loaded(file) => {
                let mut skipped =
                    apply_state_file(&file, &mut self.layout, &mut self.column_widths);
                match file
                    .language
                    .as_ref()
                    .map(|value| value.as_str().and_then(Lang::from_code))
                {
                    None => {}
                    Some(Some(lang)) => self.lang = lang,
                    Some(None) => skipped += 1,
                }
                if skipped > 0 {
                    let text = i18n::state_entries_skipped(self.lang, TUI_STATE_FILE_NAME, skipped);
                    self.show_toast(&text, ToastLevel::Info);
                }
            }
        }
    }

    // On a commit event: builds the whole file text and queues it for the
    // writer thread, so the UI thread never touches the disk. Without a state
    // file (no home directory) nothing happens; a failed build or a stopped
    // writer becomes an error toast, a failed write arrives later through
    // `state_save_failed`.
    pub fn request_state_save(&mut self, writer: &StateWriter) {
        let Some(path) = self.state_file.clone() else {
            return;
        };
        let file = build_state_file(&self.layout, &self.column_widths, self.lang);
        let queued = toml::to_string(&file)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                writer
                    .queue_state_save(StateSave { path, text })
                    .map_err(|_| "the state writer has stopped".to_string())
            });
        if let Err(detail) = queued {
            self.state_save_failed(&detail);
        }
    }

    // A failed save (reported by the writer thread): error toast in the active language.
    pub fn state_save_failed(&mut self, detail: &str) {
        let text = i18n::state_save_failed(self.lang, detail);
        self.show_toast(&text, ToastLevel::Error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::layout::IDENTITY_ORDER;
    use crate::tui::link_guard::RootGuard;
    use crate::tui::state_writer::spawn_state_writer;
    use std::sync::mpsc;

    fn ui_in(dir: &Path) -> Ui {
        Ui::new(Some(dir.join(TUI_STATE_FILE_NAME)))
    }

    // The host flow in one call: queue the save, flush the writer thread, then
    // turn a reported failure into the error toast.
    fn save_now(ui: &mut Ui) {
        let (report_tx, report_rx) = mpsc::channel::<String>();
        let mut writer = spawn_state_writer(move |detail| {
            let _ = report_tx.send(detail);
        })
        .unwrap();
        ui.request_state_save(&writer);
        writer.finish_state_saves(Duration::from_secs(5));
        while let Ok(detail) = report_rx.try_recv() {
            ui.state_save_failed(&detail);
        }
    }

    fn foreign_owner(_path: &Path) -> io::Result<u32> {
        Ok(1234)
    }

    fn customize(ui: &mut Ui) {
        ui.layout
            .move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
        assert!(ui.layout.restore_ratios(SplitGroup::CaptureRows, &[7_000]));
        assert!(ui.layout.restore_ratios(SplitGroup::InspectConn, &[6_000]));
        ui.column_widths[CardId::PacketList as usize][3] = Some(30);
        ui.column_widths[CardId::PacketList as usize][7] = Some(80);
        ui.column_widths[CardId::HistoryTable as usize][4] = Some(21);
    }

    #[test]
    fn state_path_sits_in_the_app_dir_next_to_config_toml() {
        let home = Path::new("injected-home");
        assert_eq!(
            state_path_in(Some(home)),
            Some(app_dir_in(home).join("tui.toml"))
        );
    }

    #[test]
    fn without_a_home_there_is_no_state_path_and_no_save_or_load_happens() {
        assert_eq!(state_path_in(None), None);
        let dir = tempfile::tempdir().unwrap();
        let mut ui = Ui::new(None);
        ui.load_state();
        assert!(ui.toast.is_none());
        let (report_tx, _report_rx) = mpsc::channel::<String>();
        let mut writer = spawn_state_writer(move |detail| {
            let _ = report_tx.send(detail);
        })
        .unwrap();
        ui.request_state_save(&writer);
        writer.finish_state_saves(Duration::from_secs(5));
        assert!(ui.toast.is_none());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn save_then_load_round_trips_language_orders_ratios_and_widths() {
        let dir = tempfile::tempdir().unwrap();
        let mut saved = ui_in(dir.path());
        customize(&mut saved);
        saved.lang = Lang::Ko;
        save_now(&mut saved);
        assert!(saved.toast.is_none(), "{:?}", saved.toast);

        let mut loaded = ui_in(dir.path());
        loaded.load_state();
        assert!(loaded.toast.is_none(), "{:?}", loaded.toast);
        assert_eq!(loaded.lang, Lang::Ko);
        assert_eq!(loaded.layout, saved.layout);
        assert_eq!(loaded.column_widths, saved.column_widths);
        assert_eq!(loaded.layout.order_of(SplitGroup::CaptureCols)[..2], [1, 0]);
    }

    #[test]
    fn file_text_is_stable_with_version_first_and_only_custom_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut ui = ui_in(dir.path());
        customize(&mut ui);
        save_now(&mut ui);
        let text = fs::read_to_string(dir.path().join(TUI_STATE_FILE_NAME)).unwrap();
        let expected = "version = 1\n\
            language = \"en\"\n\
            \n\
            [orders]\n\
            capture_cols = [1, 0]\n\
            \n\
            [ratios]\n\
            capture_rows = [7000]\n\
            inspect_conn = [6000]\n\
            \n\
            [column_widths]\n\
            history_table = [0, 0, 0, 0, 21]\n\
            packet_list = [0, 0, 0, 30, 0, 0, 0, 80]\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn a_default_layout_still_writes_a_loadable_file_with_version_and_language() {
        let dir = tempfile::tempdir().unwrap();
        let mut ui = ui_in(dir.path());
        save_now(&mut ui);
        let text = fs::read_to_string(dir.path().join(TUI_STATE_FILE_NAME)).unwrap();
        assert_eq!(text, "version = 1\nlanguage = \"en\"\n");
        let mut loaded = ui_in(dir.path());
        loaded.load_state();
        assert!(loaded.toast.is_none());
        assert_eq!(loaded.lang, Lang::En);
        assert_eq!(loaded.layout, CardLayout::default());
    }

    #[test]
    fn reset_then_save_overwrites_custom_entries_and_keeps_the_language() {
        let dir = tempfile::tempdir().unwrap();
        let mut ui = ui_in(dir.path());
        customize(&mut ui);
        ui.lang = Lang::Ko;
        save_now(&mut ui);
        ui.reset_layout(&SplitGroup::ALL, &CardId::ALL);
        save_now(&mut ui);
        let text = fs::read_to_string(dir.path().join(TUI_STATE_FILE_NAME)).unwrap();
        assert!(text.contains("language = \"ko\""), "{text}");
        assert!(!text.contains("capture_cols"), "{text}");
        assert!(!text.contains("packet_list"), "{text}");
    }

    #[test]
    fn a_missing_file_keeps_english_and_defaults_without_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let mut ui = ui_in(dir.path());
        ui.load_state();
        assert!(ui.toast.is_none());
        assert_eq!(ui.lang, Lang::En);
        assert!(!dir.path().join(TUI_STATE_FILE_NAME).exists());
    }

    #[test]
    fn an_unreadable_or_other_version_file_is_backed_up_ignored_and_reported_once() {
        for body in [
            "this is = = not toml [",
            "version = 2\n[orders]\ncapture_cols = [1, 0]\n",
            "orders = 5\n",
            "version = \"1\"\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(TUI_STATE_FILE_NAME);
            fs::write(&path, body).unwrap();
            let mut ui = ui_in(dir.path());
            ui.load_state();
            let toast = ui
                .toast
                .as_ref()
                .unwrap_or_else(|| panic!("no toast: {body}"));
            assert!(toast.text.contains("tui.toml"), "{}", toast.text);
            assert_eq!(ui.layout, CardLayout::default(), "{body}");
            let backup = dir.path().join("tui.toml.bak");
            assert_eq!(fs::read_to_string(backup).unwrap(), body);
            // The first save replaces the original but the backup stays.
            save_now(&mut ui);
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .starts_with("version = 1")
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("tui.toml.bak")).unwrap(),
                body
            );
        }
    }

    #[test]
    fn invalid_entries_are_skipped_while_valid_ones_apply() {
        let dir = tempfile::tempdir().unwrap();
        let body = "version = 1\n\
            language = \"fr\"\n\
            [orders]\n\
            capture_cols = [1, 0]\n\
            inspect_conn = [0, 1, 2]\n\
            inspect_proc = [1, 1]\n\
            capture_rows = [0, 1]\n\
            unknown_group = [0, 1]\n\
            [ratios]\n\
            inspect_alert = [2000]\n\
            inspect_conn = [6000, 1]\n\
            [column_widths]\n\
            packet_list = [0, 0, 0, 20, 0, 0, 0, 0]\n\
            conn_table = [0, 12]\n\
            history_table = [513, 0, 0, 0, 0]\n\
            proc_table = [-5, 0, 0, 0]\n\
            overview_chart = [1]\n";
        fs::write(dir.path().join(TUI_STATE_FILE_NAME), body).unwrap();
        let mut ui = ui_in(dir.path());
        ui.load_state();

        assert_eq!(
            ui.layout.custom_order(SplitGroup::CaptureCols),
            Some(vec![1, 0])
        );
        assert_eq!(
            ui.layout.custom_ratios(SplitGroup::InspectAlert),
            Some(vec![2_000])
        );
        assert_eq!(ui.column_widths[CardId::PacketList as usize][3], Some(20));
        assert_eq!(ui.layout.custom_order(SplitGroup::InspectConn), None);
        assert_eq!(ui.layout.custom_order(SplitGroup::InspectProc), None);
        assert_eq!(ui.layout.custom_order(SplitGroup::CaptureRows), None);
        assert_eq!(ui.layout.custom_ratios(SplitGroup::InspectConn), None);
        assert!(
            ui.column_widths[CardId::ConnTable as usize]
                .iter()
                .all(Option::is_none)
        );
        assert!(
            ui.column_widths[CardId::HistoryTable as usize]
                .iter()
                .all(Option::is_none)
        );
        assert!(
            ui.column_widths[CardId::ProcTable as usize]
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(ui.lang, Lang::En, "an unknown language is skipped");
        let toast = ui.toast.as_ref().unwrap();
        assert!(toast.text.contains("10"), "{}", toast.text);
    }

    #[test]
    fn column_widths_are_capped_at_512_and_zero_means_automatic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(TUI_STATE_FILE_NAME),
            "version = 1\n[column_widths]\nstat_talkers = [512, 0, 7]\nstat_flows = [513, 0, 0, 0, 0]\n",
        )
        .unwrap();
        let mut ui = ui_in(dir.path());
        ui.load_state();
        assert_eq!(
            ui.column_widths[CardId::StatTalkers as usize][..3],
            [Some(512), None, Some(7)]
        );
        assert!(
            ui.column_widths[CardId::StatFlows as usize]
                .iter()
                .all(Option::is_none)
        );
    }

    fn age_file(path: &Path, age: Duration) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn startup_removes_only_stale_temp_files_of_this_file() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("tui.toml.4242.tmp");
        let fresh = dir.path().join("tui.toml.4343.tmp");
        let unrelated = dir.path().join("other.toml.4242.tmp");
        let not_a_pid = dir.path().join("tui.toml.abc.tmp");
        for path in [&stale, &fresh, &unrelated, &not_a_pid] {
            fs::write(path, "x").unwrap();
        }
        let old = STALE_TEMP_AGE + Duration::from_secs(1);
        for path in [&stale, &unrelated, &not_a_pid] {
            age_file(path, old);
        }
        ui_in(dir.path()).load_state();
        assert!(!stale.exists(), "an old temp file of this state file goes");
        assert!(
            fresh.exists(),
            "a young temp file may be a save in progress"
        );
        assert!(unrelated.exists());
        assert!(not_a_pid.exists());
    }

    #[test]
    fn the_stale_age_is_judged_against_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TUI_STATE_FILE_NAME);
        let temp = dir.path().join("tui.toml.77.tmp");
        fs::write(&temp, "x").unwrap();
        let modified = fs::metadata(&temp).unwrap().modified().unwrap();
        remove_stale_temp_files(&path, modified + Duration::from_secs(59), None);
        assert!(temp.exists());
        remove_stale_temp_files(&path, modified + STALE_TEMP_AGE, None);
        assert!(!temp.exists());
    }

    #[test]
    fn a_failed_save_becomes_an_error_toast_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // The state path's parent is a regular file, so the save cannot succeed.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        let mut ui = Ui::new(Some(blocker.join(TUI_STATE_FILE_NAME)));
        save_now(&mut ui);
        let toast = ui.toast.as_ref().expect("error toast");
        assert_eq!(toast.level, ToastLevel::Error);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn make_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            Err(io::Error::new(ErrorKind::Unsupported, "no symlink support"))
        }
    }

    #[test]
    fn saving_through_a_symlink_keeps_the_link_and_updates_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let synced = dir.path().join("synced");
        let home = dir.path().join("home");
        fs::create_dir_all(&synced).unwrap();
        fs::create_dir_all(&home).unwrap();
        let target = synced.join("tui.toml");
        let link = home.join("tui.toml");
        fs::write(&target, "version = 1\nlanguage = \"en\"\n").unwrap();
        if let Err(error) = make_file_symlink(&target, &link) {
            eprintln!("skipped: the OS refused to create a symlink ({error})");
            return;
        }

        let mut ui = Ui::new(Some(link.clone()));
        ui.lang = Lang::Ko;
        ui.layout
            .move_card_to_slot(SplitGroup::CaptureCols, &IDENTITY_ORDER, 0, 1);
        save_now(&mut ui);
        assert!(ui.toast.is_none(), "{:?}", ui.toast);

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), target);
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("language = \"ko\""), "{content}");
        assert!(content.contains("capture_cols = [1, 0]"), "{content}");
        // The temp file lived next to the target and is gone.
        assert_eq!(fs::read_dir(&synced).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);

        let mut loaded = Ui::new(Some(link));
        loaded.load_state();
        assert_eq!(loaded.lang, Lang::Ko);
        assert_eq!(loaded.layout, ui.layout);
    }

    #[test]
    fn saving_through_a_dangling_symlink_creates_the_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let synced = dir.path().join("synced");
        let home = dir.path().join("home");
        fs::create_dir_all(&synced).unwrap();
        fs::create_dir_all(&home).unwrap();
        let target = synced.join("tui.toml");
        let link = home.join("tui.toml");
        if let Err(error) = make_file_symlink(&target, &link) {
            eprintln!("skipped: the OS refused to create a symlink ({error})");
            return;
        }
        let mut ui = Ui::new(Some(link.clone()));
        save_now(&mut ui);
        assert!(ui.toast.is_none(), "{:?}", ui.toast);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "version = 1\nlanguage = \"en\"\n"
        );
    }

    #[test]
    fn saving_through_a_two_level_link_chain_replaces_the_final_target() {
        let dir = tempfile::tempdir().unwrap();
        let synced = dir.path().join("synced");
        let home = dir.path().join("home");
        fs::create_dir_all(&synced).unwrap();
        fs::create_dir_all(&home).unwrap();
        let target = synced.join("tui.toml");
        let middle = synced.join("middle.toml");
        let link = home.join("tui.toml");
        fs::write(&target, "version = 1\nlanguage = \"en\"\n").unwrap();
        if make_file_symlink(&target, &middle).is_err()
            || make_file_symlink(&middle, &link).is_err()
        {
            eprintln!("skipped: the OS refused to create a symlink");
            return;
        }
        let mut ui = Ui::new(Some(link.clone()));
        ui.lang = Lang::Ko;
        save_now(&mut ui);
        assert!(ui.toast.is_none(), "{:?}", ui.toast);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(&middle)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("language = \"ko\""), "{content}");
    }

    #[test]
    fn a_root_save_refuses_a_foreign_owned_link_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.toml");
        let link = dir.path().join("tui.toml");
        fs::write(&target, "untouched").unwrap();
        if make_file_symlink(&target, &link).is_err() {
            eprintln!("skipped: the OS refused to create a symlink");
            return;
        }
        let guard = RootGuard {
            owner_of: &foreign_owner,
        };
        let error = write_state_atomic_with(&link, "version = 1\n", Some(&guard)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "no temp file is left: {names:?}");

        // Without the guard the same link is followed to its target.
        write_state_atomic_with(&link, "version = 1\n", None).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "version = 1\n");
    }

    #[test]
    fn the_temp_file_never_follows_a_link_planted_at_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, "keep").unwrap();
        let state = dir.path().join(TUI_STATE_FILE_NAME);
        let planted = dir
            .path()
            .join(format!("tui.toml.{}.tmp", std::process::id()));
        if make_file_symlink(&victim, &planted).is_err() {
            eprintln!("skipped: the OS refused to create a symlink");
            return;
        }
        write_state_atomic_with(&state, "version = 1\n", None).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(fs::read_to_string(&state).unwrap(), "version = 1\n");
    }
}
