"""Offline coverage for the SwiftUI string check."""

from pathlib import Path
import tempfile
import unittest

from check_strings_native_macos import findings


class StringCheckTests(unittest.TestCase):
    def scan(self, source):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "View.swift").write_text(source, encoding="utf-8")
            return [reason for _, _, reason in findings(Path(directory))]

    def test_flags_english_only_ui_literals_and_capitalized_raw_values(self):
        self.assertEqual(self.scan('Text("Agent Context")\n'), ['English-only literal "Agent Context"'])
        self.assertEqual(self.scan('.help("Close panel")\n'), ['English-only literal "Close panel"'])
        self.assertEqual(self.scan('Text(model.effort.capitalized)\n'), ["raw value shown via .capitalized"])

    def test_chinese_interpolated_allowed_and_symbol_text_pass(self):
        self.assertEqual(self.scan(
            'Text(localized("Agent 上下文"))\n'
            'Text("\\(count) MCP")\n'
            'Text("PNG")\n'
            'Button("—") {}\n'
        ), [])

    def test_the_swiftui_sources_are_clean(self):
        self.assertEqual(list(findings()), [])


if __name__ == "__main__":
    unittest.main()
