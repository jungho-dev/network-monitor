//! Terminal lifecycle and the shared event loop: raw mode, alternate screen,
//! mouse capture, and cursor are entered together and restored on every exit
//! path (Drop guard plus panic hook); the loop redraws on change or after an
//! idle interval and wakes for timers.

use std::io::{self, Stdout};
use std::panic;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEvent,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

// Longest wait for input, and the redraw interval when nothing changed.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(50);
pub const IDLE_REDRAW_INTERVAL: Duration = Duration::from_secs(1);

// 1. restore ------------------------------------------------------------------
// Safe to call repeatedly; failures are ignored because the shell must come back.
pub fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    );
}

// Restores on normal exit, `?` returns, and unwinding.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

// Enters the screen after installing the panic hook that restores it before
// the message prints.
pub fn enter_terminal(
    title: &str,
) -> anyhow::Result<(TerminalGuard, Terminal<CrosstermBackend<Stdout>>)> {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));
    enable_raw_mode().context("enabling raw mode failed")?;
    let guard = TerminalGuard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        Hide,
        SetTitle(title)
    )
    .context("entering the alternate screen failed")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("initializing the terminal failed")?;
    terminal.clear()?;
    Ok((guard, terminal))
}

// 2. host contract ------------------------------------------------------------
// What one TUI supplies to the shared loop. Every method returns quickly; I/O
// belongs to background threads that report through channels drained in `on_tick`.
pub trait TuiHost {
    fn draw(&mut self, frame: &mut Frame);
    // Background messages, timers, and caches; true when the screen changed.
    fn on_tick(&mut self, now: Instant) -> bool;
    // Longest input wait: `base`, shortened to the next timer.
    fn next_wake(&self, now: Instant, base: Duration) -> Duration;
    fn on_key(&mut self, key: KeyEvent) -> bool;
    fn on_mouse(&mut self, mouse: MouseEvent, now: Instant) -> bool;
    fn on_resize(&mut self);
    fn should_quit(&self) -> bool;
}

// Windows reports both press and release for every key; only a non-release acts.
pub fn accepts_key(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
}

// 3. event loop ---------------------------------------------------------------
pub fn run_event_loop<H: TuiHost>(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    host: &mut H,
) -> anyhow::Result<()> {
    let mut dirty = true;
    let mut last_draw = Instant::now();
    while !host.should_quit() {
        // 1. timers and background messages
        dirty |= host.on_tick(Instant::now());

        // 2. draw when changed or after the idle interval
        if dirty || last_draw.elapsed() >= IDLE_REDRAW_INTERVAL {
            terminal.draw(|frame| host.draw(frame))?;
            dirty = false;
            last_draw = Instant::now();
        }

        // 3. wait for input, no longer than the next timer
        let wait = host.next_wake(Instant::now(), FRAME_INTERVAL);
        if !event::poll(wait)? {
            continue;
        }

        // 4. drain queued input before the next draw
        loop {
            match event::read()? {
                Event::Key(key) if accepts_key(&key) => dirty |= host.on_key(key),
                Event::Mouse(mouse) => dirty |= host.on_mouse(mouse, Instant::now()),
                Event::Resize(_, _) => {
                    host.on_resize();
                    dirty = true;
                }
                _ => {}
            }
            if host.should_quit() || !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn key_release_is_ignored_while_press_and_repeat_act() {
        for (kind, expected) in [
            (KeyEventKind::Press, true),
            (KeyEventKind::Repeat, true),
            (KeyEventKind::Release, false),
        ] {
            let key = KeyEvent::new_with_kind(KeyCode::Char('q'), KeyModifiers::NONE, kind);
            assert_eq!(accepts_key(&key), expected, "{kind:?}");
        }
    }

    #[test]
    fn restore_is_idempotent_and_never_panics_without_a_terminal() {
        restore_terminal();
        restore_terminal();
        let _guard = TerminalGuard;
    }

    #[test]
    fn timers_wake_the_loop_no_later_than_their_deadline() {
        struct Probe {
            deadline: Duration,
        }
        impl TuiHost for Probe {
            fn draw(&mut self, _frame: &mut Frame) {}
            fn on_tick(&mut self, _now: Instant) -> bool {
                false
            }
            fn next_wake(&self, _now: Instant, base: Duration) -> Duration {
                base.min(self.deadline)
            }
            fn on_key(&mut self, _key: KeyEvent) -> bool {
                false
            }
            fn on_mouse(&mut self, _mouse: MouseEvent, _now: Instant) -> bool {
                false
            }
            fn on_resize(&mut self) {}
            fn should_quit(&self) -> bool {
                false
            }
        }
        let probe = Probe {
            deadline: Duration::from_millis(7),
        };
        assert_eq!(
            probe.next_wake(Instant::now(), FRAME_INTERVAL),
            Duration::from_millis(7)
        );
        assert_eq!(
            Probe {
                deadline: Duration::from_secs(9)
            }
            .next_wake(Instant::now(), FRAME_INTERVAL),
            FRAME_INTERVAL
        );
    }
}
