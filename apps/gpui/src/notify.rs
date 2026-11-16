//! Telling the user that a conversation needs them while the app is in the
//! background: a system notification and, if wanted, a sound.
//!
//! Each system has its own way and none of them needs a library: macOS is
//! asked through `osascript` and `afplay`, Linux through `notify-send`.
//! Windows has no notification that can be posted without registering the
//! app with the system, so none is posted there; the choice in Settings
//! still plays the system's sound. What is run is built here, apart from
//! running it, so it is covered without posting anything.
use crate::i18n::{Locale, tr};

/// Why the user is being told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A turn ended on an answer.
    Done,
    /// A turn ended on an error.
    Failed,
    /// A tool is waiting for approval.
    Approval,
    /// A conversation that is not open stopped working and waits for you:
    /// an answer to read or a question to answer.
    NeedsYou,
    /// A Run moved to the background succeeded.
    RunDone,
    /// A Run moved to the background ended otherwise: failed, cancelled,
    /// timed out or lost.
    RunEnded,
}

/// What the user chose in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub banner: bool,
    pub sound: bool,
}

impl Default for Choice {
    /// The WebView desktop's defaults: told, silently.
    fn default() -> Self {
        Self {
            banner: true,
            sound: false,
        }
    }
}

impl Choice {
    /// Reads the two saved switches; a switch that was never saved keeps
    /// its default.
    pub fn from_saved(banner: Option<&str>, sound: Option<&str>) -> Self {
        let read = |saved: Option<&str>, default| match saved.map(str::trim) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        };
        let default = Self::default();
        Self {
            banner: read(banner, default.banner),
            sound: read(sound, default.sound),
        }
    }
}

/// Whether to tell the user at all: only when they are not looking at the
/// app, and only when they asked for some way of being told.
pub fn wanted(choice: Choice, window_active: bool) -> bool {
    !window_active && (choice.banner || choice.sound)
}

/// The notification's title for `event`.
pub fn title(event: Event) -> &'static str {
    match event {
        Event::Done => tr("Wisp has answered"),
        Event::Failed => tr("A turn stopped with an error"),
        Event::Approval => tr("Wisp needs your approval"),
        Event::NeedsYou => tr("A conversation needs you"),
        // The WebView's titles for the end of a background Run.
        Event::RunDone => tr("Task completed"),
        Event::RunEnded => tr("Task failed"),
    }
}

/// What to tell when a Run moved to the background has ended, as the
/// WebView tells it: the event, and the line that names the conversation
/// and the run.
pub fn run_ended(status: &str, conversation: &str, run: &str) -> (Event, String) {
    use crate::i18n::fill;
    let (event, line) = if status == "succeeded" {
        (Event::RunDone, tr("Background run finished: {}"))
    } else {
        (Event::RunEnded, tr("Background run ended: {}"))
    };
    let line = fill(line, &[&run]);
    let conversation = conversation.trim();
    if conversation.is_empty() {
        (event, line)
    } else {
        (event, format!("{conversation} · {line}"))
    }
}

/// Whether to tell that a background Run ended: never while its
/// conversation is on screen in the focused window, as in the WebView, and
/// only when the user asked for some way of being told. A window that is
/// focused but shows another conversation is told.
pub fn run_end_wanted(choice: Choice, on_screen: bool) -> bool {
    !on_screen && (choice.banner || choice.sound)
}

/// The notification's body: which conversation, cut to a line.
pub fn body(conversation: &str) -> String {
    let name = conversation
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = name.chars();
    let kept: String = chars.by_ref().take(80).collect();
    match (kept.is_empty(), chars.next().is_some()) {
        (true, _) => match crate::i18n::current() {
            Locale::En => "A conversation".into(),
            Locale::Zh => "某个会话".into(),
        },
        (false, true) => format!("{kept}…"),
        (false, false) => kept,
    }
}

/// What to tell about conversations other than the open one, from two
/// readings of the recent conversations: each that was working and has
/// stopped, with its title. One that now waits for you says so; one that
/// simply finished has answered.
pub fn elsewhere(
    before: &[wisp_dto::SessionSearchInfo],
    after: &[wisp_dto::SessionSearchInfo],
    open: Option<&str>,
) -> Vec<(Event, String)> {
    stopped(before, after, open)
        .into_iter()
        .map(|row| {
            let event = match row.status.as_str() {
                "needs_you" => Event::NeedsYou,
                _ => Event::Done,
            };
            (event, row.title.clone())
        })
        .collect()
}

/// The conversations other than the open one that were working at the
/// first reading and are not at the second.
pub fn stopped<'a>(
    before: &[wisp_dto::SessionSearchInfo],
    after: &'a [wisp_dto::SessionSearchInfo],
    open: Option<&str>,
) -> Vec<&'a wisp_dto::SessionSearchInfo> {
    after
        .iter()
        .filter(|row| Some(row.id.as_str()) != open && row.status != "running")
        .filter(|row| {
            before
                .iter()
                .any(|was| was.id == row.id && was.status == "running")
        })
        .collect()
}

/// The systems that are told apart here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Mac,
    Linux,
    Windows,
}

impl System {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            System::Mac
        } else if cfg!(target_os = "windows") {
            System::Windows
        } else {
            System::Linux
        }
    }
}

/// A program and what it is given.
pub type Command = (&'static str, Vec<String>);

/// The sounds a system offers, as the WebView desktop offers them, its
/// default first: the alert sounds macOS ships, freedesktop sound-theme
/// ids on Linux, and the sounds .NET plays by name on Windows. A saved
/// choice only ever names one of these, never a file or an argument.
pub fn sounds(system: System) -> &'static [&'static str] {
    match system {
        System::Mac => &[
            "Glass",
            "Basso",
            "Blow",
            "Bottle",
            "Frog",
            "Funk",
            "Hero",
            "Morse",
            "Ping",
            "Pop",
            "Purr",
            "Sosumi",
            "Submarine",
            "Tink",
        ],
        System::Linux => &[
            "complete",
            "message",
            "message-new-instant",
            "bell",
            "dialog-information",
            "dialog-warning",
        ],
        System::Windows => &["Asterisk", "Beep", "Exclamation", "Hand", "Question"],
    }
}

/// The sound to play for a saved choice: one the system has, else its
/// default, as a choice made on another system falls back.
pub fn sound_of(system: System, saved: Option<&str>) -> &'static str {
    let offered = sounds(system);
    saved
        .map(str::trim)
        .and_then(|saved| offered.iter().copied().find(|sound| *sound == saved))
        .unwrap_or(offered[0])
}

/// What is run to tell the user, in order. Title and body are handed over
/// as data: on macOS inside a quoted AppleScript string with its quotes
/// and backslashes escaped, elsewhere as arguments of their own.
#[cfg(test)]
pub fn commands(system: System, choice: Choice, title: &str, body: &str) -> Vec<Command> {
    commands_with(system, choice, sound_of(system, None), title, body)
}

/// [`commands`] with the sound chosen in Settings, one of [`sounds`].
pub fn commands_with(
    system: System,
    choice: Choice,
    sound: &str,
    title: &str,
    body: &str,
) -> Vec<Command> {
    let sound = sound_of(system, Some(sound));
    let mut run = Vec::new();
    match system {
        System::Mac => {
            if choice.banner {
                let quoted = |text: &str| {
                    format!(
                        "\"{}\"",
                        text.replace('\\', "\\\\")
                            .replace('"', "\\\"")
                            .replace(['\n', '\r'], " ")
                    )
                };
                run.push((
                    "/usr/bin/osascript",
                    vec![
                        "-e".into(),
                        format!(
                            "display notification {} with title {}",
                            quoted(body),
                            quoted(title)
                        ),
                    ],
                ));
            }
            if choice.sound {
                run.push((
                    "/usr/bin/afplay",
                    vec![format!("/System/Library/Sounds/{sound}.aiff")],
                ));
            }
        }
        System::Linux => {
            if choice.banner {
                run.push((
                    "notify-send",
                    vec![
                        "--app-name=Wisp".into(),
                        "--".into(),
                        title.into(),
                        body.into(),
                    ],
                ));
            }
            if choice.sound {
                run.push((
                    "canberra-gtk-play",
                    vec![format!("--id={sound}"), "--description=Wisp".into()],
                ));
            }
        }
        System::Windows => {
            if choice.sound {
                run.push((
                    "powershell.exe",
                    vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        format!(
                            "[System.Media.SystemSounds]::{sound}.Play(); Start-Sleep -Milliseconds 600"
                        ),
                    ],
                ));
            }
        }
    }
    run
}

/// Tells the user. A system without the program simply does not tell; the
/// app goes on either way.
pub fn post(choice: Choice, sound: &str, event: Event, conversation: &str) {
    post_text(choice, sound, title(event), &body(conversation));
}

/// Tells the user that a Run moved to the background ended with `status`,
/// naming its conversation and the run.
pub fn post_run_end(choice: Choice, sound: &str, status: &str, conversation: &str, run: &str) {
    let (event, line) = run_ended(status, conversation, run);
    post_text(choice, sound, title(event), &body(&line));
}

/// [`post`] with the body as it is to be shown.
fn post_text(choice: Choice, sound: &str, title: &str, body: &str) {
    for (program, arguments) in commands_with(System::current(), choice, sound, title, body) {
        let _ = std::process::Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_that_is_not_open_is_told_about_when_it_stops() {
        let row = |id: &str, status: &str| -> wisp_dto::SessionSearchInfo {
            serde_json::from_value(serde_json::json!({
                "id": id, "project_id": "p", "project_name": "Lab",
                "title": format!("About {id}"), "status": status
            }))
            .unwrap()
        };
        let before = [
            row("a", "running"),
            row("b", "running"),
            row("c", "running"),
            row("d", "complete"),
        ];
        let after = [
            row("a", "needs_you"),
            row("b", "complete"),
            row("c", "running"),
            row("d", "needs_you"),
            row("e", "complete"),
        ];
        assert_eq!(
            elsewhere(&before, &after, None),
            [
                (Event::NeedsYou, "About a".to_string()),
                (Event::Done, "About b".to_string()),
            ]
        );
        // The open conversation is told about on its own.
        assert_eq!(
            elsewhere(&before, &after, Some("a")),
            [(Event::Done, "About b".to_string())]
        );
        assert!(elsewhere(&[], &after, None).is_empty());
        assert_eq!(title(Event::NeedsYou), "A conversation needs you");
    }

    const BOTH: Choice = Choice {
        banner: true,
        sound: true,
    };

    #[test]
    fn the_user_is_told_only_when_away_and_only_as_they_chose() {
        let quiet = Choice {
            banner: false,
            sound: false,
        };
        assert!(wanted(Choice::default(), false));
        assert!(!wanted(Choice::default(), true));
        assert!(!wanted(quiet, false));
        assert!(wanted(
            Choice {
                banner: false,
                sound: true
            },
            false
        ));
        // Saved switches are read back; anything else keeps the default.
        assert_eq!(Choice::from_saved(None, None), Choice::default());
        assert_eq!(
            Choice::from_saved(Some("false"), Some(" true ")),
            Choice {
                banner: false,
                sound: true
            }
        );
        assert_eq!(
            Choice::from_saved(Some("maybe"), Some("")),
            Choice::default()
        );
    }

    #[test]
    fn the_chosen_sound_is_played_and_only_a_known_one() {
        let chosen = |system, sound| {
            commands_with(
                system,
                Choice {
                    banner: false,
                    sound: true,
                },
                sound,
                "t",
                "b",
            )
            .remove(0)
        };
        assert_eq!(
            chosen(System::Mac, "Submarine").1,
            ["/System/Library/Sounds/Submarine.aiff"]
        );
        // A name the system does not have, or a path, falls back to its
        // default rather than reaching the player.
        assert_eq!(
            chosen(System::Mac, "../../etc/passwd").1,
            ["/System/Library/Sounds/Glass.aiff"]
        );
        assert_eq!(chosen(System::Linux, "bell").1[0], "--id=bell");
        assert_eq!(chosen(System::Linux, "Glass").1[0], "--id=complete");
        assert!(chosen(System::Windows, "Exclamation").1[3].contains("::Exclamation.Play()"));
        assert_eq!(sound_of(System::Mac, None), "Glass");
        assert_eq!(sound_of(System::Mac, Some(" Ping ")), "Ping");
        for system in [System::Mac, System::Linux, System::Windows] {
            assert!(!sounds(system).is_empty());
        }
    }

    #[test]
    fn the_end_of_a_background_run_is_told_as_the_webview_tells_it() {
        assert_eq!(
            run_ended("succeeded", " Count the reads ", "Align"),
            (
                Event::RunDone,
                "Count the reads · Background run finished: Align".to_string()
            )
        );
        for status in ["failed", "cancelled", "timed_out", "lost"] {
            assert_eq!(
                run_ended(status, "", "Align"),
                (Event::RunEnded, "Background run ended: Align".to_string()),
                "{status}"
            );
        }
        assert_eq!(title(Event::RunDone), "Task completed");
        assert_eq!(title(Event::RunEnded), "Task failed");
        // Not while its conversation is on screen in the focused window,
        // and only as the user chose.
        assert!(run_end_wanted(Choice::default(), false));
        assert!(!run_end_wanted(Choice::default(), true));
        let quiet = Choice {
            banner: false,
            sound: false,
        };
        assert!(!run_end_wanted(quiet, false));
    }

    #[test]
    fn the_body_names_the_conversation_on_one_short_line() {
        assert_eq!(body("  count\nthe   reads "), "count the reads");
        assert_eq!(body(&"x".repeat(100)), format!("{}…", "x".repeat(80)));
        assert_eq!(body("  "), "A conversation");
    }

    #[test]
    fn a_title_with_quotes_cannot_end_the_script_it_is_quoted_in() {
        let run = commands(
            System::Mac,
            BOTH,
            "Wisp has answered",
            "say \"hi\" \\ then\ndisplay dialog \"x\"",
        );
        assert_eq!(run.len(), 2);
        assert_eq!(run[0].0, "/usr/bin/osascript");
        assert_eq!(
            run[0].1,
            [
                "-e",
                "display notification \"say \\\"hi\\\" \\\\ then display dialog \\\"x\\\"\" \
                 with title \"Wisp has answered\""
            ]
        );
        assert_eq!(run[1].0, "/usr/bin/afplay");
        // Only what was chosen is run.
        let silent = commands(System::Mac, Choice::default(), "t", "b");
        assert_eq!(silent.len(), 1);
        let none = commands(
            System::Mac,
            Choice {
                banner: false,
                sound: false,
            },
            "t",
            "b",
        );
        assert!(none.is_empty());
    }

    #[test]
    fn other_systems_are_told_their_own_way_or_not_at_all() {
        let linux = commands(System::Linux, BOTH, "-t", "--body");
        // After `--`, a title that begins with a dash is still a title.
        assert_eq!(linux[0].0, "notify-send");
        assert_eq!(linux[0].1, ["--app-name=Wisp", "--", "-t", "--body"]);
        assert_eq!(linux[1].0, "canberra-gtk-play");
        // Windows has a sound and no banner.
        let windows = commands(System::Windows, BOTH, "t", "b");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0, "powershell.exe");
        assert!(commands(System::Windows, Choice::default(), "t", "b").is_empty());
    }
}
