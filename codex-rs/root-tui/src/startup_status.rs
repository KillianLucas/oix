//! A tiny, self-contained animated "shimmer" status line drawn directly to
//! stderr while the local daemon is cold-starting.
//!
//! The real TUI shimmer (see `codex-tui`'s `shimmer.rs`) is driven by the
//! ratatui frame loop, which is not running yet at this point in startup: the
//! daemon has to be reachable *before* the TUI takes over the screen. So this
//! module reimplements the same moving cosine highlight band over a short
//! string, animated on a background thread, and clears itself once the daemon
//! is ready.

use std::io::IsTerminal;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

/// The animated label, including the leading bullet that the TUI's
/// "Interpreting" indicator also shimmers.
const LABEL: &str = "\u{2022} Starting Open Interpreter\u{2026}";

/// Frame interval — matches the TUI status indicator's 32ms cadence.
const FRAME: Duration = Duration::from_millis(32);

/// A running shimmer animation. Call [`StartupStatus::finish`] once the daemon
/// is ready to stop the animation and clear the line.
pub(crate) struct StartupStatus {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StartupStatus {
    /// Begin animating the startup line. Returns `None` (and draws nothing) when
    /// stderr is not an interactive terminal — e.g. piped or redirected output.
    pub(crate) fn start() -> Option<Self> {
        if !std::io::stderr().is_terminal() {
            return None;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || animate(&stop_for_thread));
        Some(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Stop the animation, join the render thread, and clear the line so the
    /// terminal is clean before the TUI (or any subsequent output) takes over.
    pub(crate) fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        let mut stderr = std::io::stderr().lock();
        // Clear the animated line, then leave two blank lines of breathing room.
        let _ = write!(stderr, "\r\x1b[2K\n\n");
        let _ = stderr.flush();
    }
}

fn animate(stop: &AtomicBool) {
    let truecolor = supports_truecolor();
    let start = Instant::now();

    let mut stderr = std::io::stderr().lock();
    // Two newlines of separation above the status line, drawn once.
    let _ = write!(stderr, "\n\n");

    while !stop.load(Ordering::Relaxed) {
        let frame = render_frame(start.elapsed(), truecolor);
        // Carriage return + clear-to-end-of-line, then the frame.
        let _ = write!(stderr, "\r\x1b[2K{frame}");
        let _ = stderr.flush();
        thread::sleep(FRAME);
    }
}

/// Render a single frame of the shimmer as an ANSI-styled string.
///
/// Mirrors the math in `codex-tui::shimmer::shimmer_spans`: a cosine-shaped
/// highlight band of `band_half_width` sweeps across the characters once every
/// `sweep_seconds`, brightening the base color toward a highlight color.
fn render_frame(elapsed: Duration, truecolor: bool) -> String {
    let chars: Vec<char> = LABEL.chars().collect();
    let padding = 10usize;
    let period = chars.len() + padding * 2;
    let sweep_seconds = 2.0f32;
    let pos = ((elapsed.as_secs_f32() % sweep_seconds) / sweep_seconds * period as f32) as isize;
    let band_half_width = 5.0f32;

    // Dim gray base that brightens toward near-white at the band's peak.
    let base = (130u8, 130u8, 130u8);
    let highlight = (235u8, 235u8, 235u8);

    let mut out = String::with_capacity(chars.len() * 20);
    for (i, ch) in chars.iter().enumerate() {
        let dist = ((i as isize + padding as isize) - pos).abs() as f32;
        let t = if dist <= band_half_width {
            0.5 * (1.0 + (std::f32::consts::PI * (dist / band_half_width)).cos())
        } else {
            0.0
        };

        if truecolor {
            let (r, g, b) = lerp(base, highlight, t.clamp(0.0, 1.0) * 0.9);
            out.push_str(&format!("\x1b[1;38;2;{r};{g};{b}m{ch}"));
        } else if t > 0.6 {
            out.push_str(&format!("\x1b[1m{ch}\x1b[22m"));
        } else if t < 0.2 {
            out.push_str(&format!("\x1b[2m{ch}\x1b[22m"));
        } else {
            out.push(*ch);
        }
    }
    out.push_str("\x1b[0m");
    out
}

fn lerp(from: (u8, u8, u8), to: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
}
