#!/usr/bin/env python3
"""Export iOS template SVG assets directly from the shared compose_icon set."""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[3]
DEST = ROOT / "apps/ios/WispScience/Assets.xcassets"
ICONS = ("image", "arrow-up", "chevron-right", "chevron-down", "chevron-left", "calendar", "research-trail",
         "list", "notebook", "camera", "square", "check-square", "edit", "book", "check", "gear", "inbox",
         "circle-help", "square-pen", "folder", "plus", "circle", "circle-check", "circle-alert")


def main():
    source = (ROOT / "ui/src/app_support/messages.rs").read_text(encoding="utf-8")
    for name in ICONS:
        match = re.search(r'"' + re.escape(name) + r'" => view! \{ (.*?) \}\.into_view\(\)', source)
        if not match:
            raise ValueError(f"Missing compose_icon: {name}")
        svg = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" '
               'fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" '
               'stroke-linejoin="round">' + match[1].replace("currentColor", "#000000") + '</svg>\n')
        directory = DEST / f"wisp-{name}.imageset"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "icon.svg").write_text(svg, encoding="utf-8")
        (directory / "Contents.json").write_text(json.dumps({
            "images": [{"filename": "icon.svg", "idiom": "universal"}],
            "info": {"author": "xcode", "version": 1},
            "properties": {"preserves-vector-representation": True, "template-rendering-intent": "template"},
        }, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
