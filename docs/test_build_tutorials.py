import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlsplit

import build_tutorials


class TutorialBuildTests(unittest.TestCase):
    DIRECTORY = (build_tutorials.DOCS / "tutorials.html").read_text(encoding="utf-8")

    def test_articles_keep_examples_and_rebase_links_and_images(self):
        with tempfile.TemporaryDirectory() as directory:
            docs = Path(directory)
            (docs / "wechat/en").mkdir(parents=True)
            (docs / "wechat/first.md").write_text(
                '# First tutorial\n\n[Next](second.md)\n\n'
                '[Reference](../reference.md)\n\n[External](https://example.com/help)\n\n'
                '![Screenshot](../assets/demo.png)\n\n'
                '| Input | Result |\n| --- | --- |\n| A | B |\n\n'
                '```markdown\n# Example title\n[Next](second.md)\n<script>alert(1)</script>\n```\n\n'
                'The final paragraph is included.\n', encoding="utf-8"
            )
            (docs / "wechat/second.md").write_text('# Second tutorial\n\nAnother article.\n', encoding="utf-8")
            for name in ["first.md", "second.md"]:
                (docs / "wechat/en" / name).write_text((docs / "wechat" / name).read_text(encoding="utf-8"), encoding="utf-8")
            with patch.object(build_tutorials, "DOCS", docs):
                pages = build_tutorials.render_tutorials(self.DIRECTORY)
                html = pages["tutorials/first.html"]
            self.assertIn('href="second.html">Next</a>', html)
            self.assertIn('href="' + build_tutorials.REPOSITORY + 'reference.md"', html)
            self.assertIn('href="https://example.com/help"', html)
            self.assertIn('src="../assets/demo.png" alt="Screenshot" loading="lazy"', html)
            self.assertIn('<th>Input</th>', html)
            self.assertIn('<td>B</td>', html)
            self.assertIn('# Example title\n[Next](second.md)', html)
            self.assertIn('&lt;script&gt;alert(1)&lt;/script&gt;', html)
            self.assertIn('The final paragraph is included.', html)
            self.assertNotIn('Another article.', html)
            self.assertIn('Another article.', pages['tutorials/second.html'])
            self.assertNotIn('The final paragraph is included.', pages['tutorials.html'])
            self.assertIn('href="tutorials/first.html"', pages['tutorials.html'])
            self.assertIn('href="../tutorials.html#first"', html)

    def test_checked_in_page_matches_all_markdown_sources(self):
        for name, expected in build_tutorials.render_tutorials().items():
            with self.subTest(page=name):
                self.assertEqual((build_tutorials.DOCS / name).read_text(encoding="utf-8"), expected)

    def test_sibling_navigation_follows_reading_order(self):
        pages = build_tutorials.render_tutorials()
        # The directory lists cards in reading order; previous/next must walk it.
        order = re.findall(r'<a class="tutorial-card" id="([^"]+)"', pages["tutorials.html"])
        self.assertEqual(order[0], "wisp-science-quick-start")
        first = pages[f"tutorials/{order[0]}.html"]
        last = pages[f"tutorials/{order[-1]}.html"]
        self.assertNotIn('class="tutorial-previous"', first)
        self.assertNotIn('class="tutorial-next"', last)
        for previous, following in zip(order, order[1:]):
            with self.subTest(previous=previous, following=following):
                self.assertIn(f'class="tutorial-next" href="{following}.html"',
                              pages[f"tutorials/{previous}.html"])
                self.assertIn(f'class="tutorial-previous" href="{previous}.html"',
                              pages[f"tutorials/{following}.html"])
        self.assertIn('<title>快速开始 · 教程 | Wisp Science</title>', first)
        self.assertIn('src="../assets/i18n.js"', first)

    def test_cli_is_a_separate_tutorial_from_server_setup(self):
        pages = build_tutorials.render_tutorials()
        cli = pages["tutorials/wisp-science-cli.html"]
        server = pages["tutorials/wisp-science-servers-cli.html"]
        self.assertIn('data-text-zh="Wisp 命令行"', cli)
        self.assertIn('data-text-en="Wisp CLI"', cli)
        self.assertIn('WISP_API_KEY', cli)
        self.assertIn('wisp-science run --output jsonl', cli)
        self.assertIn('Get-Credential', cli)
        self.assertNotIn('WISP_API_KEY', server)
        self.assertNotIn('wisp-science run --output jsonl', server)
        self.assertIn('href="wisp-science-cli.html"', server)
        self.assertIn('href="tutorials/wisp-science-cli.html"', pages["tutorials.html"])

    def test_directory_groups_tutorials_without_losing_order_or_links(self):
        directory = build_tutorials.render_tutorials()["tutorials.html"]
        groups = re.findall(r'<section class="tutorial-group" id="([^"]+)".*?</section>',
                            directory, re.DOTALL)
        self.assertEqual(groups, ["basics", "tips", "advanced"])
        listed = []
        for group, zh, en in build_tutorials.TUTORIAL_GROUPS:
            section = re.search(rf'<section class="tutorial-group" id="{group}".*?</section>',
                                directory, re.DOTALL).group()
            self.assertIn(f'aria-labelledby="{group}-title"', section)
            self.assertIn(f'data-text-zh="{zh}" data-text-en="{en}"', section)
            article_ids = re.findall(r'<a class="tutorial-card" id="([^"]+)"', section)
            self.assertTrue(article_ids, group)
            self.assertEqual(section.count('<h3 '), len(article_ids))
            # Every card in a group carries that group's label.
            self.assertEqual(section.count(build_tutorials.localized("span", zh, en)),
                             len(article_ids))
            listed += article_ids
        # Each source appears once, and a group never interrupts the reading
        # order: card numbers count up from 01 down the whole page.
        sources = sorted(path.stem for path in (build_tutorials.DOCS / "wechat").glob("*.md"))
        self.assertEqual(sorted(listed), sources)
        self.assertEqual(re.findall(r'<span class="tutorial-number">(\d+)</span>', directory),
                         [f"{number:02d}" for number in range(1, len(sources) + 1)])

    def test_specialists_and_quick_actions_are_adjacent_bilingual_tips(self):
        pages = build_tutorials.render_tutorials()
        specialists = pages["tutorials/wisp-science-specialists.html"]
        actions = pages["tutorials/wisp-science-quick-actions.html"]
        self.assertIn('class="tutorial-next" href="wisp-science-quick-actions.html"', specialists)
        self.assertIn('class="tutorial-previous" href="wisp-science-specialists.html"', actions)
        self.assertIn('data-text-en="Wisp Science Tips: Specialists"', specialists)
        self.assertIn('data-text-en="Wisp Science Tips: Quick Actions"', actions)
        self.assertIn('src="../assets/specialists/01-overview.png"', specialists)
        self.assertIn('src="../assets/tutorials/en/specialists/01-overview.png"', specialists)
        self.assertIn('width="1612" height="710"', specialists)
        for language in ["", "en/"]:
            for screenshot in ["01-settings", "02-create"]:
                self.assertIn(f'src="../assets/tutorials/{language}quick-actions/{screenshot}.png"', actions)

    def test_every_tutorial_has_a_complete_english_source_and_language_metadata(self):
        root = build_tutorials.DOCS / "wechat"
        self.assertEqual({p.name for p in root.glob("*.md")}, {p.name for p in (root / "en").glob("*.md")})
        pages = build_tutorials.render_tutorials()
        for source in root.glob("*.md"):
            english = root / "en" / source.name
            with self.subTest(article=source.name):
                expected = english.read_text(encoding="utf-8").splitlines()[0].removeprefix("# ")
                html = pages[f"tutorials/{source.stem}.html"]
                self.assertIn(f'data-text-en="{expected}"', html)
                self.assertIn('class="tutorial-body lang-en" lang="en"', html)
                self.assertIn('class="tutorial-body lang-zh" lang="zh-CN"', html)
                self.assertIn(f'/wechat/en/{source.name}', html)
                self.assertIn('data-title-en=', html)

    def test_missing_translation_fails_instead_of_publishing_chinese_as_english(self):
        with tempfile.TemporaryDirectory() as directory:
            docs = Path(directory)
            (docs / "wechat").mkdir()
            (docs / "wechat/missing.md").write_text("# Untranslated\n\nText.", encoding="utf-8")
            with patch.object(build_tutorials, "DOCS", docs):
                with self.assertRaisesRegex(ValueError, "Missing English tutorial"):
                    build_tutorials.render_tutorials(self.DIRECTORY)

    def test_article_links_and_screenshots_exist(self):
        parser = build_tutorials.MarkdownIt("commonmark")
        for source in (build_tutorials.DOCS / "wechat").rglob("*.md"):
            for token in parser.parse(source.read_text(encoding="utf-8")):
                for child in token.children or []:
                    attribute = {"link_open": "href", "image": "src"}.get(child.type)
                    if not attribute:
                        continue
                    value = child.attrGet(attribute)
                    if urlsplit(value).scheme or value.startswith(("//", "#")):
                        continue
                    target = source.parent / urlsplit(value).path
                    with self.subTest(article=source.name, target=value):
                        self.assertTrue(target.is_file(), f"Missing tutorial resource: {target}")

    def test_english_tutorials_have_their_own_screenshots(self):
        parser = build_tutorials.MarkdownIt("commonmark")
        root = build_tutorials.DOCS / "wechat"
        def images(source):
            return [child.attrGet("src") for token in parser.parse(source.read_text(encoding="utf-8"))
                    for child in token.children or [] if child.type == "image"]
        referenced = set()
        for english in (root / "en").glob("*.md"):
            english_images = images(english)
            with self.subTest(article=english.name):
                self.assertEqual(len(english_images), len(images(root / english.name)))
                for value in english_images:
                    self.assertTrue(value.startswith("../../assets/tutorials/en/"), value)
                    target = (english.parent / value).resolve()
                    self.assertTrue(target.is_file(), str(target))
                    referenced.add(target)
        # Every published English screenshot must be used by a tutorial.
        self.assertEqual(referenced, {p.resolve() for p in (build_tutorials.DOCS / "assets/tutorials/en").rglob("*.png")})


if __name__ == "__main__":
    unittest.main()
