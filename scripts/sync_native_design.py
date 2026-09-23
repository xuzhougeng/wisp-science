#!/usr/bin/env python3
"""Export the WebView's brand, compose_icon glyphs and colors for native UI.

No rasterizer or third-party dependencies. Run --check in CI to detect drift.
WinUI links these same resources into its output; there is no second asset copy.
"""
import argparse
import colorsys
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "apps/macos/Sources/WispProjectBrowserUI/Resources"
ICONS = ("search", "refresh", "database", "folder", "star", "star-filled", "chat", "doc", "sync", "clock", "arrow-left", "chevron-left", "chevron-right", "chevron-down", "gear", "calendar", "upload", "plus", "folder-plus", "research-trail", "book", "grid", "list", "share", "timeline", "archive", "bell", "attach", "terminal", "panel", "adjustments", "close", "user", "sparkles", "wrench", "gauge", "check", "edit")
COLORS = ("bg-app", "bg-elev", "bg-sunken", "surface-hover", "text", "text-muted", "text-faint", "border", "border-strong", "clay", "clay-strong")


def exports():
    for theme in ("light", "dark"):
        yield f"wordmark-{theme}.svg", (ROOT / f"docs/assets/wordmark-{theme}.svg").read_bytes()
    source = (ROOT / "ui/src/app_support/messages.rs").read_text(encoding="utf-8")
    for icon in ICONS:
        match = re.search(r'"' + re.escape(icon) + r'" => view! \{ (.*?) \}\.into_view\(\)', source)
        if not match:
            raise ValueError(f"Missing compose_icon: {icon}")
        body = match[1].replace("currentColor", "#000000")
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>\n'
        yield f"icon-{icon}.svg", svg.encode()
    translations = (ROOT / "ui/src/i18n.rs").read_text(encoding="utf-8")
    by_locale = {"Zh": {}, "En": {}}
    for locale, key, value in re.findall(r'\(Locale::(Zh|En), "([^"]+)"\) => Some\(("(?:[^"\\]|\\.)*")\)', translations):
        try:
            by_locale[locale][key] = json.loads(value)
        except ValueError:
            pass
    navigation_source = (ROOT / "ui/src/app_support/settings.rs").read_text(encoding="utf-8")
    navigation_block = navigation_source.split("pub(crate) const SETTINGS_NAV_GROUPS:", 1)[1].split("pub(crate) fn", 1)[0]
    navigation = {}
    for group, entries in re.findall(r'"(settings.nav.[^"]+)",\s*&\[(.*?)\]', navigation_block, re.S):
        for section, aliases in re.findall(r'\(\s*"([^"]+)",\s*"([^"]+)",?\s*\)', entries):
            key = "settings.nav." + section.replace("-", "_")
            navigation[section] = {"zh": by_locale["Zh"][key], "en": by_locale["En"][key], "group": by_locale["Zh"][group], "group_en": by_locale["En"][group], "aliases": aliases}
    if len(navigation) != 19:
        raise ValueError("Review settings navigation export after WebView changes")
    yield "settings-navigation.json", (json.dumps(navigation, ensure_ascii=False, indent=2) + "\n").encode()
    labels = {value: by_locale["En"][key] for key, value in by_locale["Zh"].items() if key in by_locale["En"]}
    labels.update(json.loads((ROOT / "apps/macos/native-settings-english.json").read_text(encoding="utf-8")))
    yield "native-english.json", (json.dumps(labels, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
    settings = (ROOT / "ui/src/settings_view.rs").read_text(encoding="utf-8")
    block = settings.split("const MODEL_PRESETS:", 1)[1].split("];", 1)[0]
    presets = [{"label": label, "url": url, "model": model} for label, url, model in re.findall(r'\(\s*"([^"]*)",\s*"([^"]*)",\s*"([^"]*)",?\s*\)', block)]
    if len(presets) != 6:
        raise ValueError("Review MODEL_PRESETS export after WebView changes")
    yield "model-presets.json", (json.dumps(presets, indent=2) + "\n").encode()
    css = (ROOT / "ui/src/styles/base.css").read_text(encoding="utf-8")
    palettes = {}
    for theme, selector in (("light", ":root"), ("dark", ':root[data-theme="dark"]')):
        block = re.search(re.escape(selector) + r"\s*\{(.*?)\n\}", css, re.S)[1]
        values = dict(re.findall(r"--([\w-]+):\s*([^;]+);", block))
        palettes[theme] = {key: values[key] for key in COLORS}
    aliases = dict(zip(COLORS, ("app", "elev", "sunken", "hover", "text", "muted", "faint", "border", "border-strong", "accent", "accent-strong")))
    for theme, name, block in re.findall(r':root\[data-(light|dark)-palette="([\w-]+)"\]\s*\{([^}]+)\}', css):
        prefix = "lp" if theme == "light" else "dp"
        values = dict(re.findall(r"--([\w-]+):\s*([^;]+);", block))
        palettes[f"{theme}-{name}"] = {token: values[f"{prefix}-{alias}"] for token, alias in aliases.items()}
    trajectory = (ROOT / "ui/src/styles/chat.css").read_text(encoding="utf-8")
    for theme, selector in (("light", ".trajectory"), ("dark", ':root[data-theme="dark"] .trajectory')):
        block = re.search(re.escape(selector) + r"\s*\{(.*?)\n\}", trajectory, re.S)[1]
        colors = dict(re.findall(r"--(traj-(?:input|model|tool)-bar):\s*([^;]+);", block))
        for key, value in colors.items():
            if value.startswith("hsl("):
                h, saturation, lightness = map(float, re.findall(r"[\d.]+", value))
                colors[key] = "#" + "".join(f"{round(v * 255):02x}" for v in colorsys.hls_to_rgb(h / 360, lightness / 100, saturation / 100))
        for name, palette in palettes.items():
            if name == theme or name.startswith(theme + "-"):
                palette.update(colors)
    yield "palette.json", (json.dumps(palettes, indent=2) + "\n").encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    stale = []
    for name, contents in exports():
        path = DEST / name
        if args.check:
            # Git may check text assets out with CRLF on Windows.
            if not path.exists() or path.read_bytes().replace(b"\r\n", b"\n") != contents.replace(b"\r\n", b"\n"):
                stale.append(name)
        else:
            DEST.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
    if stale:
        parser.exit(1, "Native assets differ from WebView: " + ", ".join(stale) + "\nRun python3 scripts/sync_native_design.py\n")


if __name__ == "__main__":
    main()
