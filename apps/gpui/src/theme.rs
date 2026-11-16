//! Wisp's palette for the component library, light and dark. The values are
//! the WebView's design tokens (`ui/src/styles/base.css`), so both frontends
//! read as the same product without sharing a renderer.
use std::rc::Rc;

use gpui::{App, SharedString, Window};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeSet};

/// Which palette the window wears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Whatever the system is set to, changing with it.
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// The word it is saved as.
    pub fn key(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    /// Reads a saved choice; anything else follows the system.
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key.trim())
            .unwrap_or_default()
    }
}

/// Wears the chosen palette: the system's, or one of the two for good.
pub fn apply(appearance: Appearance, window: &mut Window, cx: &mut App) {
    match appearance {
        Appearance::System => Theme::sync_system_appearance(Some(window), cx),
        Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
}

/// One of the WebView's palettes: its accent, page and text colours, laid
/// over Wisp's theme. The first of each kind is Wisp's own, left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub key: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub accent: &'static str,
    pub background: &'static str,
    pub foreground: &'static str,
}

const fn palette(
    key: &'static str,
    name: &'static str,
    dark: bool,
    [accent, background, foreground]: [&'static str; 3],
) -> Palette {
    Palette {
        key,
        name,
        dark,
        accent,
        background,
        foreground,
    }
}

/// The WebView's palettes, light then dark, with its colours.
pub const PALETTES: [Palette; 10] = [
    palette(
        "paper",
        "Wisp Paper",
        false,
        ["#0D9488", "#FAF9F6", "#141413"],
    ),
    palette("codex", "Codex", false, ["#2563EB", "#F4F6F8", "#172033"]),
    palette("github", "GitHub", false, ["#0969DA", "#F6F8FA", "#1F2328"]),
    palette(
        "catppuccin",
        "Catppuccin Latte",
        false,
        ["#8839EF", "#EFF1F5", "#4C4F69"],
    ),
    palette(
        "everforest",
        "Everforest",
        false,
        ["#3A8F6B", "#F4F0D9", "#2F383E"],
    ),
    palette(
        "charcoal",
        "Wisp Charcoal",
        true,
        ["#2DA898", "#171614", "#EBE8E2"],
    ),
    palette("codex", "Codex", true, ["#7C8CFF", "#202123", "#F3F4F6"]),
    palette(
        "github",
        "GitHub Dark",
        true,
        ["#58A6FF", "#0D1117", "#F0F6FC"],
    ),
    palette(
        "catppuccin",
        "Catppuccin Mocha",
        true,
        ["#CBA6F7", "#1E1E2E", "#CDD6F4"],
    ),
    palette(
        "gruvbox",
        "Gruvbox",
        true,
        ["#D79921", "#282828", "#EBDBB2"],
    ),
];

/// The palettes of one kind, Wisp's first.
pub fn palettes(dark: bool) -> impl Iterator<Item = Palette> {
    PALETTES
        .into_iter()
        .filter(move |palette| palette.dark == dark)
}

/// A saved palette of one kind; anything else is Wisp's own.
pub fn palette_of(dark: bool, key: &str) -> Palette {
    palettes(dark)
        .find(|palette| palette.key == key.trim())
        .unwrap_or_else(|| palettes(dark).next().expect("each kind has palettes"))
}

/// How the window looks beyond light or dark: the palette of each kind and
/// the sizes and faces of its text. A size of 0 or an empty face keeps
/// Wisp's own.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub light: Palette,
    pub dark: Palette,
    pub text_size: f32,
    pub text_font: String,
    pub code_size: f32,
    pub code_font: String,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            light: palette_of(false, ""),
            dark: palette_of(true, ""),
            text_size: 0.,
            text_font: String::new(),
            code_size: 0.,
            code_font: String::new(),
        }
    }
}

/// Text sizes offered, in pixels; the WebView's slider goes as far.
pub const TEXT_SIZES: std::ops::RangeInclusive<f32> = 10.0..=24.0;

/// `#rrggbb` a share `amount` of the way from `from` to `to`.
pub fn mix(from: &str, to: &str, amount: f32) -> String {
    let channels = |hex: &str| {
        let hex = hex.trim_start_matches('#');
        [0, 2, 4].map(|at| u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0))
    };
    let (from, to) = (channels(from), channels(to));
    let amount = amount.clamp(0., 1.);
    let mixed: Vec<String> = (0..3)
        .map(|at| {
            let value = from[at] as f32 + (to[at] as f32 - from[at] as f32) * amount;
            format!("{:02X}", value.round() as u8)
        })
        .collect();
    format!("#{}", mixed.concat())
}

/// Wisp's theme of one kind wearing `look`: the palette's accent on what
/// is pressed, chosen or linked, its page colour on the surfaces, a shade
/// of it towards the text on what sits on them, and the text sizes and
/// faces. Wisp's own palette leaves the colours as they are.
pub fn styled(base: &ThemeConfig, look: &Look) -> ThemeConfig {
    let mut theme = base.clone();
    let palette = if base.mode.is_dark() {
        look.dark
    } else {
        look.light
    };
    if palettes(palette.dark).next() != Some(palette) {
        let (page, text, accent) = (palette.background, palette.foreground, palette.accent);
        let shade = |amount: f32| Some(SharedString::from(mix(page, text, amount)));
        let set = |value: &str| Some(SharedString::from(value.to_owned()));
        let colors = &mut theme.colors;
        for slot in [
            &mut colors.primary,
            &mut colors.primary_hover,
            &mut colors.primary_active,
            &mut colors.ring,
            &mut colors.link,
            &mut colors.link_hover,
            &mut colors.link_active,
            &mut colors.caret,
            &mut colors.progress_bar,
            &mut colors.slider_bar,
            &mut colors.switch,
            &mut colors.sidebar_primary,
        ] {
            *slot = set(accent);
        }
        colors.primary_foreground = set(if palette.dark { page } else { "#FFFFFF" });
        colors.selection = Some(SharedString::from(format!("{accent}40")));
        for slot in [
            &mut colors.background,
            &mut colors.popover,
            &mut colors.list,
            &mut colors.tab_active,
        ] {
            *slot = set(page);
        }
        for slot in [
            &mut colors.foreground,
            &mut colors.popover_foreground,
            &mut colors.secondary_foreground,
            &mut colors.sidebar_foreground,
            &mut colors.tab_active_foreground,
            &mut colors.accent_foreground,
            &mut colors.sidebar_accent_foreground,
            &mut colors.group_box_foreground,
        ] {
            *slot = set(text);
        }
        for (slot, amount) in [
            (&mut colors.sidebar, 0.03),
            (&mut colors.title_bar, 0.03),
            (&mut colors.tab_bar, 0.03),
            (&mut colors.list_head, 0.03),
            (&mut colors.list_even, 0.02),
            (&mut colors.group_box, 0.04),
            (&mut colors.secondary, 0.06),
            (&mut colors.muted, 0.06),
            (&mut colors.accent, 0.08),
            (&mut colors.sidebar_accent, 0.08),
            (&mut colors.list_hover, 0.06),
            (&mut colors.list_active, 0.1),
            (&mut colors.secondary_hover, 0.1),
            (&mut colors.secondary_active, 0.14),
            (&mut colors.border, 0.14),
            (&mut colors.input, 0.18),
            (&mut colors.sidebar_border, 0.12),
            (&mut colors.title_bar_border, 0.12),
            (&mut colors.window_border, 0.14),
            (&mut colors.scrollbar_thumb, 0.25),
            (&mut colors.scrollbar_thumb_hover, 0.35),
            (&mut colors.muted_foreground, 0.55),
            (&mut colors.tab_foreground, 0.55),
        ] {
            *slot = shade(amount);
        }
    }
    if TEXT_SIZES.contains(&look.text_size) {
        theme.font_size = Some(look.text_size);
    }
    if TEXT_SIZES.contains(&look.code_size) {
        theme.mono_font_size = Some(look.code_size);
    }
    if !look.text_font.trim().is_empty() {
        theme.font_family = Some(look.text_font.trim().to_owned().into());
    }
    if !look.code_font.trim().is_empty() {
        theme.mono_font_family = Some(look.code_font.trim().to_owned().into());
    }
    theme
}

/// Wears `look` on both kinds of theme, and the chosen appearance again
/// so the window shows it at once.
pub fn restyle(look: &Look, appearance: Appearance, window: &mut Window, cx: &mut App) {
    for base in bundled().themes {
        let styled = Rc::new(styled(&base, look));
        let theme = Theme::global_mut(cx);
        if base.mode.is_dark() {
            theme.dark_theme = styled;
        } else {
            theme.light_theme = styled;
        }
    }
    apply(appearance, window, cx);
}

const THEMES: &str = include_str!("../assets/wisp-theme.json");

fn bundled() -> ThemeSet {
    serde_json::from_str(THEMES).expect("the bundled theme is valid JSON")
}

/// Replaces the library's default themes and follows the system appearance.
pub fn install(cx: &mut App) {
    for config in bundled().themes {
        let theme = Theme::global_mut(cx);
        if config.mode.is_dark() {
            theme.dark_theme = Rc::new(config);
        } else {
            theme.light_theme = Rc::new(config);
        }
    }
    Theme::sync_system_appearance(None, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_appearance_is_read_back_and_anything_else_follows_the_system() {
        for choice in Appearance::ALL {
            assert_eq!(Appearance::from_key(choice.key()), choice);
        }
        assert_eq!(Appearance::from_key(" dark "), Appearance::Dark);
        assert_eq!(Appearance::from_key("sepia"), Appearance::System);
        assert_eq!(Appearance::from_key(""), Appearance::System);
    }

    #[test]
    fn the_bundled_theme_has_one_light_and_one_dark_palette() {
        let themes = bundled().themes;
        assert_eq!(themes.len(), 2);
        assert_eq!(themes.iter().filter(|t| t.mode.is_dark()).count(), 1);
        for theme in &themes {
            // The accent and the page colours are what make it Wisp; a typo in
            // a key would silently fall back to the library's grey.
            for color in [
                &theme.colors.background,
                &theme.colors.foreground,
                &theme.colors.primary,
                &theme.colors.sidebar,
                &theme.colors.border,
            ] {
                let color = color.as_ref().expect("a core colour is missing");
                assert!(color.starts_with('#'), "{color}");
            }
        }
    }

    #[test]
    fn every_colour_key_is_one_the_library_reads() {
        // Unknown keys are ignored by serde; compare against a round trip so a
        // misspelt key fails here instead of being dropped.
        let raw: serde_json::Value = serde_json::from_str(THEMES).unwrap();
        for (index, theme) in bundled().themes.iter().enumerate() {
            let written = raw["themes"][index]["colors"].as_object().unwrap();
            let read = serde_json::to_value(&theme.colors).unwrap();
            for (key, value) in written {
                assert_eq!(read.get(key), Some(value), "unknown theme key {key}");
            }
        }
    }

    #[test]
    fn a_palette_lays_its_colours_over_wisps_and_wisps_own_leaves_them() {
        fn text(value: &Option<SharedString>) -> Option<&str> {
            value.as_ref().map(|value| value.as_ref())
        }
        let themes = bundled().themes;
        let light = themes.iter().find(|theme| !theme.mode.is_dark()).unwrap();
        let dark = themes.iter().find(|theme| theme.mode.is_dark()).unwrap();
        // Wisp's own palette changes no colour.
        let plain = styled(light, &Look::default());
        assert_eq!(
            serde_json::to_value(&plain.colors).unwrap(),
            serde_json::to_value(&light.colors).unwrap()
        );
        let look = Look {
            light: palette_of(false, "github"),
            dark: palette_of(true, "gruvbox"),
            text_size: 15.,
            code_font: " JetBrains Mono ".into(),
            ..Look::default()
        };
        let worn = styled(light, &look);
        assert_eq!(text(&worn.colors.primary), Some("#0969DA"));
        assert_eq!(text(&worn.colors.background), Some("#F6F8FA"));
        assert_eq!(text(&worn.colors.foreground), Some("#1F2328"));
        // Surfaces are shades of the page towards the text.
        assert_eq!(
            text(&worn.colors.border),
            Some(mix("#F6F8FA", "#1F2328", 0.14).as_str())
        );
        assert_eq!(worn.font_size, Some(15.));
        assert_eq!(text(&worn.mono_font_family), Some("JetBrains Mono"));
        // The dark theme wears the dark palette.
        assert_eq!(text(&styled(dark, &look).colors.primary), Some("#D79921"));
        // A size outside the range keeps Wisp's.
        let tiny = Look {
            text_size: 3.,
            ..Look::default()
        };
        assert_eq!(styled(light, &tiny).font_size, light.font_size);
    }

    #[test]
    fn palettes_are_read_back_by_key_and_colours_mix_evenly() {
        assert_eq!(palettes(false).count(), 5);
        assert_eq!(palettes(true).count(), 5);
        assert_eq!(palette_of(true, "codex").accent, "#7C8CFF");
        assert_eq!(palette_of(false, "codex").accent, "#2563EB");
        assert_eq!(palette_of(true, "everforest").key, "charcoal");
        assert_eq!(palette_of(false, "").key, "paper");
        assert_eq!(mix("#000000", "#FFFFFF", 0.5), "#808080");
        assert_eq!(mix("#102030", "#102030", 0.7), "#102030");
        assert_eq!(mix("#000000", "#FFFFFF", 2.), "#FFFFFF");
    }
}
