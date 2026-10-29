#!/usr/bin/env python3
"""Keep English out of the SwiftUI client's Chinese interface.

The native app is Chinese-first: a user-facing literal is written in Chinese and
`localized()` maps it to English (native-english.json). An English-only literal
therefore shows up untranslated in the Chinese UI, and `.capitalized` turns a
raw host value ("high", "global") into English text. Names that are the same in
both languages (brands, file formats, sample code) are listed in ALLOWED.
Run --list to print every match with its location.
"""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / "apps/macos/Sources/WispProjectBrowserUI"
CALL = re.compile(
    r'\b(?:Text|Button|Label|Toggle|Picker|TextField|SecureField|Menu|DisclosureGroup|ProgressView'
    r'|help|accessibilityLabel|localized|title|label|hint)\s*[(:]\s*"((?:[^"\\]|\\.)*)"'
)
CJK = re.compile(r"[　-鿿＀-￯]")
ALLOWED = {
    # Product, service and format names, identical in both locales.
    "Wisp Science", "Python", "Rscript", "Agents", "Fast", "PNG", "HTML", "SHA-256", "SHA-256: ", "320–2400 px",
    # Field names as the Feishu console and HTTP spell them.
    "App ID", "App Secret", "User-Agent",
    # Not shown: a dispatch queue label.
    "science.wisp.preview-assets",
    # The appearance preview's sample notebook cell.
    'import pandas as pd\\ndata = pd.read_csv(\\"samples.csv\\")\\ndata.head()',
}


def findings(sources=SOURCES):
    for path in sorted(sources.glob("*.swift")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if ".capitalized" in line:
                yield path, number, "raw value shown via .capitalized"
            for match in CALL.finditer(line):
                text = match.group(1)
                if "\\(" in text or CJK.search(text) or not re.search(r"[A-Za-z]{2,}", text):
                    continue
                if text not in ALLOWED:
                    yield path, number, f'English-only literal "{text}"'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true", help="print matches without failing")
    args = parser.parse_args()
    found = list(findings())
    for path, number, reason in found:
        print(f"{path.relative_to(ROOT)}:{number}: {reason}")
    if found and not args.list:
        print("Write user-facing text in Chinese and wrap it in localized(), "
              "or add a same-in-both-languages name to ALLOWED in scripts/check_strings_native_macos.py.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
