"""Build the bilingual Skills catalog from reviewed summaries and the bundled inventory."""

import argparse
import json
import re
from pathlib import Path

from build_tutorials import localized

DOCS = Path(__file__).resolve().parent
START = "<!-- BEGIN GENERATED SKILLS -->"
END = "<!-- END GENERATED SKILLS -->"
GROUPS = [
    ("literature", "文献与证据", "Literature and evidence"),
    ("analysis", "数据与分析", "Data and analysis"),
    ("figures", "图表与写作", "Figures and writing"),
    ("environment", "计算环境", "Compute environments"),
    ("extension", "扩展与自动化", "Customization and automation"),
]


def render_skills():
    entries = json.loads((DOCS / "skills-catalog.json").read_text(encoding="utf-8"))
    bundled = {p.parent.name for p in (DOCS.parent / "skills").glob("*/SKILL.md")}
    listed = [entry["id"] for entry in entries]
    if len(listed) != len(set(listed)) or set(listed) != bundled:
        raise ValueError("Skills catalog must list every bundled SKILL.md exactly once")
    for entry in entries:
        for field in ["title_zh", "title_en", "summary_zh", "summary_en"]:
            if not entry.get(field, "").strip():
                raise ValueError(f"{entry['id']} needs {field}")
        if entry["group"] not in {group[0] for group in GROUPS}:
            raise ValueError(f"Unknown group for {entry['id']}")
        source = (DOCS.parent / "skills" / entry["id"] / "SKILL.md").read_text(encoding="utf-8")
        name = re.search(r"^name:\s*(.+)$", source, re.MULTILINE)
        if not name or name.group(1).strip(' \"\'') != entry["id"]:
            raise ValueError(f"Skill name does not match directory: {entry['id']}")
    count = len(entries)
    output = ['<section class="doc-hero"><div class="container">',
        localized("div", "让科研方法可以复用", "Reusable research methods", ' class="eyebrow"'),
        localized("h1", "科研技能", "Research Skills"),
        localized("p", f"Wisp Science 内置 {count} 个技能，覆盖文献调研、数据分析、图表制作、环境配置与写作交付。把成熟的步骤和检查要求交给 Agent，让每次任务都有方法可循。", f"Wisp Science includes {count} Skills for literature, analysis, figures, environments, and writing. Give the agent reusable methods and checks to follow on each task.", ' class="lead"'),
        '<div class="hero-actions"><a class="btn btn-primary" href="tutorials/wisp-science-skills.html">'
        + localized("span", "学习使用 Skills", "Learn to use Skills") + '</a>'
        '<a class="btn btn-secondary" href="https://github.com/xuzhougeng/wisp-science/tree/main/skills">'
        + localized("span", "查看技能源码", "Browse Skill sources") + '</a></div></div></section>',
        '<section class="skills-intro"><div class="container"><div class="feature-list doc-card-grid">']
    for zh_title, en_title, zh_body, en_body in [
        ("把方法写进任务", "Give the task a method", "Skills 规定执行步骤、检查事项和交付要求；MCP 提供访问外部工具与数据的连接。两者可以一起使用。", "Skills describe steps, checks, and deliverables. MCP connects external tools and data. The two can work together."),
        ("按任务选择技能", "Choose for the task", "在设置 → 技能中查看说明与启用状态，也可以在输入框输入 /，为下一条消息附加技能。", "Review instructions and enabled status in Settings → Skills, or type / in the composer to attach a Skill to your next message."),
        ("按需准备运行条件", "Prepare what it needs", "内置技能不等于依赖已经安装。使用前请确认所需的解释器、软件包、服务凭据和数据都已就绪。", "Bundled instructions do not install dependencies. Check required interpreters, packages, service credentials, and input data before running a workflow."),
    ]:
        output.append('<div class="feature-list-item">' + localized("h4", zh_title, en_title) + localized("p", zh_body, en_body) + '</div>')
    output.append('</div><nav class="skills-groups" aria-label="技能分类" data-i18n-aria="skills.categories">')
    for group, zh, en in GROUPS:
        output.append(f'<a href="#{group}">' + localized("span", zh, en) + '</a>')
    output.append('</nav>' + localized("p", f"下面列出项目随附的全部 {count} 个技能。你当前项目的可用技能与启用状态，以应用内设置为准；每个条目都链接到对应说明。", f"The catalog below lists all {count} bundled Skills. Check the app for availability and enabled status in your project. Each entry links to its instructions.", ' class="skills-note"') + '</div></section>')
    for group, zh, en in GROUPS:
        members = [entry for entry in entries if entry["group"] == group]
        output.append(f'<section class="skill-section" id="{group}"><div class="container">'
                      '<div class="section-head">' + localized("h2", zh, en)
                      + localized("p", f"{len(members)} 个技能", f"{len(members)} Skills") + '</div>')
        if group == "literature":
            output.append(localized("p", "bear-* 系列已改为按需安装：在设置 → 技能 → 浏览社区技能中选择 BEAR Research Skills 来源。使用前需配置 scimaster-cli（sci）。", "Install the bear-* family as needed from the BEAR Research Skills source in Settings → Skills → Browse community Skills. Configure scimaster-cli (sci) before use.", ' class="skills-note"'))
        output.append('<div class="skills-list">')
        for entry in members:
            skill_id = entry["id"]
            output.append(f'<article class="skill-card" data-skill-id="{skill_id}"><code>{skill_id}</code>'
                          + localized("h3", entry["title_zh"], entry["title_en"])
                          + localized("p", entry["summary_zh"], entry["summary_en"])
                          + f'<a href="https://github.com/xuzhougeng/wisp-science/blob/main/skills/{skill_id}/SKILL.md">'
                          + localized("span", "查看技能说明", "Read Skill instructions") + '</a></article>')
        output.append('</div></div></section>')
    return "\n".join(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    page = DOCS / "skills.html"
    original = page.read_text(encoding="utf-8")
    before, rest = original.split(START)
    _, after = rest.split(END)
    updated = before + START + "\n" + render_skills() + "\n" + END + after
    if args.check:
        if updated != original:
            raise SystemExit("Skills page is out of date; run python3 docs/build_skills.py")
    else:
        page.write_text(updated, encoding="utf-8")


if __name__ == "__main__":
    main()
