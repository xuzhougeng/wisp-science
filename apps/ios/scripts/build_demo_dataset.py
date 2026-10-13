#!/usr/bin/env python3
"""Build the checked-in, synthetic offline demo. Requires matplotlib only here;
the iOS app reads the resulting bundle without Python or external services.
"""
import csv
import json
from pathlib import Path
import textwrap

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[1] / "WispScience/DemoData"
FILES = ROOT / "files"
SAMPLES = [
    ("Control_01", "control", 24.8, 96.2, 93.8, 12.4),
    ("Control_02", "control", 25.3, 95.8, 94.1, 11.8),
    ("Control_03", "control", 23.9, 96.5, 93.5, 13.1),
    ("Treatment_01", "treatment", 26.1, 95.4, 94.3, 12.0),
    ("Treatment_02", "treatment", 24.5, 96.1, 93.9, 12.7),
    ("Treatment_03", "treatment", 25.7, 95.9, 94.0, 11.5),
]


def write(path, text):
    target = FILES / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(textwrap.dedent(text).strip() + "\n", encoding="utf-8")


def main():
    for directory in ("data", "results", "scripts"):
        (FILES / directory).mkdir(parents=True, exist_ok=True)
    with (FILES / "data/samples.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream)
        writer.writerow(["sample_id", "group", "replicate"])
        writer.writerows((row[0], row[1], index % 3 + 1) for index, row in enumerate(SAMPLES))
    with (FILES / "data/qc_metrics.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream)
        writer.writerow(["sample_id", "group", "read_pairs_million", "mapped_pct", "q30_pct", "duplicate_pct"])
        writer.writerows(SAMPLES)

    total = sum(row[2] for row in SAMPLES)
    mapping = sum(row[3] for row in SAMPLES) / len(SAMPLES)
    table = "| 样本 | Read pairs (M) | 比对率 | Q30 |\n| --- | --- | --- | --- |\n"
    table += "\n".join(f"| {r[0]} | {r[2]:.1f} | {r[3]:.1f}% | {r[4]:.1f}% |" for r in SAMPLES)
    write("README.md", """
    # Wisp Science 离线演示

    这是随 App 打包的合成 RNA-seq 项目，不包含真实受试者、患者或用户数据。
    数字、对话、命令日志和分析结论均用于界面演示，没有运行实际测序分析。

    ## 可以体验
    浏览对话和 Run，打开 CSV、Markdown 报告与图表，发送消息获取明确标注的本地预设回复，体验批准与拒绝操作。

    [查看质控报告](results/qc_report.md)

    [查看样本表](data/samples.csv)

    ## 演示边界
    不访问远程主机，不调用模型，不执行脚本，不上传照片。交互只保存在内存。
    在演示设置中可重置数据；退出演示、重置或重新启动后恢复初始内容。
    连接自己的 Wisp 主机后才能运行真实任务。
    """)
    report = f"""# RNA-seq 质控报告

**合成演示数据 · 非真实实验结果**

本报告展示 3 个 control 和 3 个 treatment 样本的阅读体验，不支持实际科学结论。
共有 {total:.1f} M read pairs，平均比对率约 {mapping:.1f}%。

## 样本概览

{table}

## 比对率图表

![Synthetic demo — mapping rate](qc_summary.png)

## 演示结论

6 个示例样本的比对率均高于示例阈值 90%。这是预先设定的合成数值，不能据此判断真实实验质量。
真实分析还需要结合实验设计、文库类型、污染、批次效应及下游用途进行评估。

[查看原始示例指标](../data/qc_metrics.csv)

[查看示例脚本](../scripts/qc_demo.py)
"""
    write("results/qc_report.md", report)
    write("scripts/qc_demo.py", '''# Synthetic demo only. Displayed by the app, never executed by it.
import csv
from pathlib import Path

rows = list(csv.DictReader(Path("data/qc_metrics.csv").open()))
for row in rows:
    print(row["sample_id"], row["mapped_pct"], "synthetic mapping %")
''')

    plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 12})
    fig, ax = plt.subplots(figsize=(9, 5.2), layout="constrained")
    fig.set_facecolor("#FFFFFF")
    colors = ["#276F65"] * 3 + ["#6986B5"] * 3
    bars = ax.barh([row[0] for row in SAMPLES], [row[3] for row in SAMPLES], color=colors, height=0.62)
    ax.invert_yaxis()
    ax.set_xlim(0, 108)
    ax.set_xticks([0, 25, 50, 75, 100], ["0", "25", "50", "75", "100"])
    ax.set_xlabel("Mapped read pairs (%)")
    ax.set_title("RNA-seq mapping overview", loc="left", fontsize=18, fontweight="bold", pad=34)
    ax.text(0, 1.06, "SYNTHETIC DEMO DATA  /  6 SAMPLES  /  NO REAL ANALYSIS", transform=ax.transAxes, color="#64706C", fontsize=10)
    ax.bar_label(bars, labels=[f"{row[3]:.1f}%" for row in SAMPLES], padding=7, fontsize=11)
    ax.xaxis.grid(True, color="#E5EBE8")
    ax.set_axisbelow(True)
    ax.tick_params(axis="both", length=0, pad=8)
    for spine in ax.spines.values():
        spine.set_visible(False)
    fig.savefig(FILES / "results/qc_summary.png", dpi=160, facecolor="white")
    plt.close(fig)

    def message(role, text):
        return {"role": role, "text": text}

    manifest = {
        "schema": "wisp.ios-demo.v1",
        "project": {"id": "demo-rnaseq", "name": "RNA-seq 质控 · 演示"},
        "sessions": [
            {
                "id": "demo-qc", "title": "样本质控与报告", "approvals": [],
                "items": [
                    message("user", "请汇总这 6 个样本的质控结果，并给出报告和图表。"),
                    message("assistant", "# 示例质控已整理\n\n**离线演示 · 以下内容是预先打包的合成数据。**\n\n"
                            f"6 个样本，共 {total:.1f} M read pairs，平均比对率约 {mapping:.1f}%。\n\n"
                            + table + "\n\n![演示样本比对率](results/qc_summary.png)\n\n"
                            "[打开完整报告](results/qc_report.md) · [查看样本表](data/samples.csv)\n\n"
                            "可展开下方运行记录查看示例日志，或打开项目文件浏览报告。"),
                ],
                "run_cards": [
                    {"id": "demo-validate", "title": "检查样本表与分组", "status": "succeeded",
                     "command": "python scripts/qc_demo.py", "stdout_tail": "[DEMO] Loaded 6 synthetic samples: 3 control + 3 treatment.\nNo command was executed by this app."},
                    {"id": "demo-report", "title": "汇总质控报告与图表", "status": "succeeded",
                     "stdout_tail": "[DEMO] Bundled outputs: results/qc_report.md, results/qc_summary.png\nRead pairs: 150.3 M; mean mapping: 96.0%."},
                ],
                "follow_ups": ["查看完整质控报告", "如何体验分析流程与审批？"],
            },
            {
                "id": "demo-workflow", "title": "分析流程与审批",
                "items": [
                    message("user", "我想了解远程分析前的审批流程。"),
                    message("assistant", "# 先确认，再执行\n\n**这是本地交互演示。**\n\n真实远程任务执行命令或写文件前，会在对话中请求你的批准。下方展示示例脚本和审批卡片。\n\n你可以点击「批准一次」或「拒绝」。两种操作都只改变本地演示状态，不会运行脚本或连接服务器。\n\n[查看示例脚本](scripts/qc_demo.py)"),
                ],
                "run_cards": [{"id": "demo-retry", "title": "读取示例输入文件（失败记录演示）", "status": "failed",
                               "stderr_tail": "[DEMO] Example input not found: missing_sample.csv\nThis is a bundled example of a failed run, not a live error."}],
                "approvals": [{"approval_id": "demo-approval", "frame_id": "demo-workflow", "tool": "shell（演示）",
                               "message": "体验一次命令审批。批准或拒绝都不会实际执行命令。", "preview": "python scripts/qc_demo.py"}],
                "follow_ups": ["查看完整质控报告"],
            },
            {
                "id": "demo-options", "title": "问题与选项", "approvals": [], "run_cards": [], "follow_ups": [],
                "items": [
                    message("assistant", "**本地选项演示**\n\n点选选项后点击「提交回答」。选择和补充说明只保存在本次演示中，不会调用 AI 或执行分析。"),
                    message("question", json.dumps({
                        "v": 1, "source": "native",
                        "question": "这组演示样本的质控结果已经整理好。接下来希望如何查看？请选择一项，也可以填写自己的要求。",
                        "options": [
                            {"label": "按建议标准查看", "description": "查看每个样本的比对率、Q30 和分组汇总，并打开预先打包的质控报告。"},
                            {"label": "只查看汇总报告", "description": "直接阅读报告和图表，暂时跳过逐样本详情。"},
                            {"label": "使用自定义标准", "description": "在下方补充你关注的指标或阈值，例如希望重点查看哪些样本。"},
                        ],
                        "allow_freeform": True,
                        "note": "Question submitted; it is now waiting for the user. End your turn here — the answer arrives as the next user message.",
                    }, ensure_ascii=False, separators=(",", ":"))),
                ],
            },
        ],
        "files": [{"path": str(path.relative_to(FILES))} for path in sorted(FILES.rglob("*")) if path.is_file()],
    }
    (ROOT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
