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
ICONS = ("search", "sort", "refresh", "database", "folder", "star", "star-filled", "chat", "doc", "sync", "clock", "arrow-left", "arrow-right", "branch", "chevron-left", "chevron-right", "chevron-down", "chevron-up", "gear", "calendar", "upload", "download", "link", "plus", "minus", "folder-plus", "research-trail", "book", "grid", "list", "share", "timeline", "archive", "archive-import", "archive-export", "conversation-import", "conversation-move", "fork", "bell", "attach", "terminal", "panel", "runtime-panel", "adjustments", "close", "user", "sparkles", "lightbulb", "wrench", "gauge", "check", "hand", "ban", "edit", "pin", "trash", "copy", "more", "grip-vertical", "circle-alert", "server", "arrow-up", "bolt", "plan", "expand", "bubble", "sun", "moon", "monitor")
ICONS += ("external-link", "loader", "image", "note-plus", "flask", "eye-off", "play", "save", "table")
ICONS += ("panel-left", "layers", "memory")
ICONS += ("clipboard", "circle-check", "controls")
ICONS += ("review", "skill")
ICONS += ("up",)
ICONS += ("circle-help",)
ICONS += ("shield",)
ICONS += ("notebook", "lock", "square-pen")
ICONS += ("eye",)
ICONS += ("brain",)
ICONS += ("undo",)
ICONS += ("folder-move",)
ICONS += ("bot",)
ICONS += ("activity-orbit", "circle-minus")
ICONS += ("context-compact",)
ICONS += ("dock", "resize")
ICONS += ("stop",)
ICONS += ("history",)
ICONS += ("undo-compact",)
ICONS += ("type-increase", "type-decrease", "code-increase", "code-decrease")
ICONS += ("compass", "swarm")
# WebView-only settings sections the native preview has no page for yet.
WEBVIEW_ONLY_SECTIONS = set()
COLORS = ("bg-app", "bg-elev", "bg-sunken", "bg-panel", "bg-input", "surface-hover", "text", "text-muted", "text-faint", "border", "border-strong", "clay", "clay-strong", "clay-soft", "ok", "warn", "err", "on-clay")
THEME_COLORS = ("focus-ring", "warn-soft", "info", "info-soft", "syntax-text", "syntax-comment", "syntax-string", "syntax-variable", "syntax-keyword")


def validate_palette_tokens(palettes, used_tokens):
    for name, palette in palettes.items():
        missing = used_tokens - palette.keys()
        if missing:
            raise ValueError(f"Native palette {name} missing WinUI colors: {sorted(missing)}")


def native_icon_references(source):
    direct = re.findall(r'\.Icon\("([^"]+)"', source)
    actions = re.findall(r'(?:\.ToolButton|\bIcon)\("[^"]+",\s*"([^"]+)"', source)
    file_actions = re.findall(r'\bFileButton\((?:design\.(?:T|L)\([^;\n]*?\)|"[^"]+"),\s*"([^"]+)"', source)
    return set(direct + actions + file_actions)


def native_color(value):
    # Both native renderers consume hex/rgba; the WebView also uses CSS HSL.
    if value.startswith("hsl("):
        h, saturation, lightness, *alpha = map(float, re.findall(r"[\d.]+", value))
        rgb = [round(v * 255) for v in colorsys.hls_to_rgb(h / 360, lightness / 100, saturation / 100)]
        if alpha:
            opacity = alpha[0] / 100 if re.search(r"/\s*[\d.]+%", value) else alpha[0]
            return f"rgba({rgb[0]}, {rgb[1]}, {rgb[2]}, {opacity:g})"
        return "#" + "".join(f"{v:02x}" for v in rgb)
    return value


def locale_catalog(source):
    catalog = {"en": {}, "zh": {}}
    pattern = r'\(\s*Locale::(Zh|En),\s*"([^"]+)"\s*,?\s*\)\s*=>\s*(?:\{\s*)?Some\(\s*("(?:[^"\\]|\\.)*")\s*,?\s*\)'
    # Rust's escaped newline removes the following indentation. JSON strings do
    # not have that syntax, so normalize it before decoding the literal.
    source = re.sub(r'\\\r?\n\s*', '', source)
    for locale, key, literal in re.findall(pattern, source):
        literal = re.sub(r'\\u\{([0-9a-fA-F]+)\}', lambda m: chr(int(m[1], 16)), literal)
        catalog[locale.lower()][key] = json.loads(literal)
    return catalog


def native_translation_keys(sources):
    # A partial page may declare its T shorthand in a different file. A local
    # shorthand (e.g. the run review) applies only in its own source file.
    member_prefixes = {}
    prefixes = {}
    for name, source in sources.items():
        helper = re.search(r'(private\s+)?string T\(string (\w+)\)\s*=>\s*\w+\.T\("([^"]+)"\s*\+\s*\2\)', source)
        if helper:
            prefixes[name] = helper[3]
            if helper[1]:
                member_prefixes[name.split(".")[0]] = helper[3]
    for name, source in sources.items():
        prefix = prefixes.get(name, member_prefixes.get(name.split(".")[0], ""))
        for receiver, key in re.findall(r'\b(?:(\w+)\.)?T\("([^"]+)"\)', source):
            yield key if receiver else prefix + key


def exports():
    # Fail the build before a missing asset can abort a WinUI page constructor.
    native_icons = set()
    native_colors = set()
    for path in (ROOT / "apps/windows/Wisp.Science.Preview").glob("*.cs"):
        native_source = path.read_text(encoding="utf-8")
        native_icons.update(native_icon_references(native_source))
        native_colors.update(re.findall(r'\.(?:Brush|ColorText)\("([^"]+)"', native_source))
    palette = (ROOT / "apps/windows/Wisp.ProjectBrowser/NativePaletteActions.cs").read_text(encoding="utf-8")
    native_icons.update(re.findall(r'new\("[^"]+",\s*"[^"]+",\s*"([^"]+)"', palette))
    missing = native_icons - set(ICONS)
    if missing:
        raise ValueError(f"WinUI icons missing from native exports: {sorted(missing)}")
    # Use the same pinned language grammars as WebView; JavaScriptCore produces
    # token ranges while AppKit keeps the text selectable and renders it natively.
    yield "highlight.min.js", (ROOT / "ui/vendor-src/highlight.min.js").read_bytes()
    rich = ROOT / "apps/windows/Wisp.Science.Preview/Assets/RichPreview"
    for native, shared in (("native-rich-preview.css", "preview.css"), ("native-office-preview.css", "office.css"), ("native-preview-selection.mjs", "selection.mjs")):
        yield native, (rich / shared).read_bytes()
    for theme in ("light", "dark"):
        yield f"wordmark-{theme}.svg", (ROOT / f"docs/assets/wordmark-{theme}.svg").read_bytes()
    yield "logo.svg", (ROOT / "ui/logo.svg").read_bytes()
    source = (ROOT / "ui/src/app_support/messages.rs").read_text(encoding="utf-8")
    for icon in ICONS:
        match = re.search(r'"' + re.escape(icon) + r'"\s*=>\s*view!\s*\{\s*(.*?)\s*\}\.into_view\(\)', source, re.DOTALL)
        if not match:
            raise ValueError(f"Missing compose_icon: {icon}")
        body = match[1].replace("currentColor", "#000000")
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>\n'
        yield f"icon-{icon}.svg", svg.encode()
    translations = (ROOT / "ui/src/i18n.rs").read_text(encoding="utf-8")
    catalog = locale_catalog(translations)
    used_keys = set(native_translation_keys({path.name: path.read_text(encoding="utf-8")
        for path in (ROOT / "apps/windows/Wisp.Science.Preview").glob("*.cs")}))
    for locale, strings in catalog.items():
        missing = used_keys - strings.keys()
        if missing:
            raise ValueError(f"Missing {locale} native UI translations: {sorted(missing)}")
    yield "native-locales.json", (json.dumps(catalog, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
    by_locale = {"Zh": {}, "En": {}}
    for locale, key, value in re.findall(r'\(Locale::(Zh|En), "([^"]+)"\) => Some\(("(?:[^"\\]|\\.)*")\)', translations):
        try:
            by_locale[locale][key] = json.loads(value)
        except ValueError:
            pass
    navigation_source = (ROOT / "ui/src/app_support/settings.rs").read_text(encoding="utf-8")
    # Service ordering, fields, help and official links share the WebView registry.
    credential_block = navigation_source.split("pub(crate) const CRED_GROUPS:", 1)[1].split("pub(crate) fn", 1)[0]
    credentials = []
    for body in re.split(r"\bCredGroup\s*\{", credential_block)[1:]:
        group = dict(re.findall(r'(\w+):\s*"([^"]*)"', body.split("links:", 1)[0]))
        group["links"] = [dict(re.findall(r'(\w+):\s*"([^"]*)"', link)) for link in re.findall(r"CredLink\s*\{(.*?)\}", body, re.S)]
        group["fields"] = []
        for field in re.findall(r"CredField\s*\{(.*?)\}", body, re.S):
            item = dict(re.findall(r'(\w+):\s*"([^"]*)"', field))
            item["secret"] = re.search(r"secret:\s*(true|false)", field)[1] == "true"
            group["fields"].append(item)
        if not group.get("id") or not group["fields"]:
            raise ValueError("Review credential group export after WebView changes")
        credentials.append(group)
    if not credentials:
        raise ValueError("Missing WebView credential groups")
    yield "credential-groups.json", (json.dumps(credentials, indent=2) + "\n").encode()
    navigation_block = navigation_source.split("pub(crate) const SETTINGS_NAV_GROUPS:", 1)[1].split("pub(crate) fn", 1)[0]
    navigation = {}
    for group, entries in re.findall(r'"(settings.nav.[^"]+)",\s*&\[(.*?)\]', navigation_block, re.S):
        for section, aliases in re.findall(r'\(\s*"([^"]+)",\s*"([^"]+)",?\s*\)', entries):
            if section in WEBVIEW_ONLY_SECTIONS:
                continue
            key = "settings.nav." + section.replace("-", "_")
            navigation[section] = {"zh": by_locale["Zh"][key], "en": by_locale["En"][key], "group": by_locale["Zh"][group], "group_en": by_locale["En"][group], "aliases": aliases}
    if len(navigation) != 21:
        raise ValueError("Review settings navigation export after WebView changes")
    # Keep native automation presets identical to the WebView, including weekday
    # conventions (the exported value remains JavaScript Sunday=0).
    automation = (ROOT / "ui/src/automation.rs").read_text(encoding="utf-8")
    block = automation.split("static TEMPLATES:", 1)[1].split("];", 1)[0]
    templates = []
    pattern = r'Template \{\s*icon: "([^"]+)",\s*title: \("([^"]+)", "([^"]+)"\),\s*prompt: \(\s*("(?:[^"\\]|\\.)*"),\s*("(?:[^"\\]|\\.)*"),\s*\),\s*cadence: Cadence::(\w+),\s*time: "([^"]+)",\s*weekday: (\d+),\s*\}'
    for icon, en, zh, prompt_en, prompt_zh, cadence, time, weekday in re.findall(pattern, block):
        templates.append(dict(icon=icon, en=en, zh=zh, prompt_en=json.loads(prompt_en), prompt_zh=json.loads(prompt_zh), cadence=cadence.lower(), time=time, weekday=int(weekday)))
    if len(templates) != 3:
        raise ValueError("Review automation template export after WebView changes")
    yield "automation-templates.json", (json.dumps(templates, ensure_ascii=False, indent=2) + "\n").encode()
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
    # Settings → Models → 决策模型: the services, think modes and both locales'
    # strings, so the native page never hand-copies them.
    decision = (ROOT / "crates/wisp-dto/src/decision_model.rs").read_text(encoding="utf-8")
    block = decision.split("pub const DECISION_PRESETS:", 1)[1].split("];", 1)[0]
    pattern = r'DecisionPreset \{\s*id: "([^"]+)",\s*base_url: "([^"]*)",\s*model: "([^"]*)",\s*timeout_secs: (\d+),\s*key_required: (true|false),\s*key_url: "([^"]*)",\s*\}'
    decision_presets = [dict(id=i, base_url=b, model=m, timeout_secs=int(t), key_required=k == "true", key_url=u) for i, b, m, t, k, u in re.findall(pattern, block)]
    if not decision_presets or len(decision_presets) != block.count("DecisionPreset {"):
        raise ValueError("Review DECISION_PRESETS export after decision model changes")
    think_modes = json.loads("[" + decision.split("pub const DECISION_THINK_MODES: &[&str] = &[", 1)[1].split("];", 1)[0] + "]")
    strings = {key: {"zh": value, "en": by_locale["En"].get(key, value)} for key, value in by_locale["Zh"].items() if key.startswith("decision.")}
    yield "decision-model.json", (json.dumps({"presets": decision_presets, "think_modes": think_modes, "strings": strings}, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
    # Image/video selectors consume the exact shared DTO option order.
    dto = (ROOT / "crates/wisp-dto/src/lib.rs").read_text(encoding="utf-8")
    media = {}
    for name in ("OPENAI_IMAGE_SIZES", "OPENAI_IMAGE_QUALITIES", "GROK_IMAGE_ASPECT_RATIOS",
                 "GROK_IMAGE_RESOLUTIONS", "GROK_IMAGE_QUALITIES", "VIDEO_ASPECT_RATIOS", "VIDEO_RESOLUTIONS"):
        block = re.search(r'pub const ' + name + r':\s*&\[&str\]\s*=\s*&\[(.*?)\];', dto, re.S)
        if not block:
            raise ValueError(f"Missing shared media options: {name}")
        media[name] = re.findall(r'"([^"]*)"', block[1])
    for name in ("VIDEO_DURATION_MIN_SECS", "VIDEO_DURATION_MAX_SECS", "VIDEO_DURATION_DEFAULT_SECS"):
        media[name] = int(re.search(r'pub const ' + name + r':\s*u32\s*=\s*(\d+);', dto)[1])
    yield "model-media-options.json", (json.dumps(media, indent=2) + "\n").encode()
    css = (ROOT / "ui/src/styles/base.css").read_text(encoding="utf-8")
    palettes = {}
    for theme, selector in (("light", ":root"), ("dark", ':root[data-theme="dark"]')):
        block = re.search(re.escape(selector) + r"\s*\{(.*?)\n\}", css, re.S)[1]
        values = dict(re.findall(r"--([\w-]+):\s*([^;]+);", block))
        palettes[theme] = {key: values[key] for key in COLORS + THEME_COLORS}
    aliases = dict(zip(COLORS, ("app", "elev", "sunken", "panel", "input", "hover", "text", "muted", "faint", "border", "border-strong", "accent", "accent-strong", "accent-soft", "ok", "warn", "err", "on-clay"), strict=True))
    for theme, name, block in re.findall(r':root\[data-(light|dark)-palette="([\w-]+)"\]\s*\{([^}]+)\}', css):
        prefix = "lp" if theme == "light" else "dp"
        values = dict(re.findall(r"--([\w-]+):\s*([^;]+);", block))
        # Semantic approval colors and accent foreground inherit the WebView theme.
        palettes[f"{theme}-{name}"] = {token: palettes[theme][token] if token in ("ok", "warn", "err", "on-clay") else values[f"{prefix}-{alias}"] for token, alias in aliases.items()}
        palettes[f"{theme}-{name}"].update({token: palettes[theme][token] for token in THEME_COLORS})
    trajectory = (ROOT / "ui/src/styles/chat.css").read_text(encoding="utf-8")
    for theme, selector in (("light", ".trajectory"), ("dark", ':root[data-theme="dark"] .trajectory')):
        block = re.search(re.escape(selector) + r"\s*\{(.*?)\n\}", trajectory, re.S)[1]
        colors = dict(re.findall(r"--(traj-(?:input|model|tool)-bar):\s*([^;]+);", block))
        for name, palette in palettes.items():
            if name == theme or name.startswith(theme + "-"):
                palette.update(colors)
    palettes = {name: {token: native_color(value) for token, value in palette.items()} for name, palette in palettes.items()}
    validate_palette_tokens(palettes, native_colors)
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
