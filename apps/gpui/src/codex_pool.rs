//! How the ChatGPT (Codex) account pool reads on the Models page: an
//! account's plan, and its usage meters (what part of the allowance is
//! used in a window, and when the window resets).
use crate::{
    i18n::{fill, tr},
    present::Tone,
};
use wisp_server::desktop::UsageWindow;

/// "plus" → "Plus".
pub fn plan_label(plan: &str) -> String {
    let mut chars = plan.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The window's length: whole days, else hours (at least one).
pub fn window_label(window_seconds: i64) -> String {
    match window_seconds {
        s if s > 0 && s % 86_400 == 0 => fill(tr("{}-day window"), &[&(s / 86_400)]),
        s if s > 0 => fill(tr("{}-hour window"), &[&((s + 1_799) / 3_600).max(1)]),
        _ => tr("Limit").to_string(),
    }
}

/// The part used, 0–100.
pub fn used(window: &UsageWindow) -> u32 {
    window.used_percent.round().clamp(0.0, 100.0) as u32
}

/// How a meter is coloured: nearly used up, then used up.
pub fn tone(window: &UsageWindow) -> Tone {
    match used(window) {
        100.. => Tone::Danger,
        80.. => Tone::Attention,
        _ => Tone::Neutral,
    }
}

/// "2d 3h", "4h 10m", "12m".
pub fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (days, hours, mins) = (
        seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
    );
    match (days, hours) {
        (0, 0) => format!("{mins}m"),
        (0, _) => format!("{hours}h {mins}m"),
        _ => format!("{days}d {hours}h"),
    }
}

/// "42% used · resets in 4h 10m", at `now` (Unix seconds).
pub fn meter_text(window: &UsageWindow, now: i64) -> String {
    let used = fill(tr("{}% used"), &[&used(window)]);
    if window.reset_at > 0 {
        let reset = fill(tr("resets in {}"), &[&duration(window.reset_at - now)]);
        format!("{used} · {reset}")
    } else {
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(used_percent: f64, window_seconds: i64, reset_at: i64) -> UsageWindow {
        UsageWindow {
            used_percent,
            window_seconds,
            reset_at,
        }
    }

    #[test]
    fn plans_read_capitalised() {
        assert_eq!(plan_label("plus"), "Plus");
        assert_eq!(plan_label(""), "");
    }

    #[test]
    fn windows_are_named_by_their_length() {
        assert_eq!(window_label(5 * 3_600), "5-hour window");
        assert_eq!(window_label(7 * 86_400), "7-day window");
        // A window just under an hour still reads as one.
        assert_eq!(window_label(600), "1-hour window");
        assert_eq!(window_label(0), "Limit");
    }

    #[test]
    fn meters_say_what_is_used_and_when_it_resets() {
        let now = 1_000_000;
        assert_eq!(
            meter_text(&window(41.6, 18_000, now + 4 * 3_600 + 600), now),
            "42% used · resets in 4h 10m"
        );
        assert_eq!(
            meter_text(&window(3.0, 604_800, now + 2 * 86_400 + 3 * 3_600), now),
            "3% used · resets in 2d 3h"
        );
        // No reset time, or one already past.
        assert_eq!(meter_text(&window(120.0, 18_000, 0), now), "100% used");
        assert_eq!(duration(-5), "0m");
    }

    #[test]
    fn meters_warn_when_nearly_used_up() {
        assert!(matches!(tone(&window(10.0, 1, 0)), Tone::Neutral));
        assert!(matches!(tone(&window(79.6, 1, 0)), Tone::Attention));
        assert!(matches!(tone(&window(100.0, 1, 0)), Tone::Danger));
    }
}
