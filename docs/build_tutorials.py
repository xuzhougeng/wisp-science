"""Render bilingual tutorial pages; --check detects generated drift or missing translations."""

import argparse
import posixpath
import re
import struct
from html import escape
from pathlib import Path
from urllib.parse import urljoin, urlsplit, urlunsplit

from markdown_it import MarkdownIt


DOCS = Path(__file__).resolve().parent
START = "<!-- BEGIN GENERATED TUTORIALS -->"
END = "<!-- END GENERATED TUTORIALS -->"
REPOSITORY = "https://github.com/xuzhougeng/wisp-science/blob/main/docs/"
TUTORIAL_GROUPS = [
    ("basics", "基础入门", "Basics"),
    ("tips", "使用技巧", "Tips"),
    ("advanced", "进阶", "Advanced"),
]
READING_ORDER = [
    "wisp-science-quick-start", "wisp-science-models", "wisp-science-browser",
    "wisp-science-servers-cli", "wisp-science-transfer", "wisp-science-mcp",
    "wisp-science-skills", "wisp-science-trajectory", "wisp-science-research-journey",
    "wisp-science-specialists", "wisp-science-quick-actions",
    "wisp-science-transcriptome-upstream", "wisp-science-rnaseq-downstream",
    "wisp-science-agent-workflow", "wisp-science-agent-workflow-create",
    "wisp-science-map-items", "wisp-science-research-assistant", "wisp-science-remote-web",
    "wisp-science-sealos-remote-web", "wisp-science-lab", "wisp-science-cli",
    "wisp-science-acp",
]


def rewrite_url(value, source, articles):
    if urlsplit(value).scheme or value.startswith(("//", "#")):
        return value
    resolved = urlsplit(urljoin(source.relative_to(DOCS).as_posix(), value))
    target = DOCS / resolved.path
    if target in articles:
        path = target.stem + ".html"
    elif target.suffix == ".md":
        return REPOSITORY + urlunsplit(resolved)
    else:
        path = posixpath.relpath(resolved.path, "tutorials")
    return urlunsplit(("", "", path, resolved.query, resolved.fragment))


def localized(tag, zh, en, attributes=""):
    return (f'<{tag}{attributes} data-text-zh="{escape(zh)}" data-text-en="{escape(en)}">'
            f'{escape(zh)}</{tag}>')


def article_shell(directory):
    """Reuse the directory's site header/footer, with paths relative to articles."""
    head = directory.split("  <main>", 1)[0]
    footer = directory.split("  </main>", 1)[1]

    def rebase(match):
        attribute, value = match.groups()
        if not urlsplit(value).scheme and not value.startswith(("//", "#")):
            value = "../" + value
        return f'{attribute}="{value}"'

    head, footer = [re.sub(r'(href|src)="([^"]+)"', rebase, part) for part in (head, footer)]
    head = head.replace("tutorial-index", "tutorial-detail")
    head = head.replace('data-page="tutorials"', 'data-page="tutorial-article"')
    head = head.replace('aria-current="page"', 'aria-current="location"')
    return head, footer


def render_article(source, articles, language):
    parser = MarkdownIt("commonmark", {"html": False}).enable("table")
    tokens = parser.parse(source.read_text(encoding="utf-8"))
    if not tokens or tokens[0].type != "heading_open" or tokens[0].tag != "h1":
        raise ValueError(f"{source} must begin with a title")
    title = tokens[1].content
    prefix, separator, short = title.partition("：" if language == "zh" else ":")
    short = short.strip() if separator else title
    category = (prefix.replace("Wisp Science", "").strip() if separator
                else ("教程" if language == "zh" else "Tutorial"))
    tokens = tokens[3:]
    # Rewrite parsed links/images only; keep code examples intact.
    for token in tokens:
        for child in token.children or []:
            attribute = {"link_open": "href", "image": "src"}.get(child.type)
            if attribute:
                child.attrSet(attribute, rewrite_url(child.attrGet(attribute), source, articles))
            if child.type == "image":
                child.attrSet("loading", "lazy")
                image_path = (DOCS / "tutorials" / urlsplit(child.attrGet("src")).path).resolve()
                if image_path.is_file() and image_path.suffix.lower() == ".png":
                    with image_path.open("rb") as image_file:
                        header = image_file.read(24)
                    if len(header) == 24 and header[:8] == b"\x89PNG\r\n\x1a\n":
                        width, height = struct.unpack(">II", header[16:24])
                        child.attrSet("width", str(width))
                        child.attrSet("height", str(height))
        if token.type == "table_open":
            token.attrSet("class", "doc-table doc-table-compact")
    body = parser.renderer.render(tokens, parser.options, {})
    table_label = "教程表格" if language == "zh" else "Tutorial table"
    body = body.replace('<table class="doc-table doc-table-compact">',
                        f'<div class="table-wrap" tabindex="0" role="region" aria-label="{table_label}">'
                        '<table class="doc-table doc-table-compact">')
    body = body.replace("</table>", "</table></div>")
    return {"title": title, "short": short, "category": category, "body": body,
            "source": REPOSITORY + source.relative_to(DOCS).as_posix()}


def render_tutorials(directory=None):
    if directory is None:
        directory = (DOCS / "tutorials.html").read_text(encoding="utf-8")
    sources = sorted((DOCS / "wechat").glob("*.md"), key=lambda path: (
        READING_ORDER.index(path.stem) if path.stem in READING_ORDER else len(READING_ORDER),
        path.name,
    ))
    if not sources:
        raise ValueError("No tutorials found in docs/wechat")
    translations = [source.parent / "en" / source.name for source in sources]
    for translation in translations:
        if not translation.is_file():
            raise ValueError(f"Missing English tutorial: {translation}")
    all_sources = sources + translations
    entries = [(source.stem, render_article(source, all_sources, "zh"),
                render_article(translation, all_sources, "en"))
               for source, translation in zip(sources, translations)]
    cards = {group: [] for group, _, _ in TUTORIAL_GROUPS}
    for number, (anchor, zh, en) in enumerate(entries, 1):
        group, category_zh, category_en = next(
            (item for item in TUTORIAL_GROUPS if item[1] == zh["category"]),
            TUTORIAL_GROUPS[-1],
        )
        cards[group].append(
            f'<a class="tutorial-card" id="{anchor}" href="tutorials/{anchor}.html">'
            f'<span class="tutorial-card-meta"><span class="tutorial-number">{number:02d}</span>'
            + localized("span", category_zh, category_en) + '</span>'
            + localized("h3", zh["short"], en["short"])
            + '<span class="tutorial-read" data-i18n="tutorials.read">阅读教程</span></a>'
        )
    sections = []
    for group, zh, en in TUTORIAL_GROUPS:
        if cards[group]:
            sections.append(
                f'<section class="tutorial-group" id="{group}" aria-labelledby="{group}-title">\n'
                + localized("h2", zh, en, f' class="tutorial-group-title" id="{group}-title"')
                + '\n<div class="tutorial-cards">\n' + "\n".join(cards[group])
                + '\n</div>\n</section>'
            )
    before, rest = directory.split(START)
    _, after = rest.split(END)
    pages = {"tutorials.html": before + START + '\n'
             + "\n".join(sections) + "\n" + END + after}
    head, footer = article_shell(directory)
    for index, (anchor, zh, en) in enumerate(entries):
        zh_title = zh["short"] + " · 教程 | Wisp Science"
        en_title = en["short"] + " · Tutorials | Wisp Science"
        article_head = re.sub(r"<title>.*?</title>", lambda _: f"<title>{escape(zh_title)}</title>", head)
        article_head = re.sub(r'<meta name="description" content="[^"]*">',
                              lambda _: f'<meta name="description" content="{escape(zh["title"])}">', article_head)
        article_head = article_head.replace('data-page="tutorial-article"',
            f'data-page="tutorial-article" data-title-zh="{escape(zh_title)}" '
            f'data-title-en="{escape(en_title)}" data-desc-zh="{escape(zh["title"])}" '
            f'data-desc-en="{escape(en["title"])}"')
        back = (f'<a class="tutorial-back" href="../tutorials.html#{anchor}" '
                'data-i18n="tutorials.back">返回教程目录</a>')
        siblings = []
        for offset, key, label in [(-1, "previous", "上一篇"), (1, "next", "下一篇")]:
            target = index + offset
            if 0 <= target < len(entries):
                sibling_id, sibling_zh, sibling_en = entries[target]
                siblings.append(f'<a class="tutorial-{key}" href="{sibling_id}.html">'
                                f'<span data-i18n="tutorials.{key}">{label}</span>'
                                + localized("strong", sibling_zh["short"], sibling_en["short"]) + '</a>')
        pages[f"tutorials/{anchor}.html"] = (
            article_head + '  <main class="tutorial-reader container">\n'
            f'<nav class="tutorial-breadcrumb" aria-label="教程导航" data-i18n-aria="tutorials.readerNav">{back}</nav>\n'
            f'<article class="tutorial-article" id="{anchor}" aria-labelledby="article-title">\n<header>'
            + localized("h1", zh["title"], en["title"], ' id="article-title"')
            + f'\n<a href="{escape(zh["source"])}" data-href-zh="{escape(zh["source"])}" '
            f'data-href-en="{escape(en["source"])}" data-i18n="tutorials.source">查看原文</a></header>\n'
            + f'<div class="tutorial-body lang-zh" lang="zh-CN">\n{zh["body"]}</div>\n'
            f'<div class="tutorial-body lang-en" lang="en">\n{en["body"]}</div>\n</article>\n'
            '<nav class="tutorial-pagination" aria-label="相邻教程" data-i18n-aria="tutorials.pagination">'
            + "".join(siblings) + f'</nav>\n<div class="tutorial-reader-back">{back}</div>\n'
            '  </main>' + footer
        )
    return pages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for name, updated in render_tutorials().items():
        page = DOCS / name
        if args.check:
            if not page.exists() or page.read_text(encoding="utf-8") != updated:
                raise SystemExit(f"{name} is out of date; run python3 docs/build_tutorials.py")
        else:
            page.parent.mkdir(parents=True, exist_ok=True)
            page.write_text(updated, encoding="utf-8")


if __name__ == "__main__":
    main()
