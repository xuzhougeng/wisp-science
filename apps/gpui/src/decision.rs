//! The decision model form: what each service is called, what the form
//! holds when a service is picked, and how its state reads.
use wisp_dto::{DecisionModelProbe, DecisionModelSettings, DecisionPreset};

use crate::i18n::{fill, tr};

/// A service as the form names it. The names are the services' own.
pub fn preset_label(id: &str) -> &'static str {
    match id {
        "typesafe" => "TypeSafe Jev",
        "startlux_bamboo" => "StartLux (Bamboo)",
        "startlux_local" => tr("StartLux (self-hosted)"),
        _ => tr("Custom server"),
    }
}

pub fn preset_hint(id: &str) -> &'static str {
    match id {
        "typesafe" => tr("TypeSafe's hosted Jev models. Needs an API key."),
        "startlux_bamboo" => tr(
            "StartLux-Decision 27B AutoThink hosted on Bamboo. Needs a Bamboo API key. It \
             thinks when unsure, which can take much longer.",
        ),
        "startlux_local" => tr(
            "A server started with startlux-decide serve on this machine or your network. \
             Needs an API key only if it was started with one.",
        ),
        _ => tr(
            "Any server that implements POST /v1/systemone. The base URL may be the root, \
             /v1, or the full path.",
        ),
    }
}

/// How thinking is named on its button; empty keeps the server's own way.
pub fn think_label(mode: &str) -> &'static str {
    match mode {
        "auto" => tr("When unsure"),
        "never" => tr("Never"),
        "always" => tr("Always"),
        _ => tr("Server default"),
    }
}

/// Whether the saved decision model is in use, in a sentence.
pub fn status(saved: &DecisionModelSettings) -> String {
    let service = preset_label(&saved.preset);
    let needs_key = DecisionPreset::find(&saved.preset).is_some_and(|preset| preset.key_required);
    if !saved.enabled {
        tr("Off. The model's map_items tool offers no decide step.").to_owned()
    } else if needs_key && !saved.has_key(&saved.preset) {
        fill(
            tr("{} is on but has no saved API key, so it is not in use."),
            &[&service],
        )
    } else if saved.model.is_empty() {
        fill(tr("In use: {}"), &[&service])
    } else {
        fill(tr("In use: {} · {}"), &[&service, &saved.model])
    }
}

/// The text fields, in the order the form shows them: base URL, model,
/// timeout in seconds.
pub fn values(settings: &DecisionModelSettings) -> [String; 3] {
    [
        settings.base_url.clone(),
        settings.model.clone(),
        settings.timeout_secs.to_string(),
    ]
}

/// The draft once `preset` is picked: the service's own address, model
/// and timeout; whether it is on and how it thinks stay as they were.
pub fn picked(draft: &DecisionModelSettings, preset: &DecisionPreset) -> DecisionModelSettings {
    DecisionModelSettings {
        enabled: draft.enabled,
        think_mode: draft.think_mode.clone(),
        stored_keys: draft.stored_keys.clone(),
        ..DecisionModelSettings::from_preset(preset)
    }
}

/// What the form sends: the draft's switches with what was typed. A
/// timeout that is no number is sent as none, and the host takes the
/// service's own.
pub fn form(draft: &DecisionModelSettings, typed: [String; 3]) -> DecisionModelSettings {
    let [base_url, model, timeout] = typed;
    DecisionModelSettings {
        base_url: base_url.trim().to_owned(),
        model: model.trim().to_owned(),
        timeout_secs: timeout.trim().parse().unwrap_or(0),
        stored_keys: Vec::new(),
        ..draft.clone()
    }
}

/// Whether saving would change anything: the form differs from what is
/// kept, or a key was typed.
pub fn changed(form: &DecisionModelSettings, saved: &DecisionModelSettings, key: &str) -> bool {
    let kept = DecisionModelSettings {
        stored_keys: Vec::new(),
        ..saved.clone()
    };
    form != &kept || !key.trim().is_empty()
}

/// What a passed test says.
pub fn probe_note(probe: &DecisionModelProbe) -> String {
    let model = if probe.model.is_empty() {
        tr("the server's model")
    } else {
        probe.model.as_str()
    };
    fill(tr("{} answered in {} ms."), &[&model, &probe.elapsed_ms])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(id: &str) -> &'static DecisionPreset {
        DecisionPreset::find(id).unwrap()
    }

    #[test]
    fn the_state_says_whether_it_is_really_in_use() {
        let mut saved = DecisionModelSettings::from_preset(preset("typesafe"));
        assert!(status(&saved).starts_with("Off."));
        saved.enabled = true;
        assert_eq!(
            status(&saved),
            "TypeSafe Jev is on but has no saved API key, so it is not in use."
        );
        saved.stored_keys = vec!["typesafe".into()];
        assert_eq!(status(&saved), "In use: TypeSafe Jev · jev-latest");
        // A self-hosted server needs no key and may name no model.
        let local = DecisionModelSettings {
            enabled: true,
            ..DecisionModelSettings::from_preset(preset("startlux_local"))
        };
        assert_eq!(status(&local), "In use: StartLux (self-hosted)");
    }

    #[test]
    fn picking_a_service_fills_its_own_values_and_keeps_the_switches() {
        let draft = DecisionModelSettings {
            enabled: true,
            think_mode: "never".into(),
            stored_keys: vec!["typesafe".into()],
            ..DecisionModelSettings::from_preset(preset("typesafe"))
        };
        let next = picked(&draft, preset("startlux_bamboo"));
        assert_eq!(next.preset, "startlux_bamboo");
        assert_eq!(
            values(&next),
            [
                "https://api.bamboonode.dev".to_owned(),
                "startlux-decision-27b-autothink".to_owned(),
                "60".to_owned()
            ]
        );
        assert!(next.enabled && next.think_mode == "never");
        assert_eq!(next.stored_keys, ["typesafe"]);
    }

    #[test]
    fn the_form_is_what_was_typed_and_knows_when_it_differs() {
        let saved = DecisionModelSettings {
            stored_keys: vec!["typesafe".into()],
            ..DecisionModelSettings::from_preset(preset("typesafe"))
        };
        let same = form(&saved, values(&saved));
        assert!(same.stored_keys.is_empty());
        assert!(!changed(&same, &saved, "  "));
        assert!(changed(&same, &saved, "sk-made-up"));
        let typed = form(
            &saved,
            [" http://lab:8090 ".into(), " jev-2 ".into(), "soon".into()],
        );
        assert_eq!(
            (
                typed.base_url.as_str(),
                typed.model.as_str(),
                typed.timeout_secs
            ),
            ("http://lab:8090", "jev-2", 0)
        );
        assert!(changed(&typed, &saved, ""));
    }

    #[test]
    fn a_passed_test_names_who_answered_and_how_fast() {
        let probe = DecisionModelProbe {
            model: "jev-1.13.0".into(),
            elapsed_ms: 42,
        };
        assert_eq!(probe_note(&probe), "jev-1.13.0 answered in 42 ms.");
        assert_eq!(
            probe_note(&DecisionModelProbe::default()),
            "the server's model answered in 0 ms."
        );
        assert_eq!(think_label(""), "Server default");
        assert_eq!(think_label("auto"), "When unsure");
    }
}
