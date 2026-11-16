//! The first-run guide: what it shows on each step and what its main
//! button says.
use crate::i18n::tr;

/// Welcome, what Wisp can do, a model, the tools on this computer.
pub const STEPS: usize = 4;
/// The step that sets up a model.
pub const MODEL: usize = 2;
/// Where a DeepSeek key is made; the guide sets DeepSeek up, as the
/// WebView's does. Other providers are added in Settings.
pub const DEEPSEEK_KEYS: &str = "https://platform.deepseek.com/api_keys";

pub fn title(step: usize) -> &'static str {
    match step {
        0 => tr("Welcome to Wisp Science"),
        1 => tr("What Wisp can do"),
        MODEL => tr("Set up your model"),
        _ => tr("Tools on this computer"),
    }
}

/// The main button: on through the steps, "later" when the model step is
/// left empty, and a start on the last.
pub fn next_label(step: usize, key_typed: bool, model_settable: bool) -> &'static str {
    if step + 1 >= STEPS {
        tr("Get started")
    } else if step == MODEL && model_settable && !key_typed {
        tr("Set up later")
    } else {
        tr("Next")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_button_says_what_pressing_it_does() {
        assert_eq!(next_label(0, false, true), "Next");
        assert_eq!(next_label(MODEL, false, true), "Set up later");
        assert_eq!(next_label(MODEL, true, true), "Next");
        // With the model set by the environment there is nothing to put off.
        assert_eq!(next_label(MODEL, false, false), "Next");
        assert_eq!(next_label(STEPS - 1, false, true), "Get started");
        assert_eq!(title(MODEL), "Set up your model");
        assert_eq!(title(STEPS - 1), "Tools on this computer");
    }
}
