import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build_skills


class SkillsPageTests(unittest.TestCase):
    def test_bear_is_a_marketplace_source_not_a_bundled_catalog_entry(self):
        html = build_skills.render_skills()
        self.assertIn("BEAR Research Skills", html)
        self.assertNotIn('data-skill-id="bear-', html)
        self.assertFalse(any((build_skills.DOCS.parent / "skills").glob("bear-*/SKILL.md")))

    def test_catalog_covers_bundled_skills_and_both_languages(self):
        docs = build_skills.DOCS
        entries = json.loads((docs / "skills-catalog.json").read_text(encoding="utf-8"))
        bundled = {p.parent.name for p in (docs.parent / "skills").glob("*/SKILL.md")}
        self.assertEqual({entry["id"] for entry in entries}, bundled)
        self.assertEqual(len(entries), len(bundled))
        self.assertEqual({entry["group"] for entry in entries}, {group[0] for group in build_skills.GROUPS})
        for entry in entries:
            with self.subTest(skill=entry["id"]):
                self.assertTrue(all(entry[field].strip() for field in ["title_zh", "title_en", "summary_zh", "summary_en"]))

    def test_checked_in_catalog_matches_the_generator(self):
        html = (build_skills.DOCS / "skills.html").read_text(encoding="utf-8")
        generated = html.split(build_skills.START)[1].split(build_skills.END)[0]
        self.assertEqual(generated.strip(), build_skills.render_skills())

    def test_stale_catalog_fails_instead_of_silently_omitting_a_skill(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            docs = root / "docs"
            docs.mkdir()
            (root / "skills/new-skill").mkdir(parents=True)
            (root / "skills/new-skill/SKILL.md").write_text("---\nname: new-skill\n---\n", encoding="utf-8")
            (docs / "skills-catalog.json").write_text("[]", encoding="utf-8")
            with patch.object(build_skills, "DOCS", docs):
                with self.assertRaisesRegex(ValueError, "every bundled SKILL.md"):
                    build_skills.render_skills()


if __name__ == "__main__":
    unittest.main()
