"""Guard the WinUI initialization order that otherwise drops all but the first line.

This source guard complements the real-window regression: open Reviewer's
read-only instructions and verify Rules and the output JSON remain scrollable.
It does not replace that native rendering check.
"""
from pathlib import Path
import re
import unittest


class NativeMultilineTests(unittest.TestCase):
    def test_multiline_mode_is_enabled_before_loading_text(self):
        root = Path(__file__).resolve().parents[1] / "apps/windows/Wisp.Science.Preview"
        failures = []
        checked = 0
        for path in root.glob("*.cs"):
            source = path.read_text(encoding="utf-8-sig")
            for match in re.finditer(r"new TextBox\s*\{([^;]*?)\};?", source, re.DOTALL):
                initializers = match.group(1)
                mode = re.search(r"\bAcceptsReturn\s*=", initializers)
                text = re.search(r"\bText\s*=", initializers)
                if mode and text:
                    checked += 1
                    if text.start() < mode.start():
                        line = source.count("\n", 0, match.start()) + 1
                        failures.append(f"{path.name}:{line}")
        self.assertGreater(checked, 0, "No multiline controls were inspected")
        self.assertEqual(failures, [], "Text assigned before multiline mode: " + ", ".join(failures))


if __name__ == "__main__":
    unittest.main()
