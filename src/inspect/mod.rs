//! The inspector TUI: attaches to a running daemon over its Unix socket and
//! shows live connections, per-process and per-domain breakdowns, the alerts
//! feed, and recent history. It shares the capture TUI's theme, language
//! catalog, layout, state file, interaction, and chrome. The state and views
//! build on every platform (they hold plain rows); only the socket glue is Unix.

pub mod state;
#[cfg(test)]
mod tests;
pub mod views;

#[cfg(unix)]
mod glue;
#[cfg(unix)]
mod restart;
#[cfg(unix)]
pub use glue::run_inspector;
#[cfg(unix)]
pub use restart::restart_daemon_and_inspector;
