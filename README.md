<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.svg" />
  <source media="(prefers-color-scheme: light)" srcset="docs/assets/wordmark-light.svg" />
  <img src="docs/assets/wordmark-light.svg" alt="Wisp Science" width="360" />
</picture>

# Wisp Science

**The open-source, local-first AI research workbench.**

WISP — *Workspace for Intelligent Scientific Practice*

<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/github/v/release/xuzhougeng/wisp-science" alt="Release"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/github/downloads/xuzhougeng/wisp-science/total" alt="Downloads"></a>
<a href="https://doi.org/10.5281/zenodo.22009273"><img src="https://zenodo.org/badge/1285857639.svg" alt="DOI"></a>
<a href="https://github.com/xuzhougeng/wisp-science/blob/main/LICENSE"><img src="https://img.shields.io/github/license/xuzhougeng/wisp-science" alt="License"></a>
<a href="https://github.com/xuzhougeng/wisp-science/stargazers"><img src="https://img.shields.io/github/stars/xuzhougeng/wisp-science?style=social" alt="Stars"></a>
<br>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/Windows-supported-0078D4" alt="Windows supported"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/macOS-supported-000000" alt="macOS supported"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/Linux-supported-FCC624" alt="Linux supported"></a>

[English](README.md) · [简体中文](README_zh.md) · [Website](https://wispscience.com/) · [Releases](https://github.com/xuzhougeng/wisp-science/releases)

<img src="docs/assets/app-home.png" alt="Wisp Science desktop app running a bundled RNA-seq analysis demo" width="100%" />

</div>

Search the literature, run Python and R, query ~80 scientific databases, and
keep the trail — figures, runs, decisions, and drafts — in one project. Your
data, conversations, and credentials stay on your machines.

Bring your own model. Keep your science.

## What you can do

**An agent that does the work**

Bring OpenAI-compatible or Anthropic models, or drive Codex / Claude Code over
ACP. The agent reads and writes project files, runs shell, and loads reusable
Skills (`SKILL.md`) without flooding the prompt. Approval gates stay on unless
you opt into Full Permission.

**Compute from laptop to remote servers**

Persistent Python and R kernels keep variables across cells and turns; each
conversation gets its own isolated kernel, so parallel sessions never share
state. Register local, WSL, and SSH hosts once; probe hardware; submit
long **Runs** with live logs. Keys live in the OS keyring, never in SQLite.

**Built for science**

PubMed, GEO, and ~80 other databases through bundled MCP servers. Offline
previews for notebooks, PDFs, Office files, and images. Isolated
[explorations](docs/exploration-branches.md) to try a direction without
touching the mainline. A [Publication Workspace](docs/publication-evidence.md)
that freezes manuscript revisions into verifiable Evidence Capsules.

**A workbench that remembers**

Restart and the full history is back. Undo a turn's file edits. Attach
artifacts, files, and runtimes with `@`; search saved sessions with `#`; apply
a skill with `/`. Encrypted [manual sync](docs/project-sync.md) and
[project transfer](docs/project-transfer.md) — nothing syncs in the background.

## Get started

1. Open the [download page](https://wispscience.com/?lang=en#download), choose your operating system and processor, and download via Cloudflare. [GitHub Releases](https://github.com/xuzhougeng/wisp-science/releases) is also available.
2. Open a bundled demo — no API key needed — to see a full RNA-seq trajectory.
3. Add a model in **Settings → Models** and start a project.

| Platform | Package |
|----------|---------|
| Windows  | Signed MSI / NSIS |
| macOS    | Signed, notarized `.dmg` (Apple Silicon + Intel) |
| Linux    | `.deb` / AppImage (x86_64 + aarch64) |

Setup walkthrough: [Quick Start](docs/wechat/en/wisp-science-quick-start.md) ·
[basic configuration](docs/basic-configuration.md) ·
[model profiles](docs/model-configuration.md) ·
[ACP agents](docs/acp-agents.md)

Build from source, CLI, and architecture: [development](docs/development.md).

## Documentation

| | |
|---|---|
| **Start** | [Basic setup](docs/basic-configuration.md) · [Models](docs/model-configuration.md) · [ACP agents](docs/acp-agents.md) |
| **Research** | [Explorations](docs/exploration-branches.md) · [Evidence capsules](docs/publication-evidence.md) · [Case studies](docs/case-studies.zh-CN.md) |
| **Projects** | [Transfer](docs/project-transfer.md) · [Sync](docs/project-sync.md) · [Global library](docs/global-library.md) |
| **Compute** | [Terminals](docs/terminal-sessions.md) · [Remote files](docs/remote-file-browser.md) · [Transfers](docs/server-transfers.md) |
| **Extend** | [Skills](docs/skills.md) · [Plugins](docs/feature-plugins.md) · [Delegation](docs/agent-delegation.md) · [Channels](docs/channels.md) · [Browser](docs/real-browser-automation.md) |
| **Develop** | [Development](docs/development.md) · [Headless eval](docs/headless-agent-testing.md) |

## Community

Thanks to everyone who filed issues, sent PRs, and used Wisp on real projects.
Special thanks to [SpicyChicken6](https://github.com/xuzhougeng/wisp-science/discussions/1154)
for the molecular wordmark and three-wisp desktop icon designs.

<p>
  <a href="https://github.com/Yu-Qiao-sjtu"><img src="https://avatars.githubusercontent.com/u/88706761?v=4&amp;s=96" width="64" height="64" alt="@Yu-Qiao-sjtu" title="@Yu-Qiao-sjtu"></a>
  <a href="https://github.com/lfz0924"><img src="https://avatars.githubusercontent.com/u/82395287?v=4&amp;s=96" width="64" height="64" alt="@lfz0924" title="@lfz0924"></a>
  <a href="https://github.com/jarxunlai"><img src="https://avatars.githubusercontent.com/u/199478724?v=4&amp;s=96" width="64" height="64" alt="@jarxunlai" title="@jarxunlai"></a>
  <a href="https://github.com/OrigamiSheep"><img src="https://avatars.githubusercontent.com/u/48906039?v=4&amp;s=96" width="64" height="64" alt="@OrigamiSheep" title="@OrigamiSheep"></a>
  <a href="https://github.com/LeeJyee"><img src="https://avatars.githubusercontent.com/u/166231040?v=4&amp;s=96" width="64" height="64" alt="@LeeJyee" title="@LeeJyee"></a>
  <a href="https://github.com/stardustFFF"><img src="https://avatars.githubusercontent.com/u/306053694?v=4&amp;s=96" width="64" height="64" alt="@stardustFFF" title="@stardustFFF"></a>
  <a href="https://github.com/Doctorluka"><img src="https://avatars.githubusercontent.com/u/101385826?v=4&amp;s=96" width="64" height="64" alt="@Doctorluka" title="@Doctorluka"></a>
  <a href="https://github.com/Charlesyu153"><img src="https://avatars.githubusercontent.com/u/232734740?v=4&amp;s=96" width="64" height="64" alt="@Charlesyu153" title="@Charlesyu153"></a>
  <a href="https://github.com/xiaowen621"><img src="https://avatars.githubusercontent.com/u/241900839?v=4&amp;s=96" width="64" height="64" alt="@xiaowen621" title="@xiaowen621"></a>
  <a href="https://github.com/liaoyuan919"><img src="https://avatars.githubusercontent.com/u/240658511?v=4&amp;s=96" width="64" height="64" alt="@liaoyuan919" title="@liaoyuan919"></a>
  <a href="https://github.com/mayunyu925"><img src="https://avatars.githubusercontent.com/u/256124565?v=4&amp;s=96" width="64" height="64" alt="@mayunyu925" title="@mayunyu925"></a>
  <a href="https://github.com/lhx-JIPS"><img src="https://avatars.githubusercontent.com/u/33241642?v=4&amp;s=96" width="64" height="64" alt="@lhx-JIPS" title="@lhx-JIPS"></a>
  <a href="https://github.com/chenzhiyu48"><img src="https://avatars.githubusercontent.com/u/65606400?v=4&amp;s=96" width="64" height="64" alt="@chenzhiyu48" title="@chenzhiyu48"></a>
  <a href="https://github.com/liuyc414"><img src="https://avatars.githubusercontent.com/u/190511200?v=4&amp;s=96" width="64" height="64" alt="@liuyc414" title="@liuyc414"></a>
  <a href="https://github.com/kevinzzzhang76-dot"><img src="https://avatars.githubusercontent.com/u/251931886?v=4&amp;s=96" width="64" height="64" alt="@kevinzzzhang76-dot" title="@kevinzzzhang76-dot"></a>
  <a href="https://github.com/Shawn-Gua"><img src="https://avatars.githubusercontent.com/u/110019576?v=4&amp;s=96" width="64" height="64" alt="@Shawn-Gua" title="@Shawn-Gua"></a>
  <a href="https://github.com/Hayesss"><img src="https://avatars.githubusercontent.com/u/66942436?v=4&amp;s=96" width="64" height="64" alt="@Hayesss" title="@Hayesss"></a>
  <a href="https://github.com/hufanglq"><img src="https://avatars.githubusercontent.com/u/10824450?v=4&amp;s=96" width="64" height="64" alt="@hufanglq" title="@hufanglq"></a>
  <a href="https://github.com/Az-Fan"><img src="https://avatars.githubusercontent.com/u/189823792?v=4&amp;s=96" width="64" height="64" alt="@Az-Fan" title="@Az-Fan"></a>
  <a href="https://github.com/19951219asd"><img src="https://avatars.githubusercontent.com/u/118892832?v=4&amp;s=96" width="64" height="64" alt="@19951219asd" title="@19951219asd"></a>
  <a href="https://github.com/yeshubiao2017-source"><img src="https://avatars.githubusercontent.com/u/233231577?v=4&amp;s=96" width="64" height="64" alt="@yeshubiao2017-source" title="@yeshubiao2017-source"></a>
  <a href="https://github.com/xuxh95"><img src="https://avatars.githubusercontent.com/u/299415390?v=4&amp;s=96" width="64" height="64" alt="@xuxh95" title="@xuxh95"></a>
  <a href="https://github.com/xiaoshen19930901"><img src="https://avatars.githubusercontent.com/u/24424905?v=4&amp;s=96" width="64" height="64" alt="@xiaoshen19930901" title="@xiaoshen19930901"></a>
  <a href="https://github.com/scsksprings"><img src="https://avatars.githubusercontent.com/u/60927616?v=4&amp;s=96" width="64" height="64" alt="@scsksprings" title="@scsksprings"></a>
  <a href="https://github.com/lpc520"><img src="https://avatars.githubusercontent.com/u/61644087?v=4&amp;s=96" width="64" height="64" alt="@lpc520" title="@lpc520"></a>
  <a href="https://github.com/lijianchunChina"><img src="https://avatars.githubusercontent.com/u/42370856?v=4&amp;s=96" width="64" height="64" alt="@lijianchunChina" title="@lijianchunChina"></a>
  <a href="https://github.com/kjiojio"><img src="https://avatars.githubusercontent.com/u/118580250?v=4&amp;s=96" width="64" height="64" alt="@kjiojio" title="@kjiojio"></a>
  <a href="https://github.com/Kururu1799"><img src="https://avatars.githubusercontent.com/u/64822570?v=4&amp;s=96" width="64" height="64" alt="@Kururu1799" title="@Kururu1799"></a>
  <a href="https://github.com/dmh-git-cop"><img src="https://avatars.githubusercontent.com/u/270353192?v=4&amp;s=96" width="64" height="64" alt="@dmh-git-cop" title="@dmh-git-cop"></a>
  <a href="https://github.com/ZZRSCAR"><img src="https://avatars.githubusercontent.com/u/255126066?v=4&amp;s=96" width="64" height="64" alt="@ZZRSCAR" title="@ZZRSCAR"></a>
  <a href="https://github.com/Toomi0124"><img src="https://avatars.githubusercontent.com/u/300393761?v=4&amp;s=96" width="64" height="64" alt="@Toomi0124" title="@Toomi0124"></a>
  <a href="https://github.com/Lezhao0226"><img src="https://avatars.githubusercontent.com/u/72743280?v=4&amp;s=96" width="64" height="64" alt="@Lezhao0226" title="@Lezhao0226"></a>
  <a href="https://github.com/HSsnano"><img src="https://avatars.githubusercontent.com/u/87816341?v=4&amp;s=96" width="64" height="64" alt="@HSsnano" title="@HSsnano"></a>
  <a href="https://github.com/zhuifeng1991"><img src="https://avatars.githubusercontent.com/u/186464554?v=4&amp;s=96" width="64" height="64" alt="@zhuifeng1991" title="@zhuifeng1991"></a>
  <a href="https://github.com/zwbao"><img src="https://avatars.githubusercontent.com/u/24564677?v=4&amp;s=96" width="64" height="64" alt="@zwbao" title="@zwbao"></a>
  <a href="https://github.com/yuzhenpeng"><img src="https://avatars.githubusercontent.com/u/31943277?v=4&amp;s=96" width="64" height="64" alt="@yuzhenpeng" title="@yuzhenpeng"></a>
  <a href="https://github.com/youxiudongdong-lang"><img src="https://avatars.githubusercontent.com/u/306058340?v=4&amp;s=96" width="64" height="64" alt="@youxiudongdong-lang" title="@youxiudongdong-lang"></a>
  <a href="https://github.com/ying-ge"><img src="https://avatars.githubusercontent.com/u/45988974?v=4&amp;s=96" width="64" height="64" alt="@ying-ge" title="@ying-ge"></a>
  <a href="https://github.com/yemiaoyong"><img src="https://avatars.githubusercontent.com/u/61010663?v=4&amp;s=96" width="64" height="64" alt="@yemiaoyong" title="@yemiaoyong"></a>
  <a href="https://github.com/yejia1988"><img src="https://avatars.githubusercontent.com/u/164177661?v=4&amp;s=96" width="64" height="64" alt="@yejia1988" title="@yejia1988"></a>
  <a href="https://github.com/xingzhuo123"><img src="https://avatars.githubusercontent.com/u/167210517?v=4&amp;s=96" width="64" height="64" alt="@xingzhuo123" title="@xingzhuo123"></a>
  <a href="https://github.com/xiaochuheying19901216"><img src="https://avatars.githubusercontent.com/u/304343377?v=4&amp;s=96" width="64" height="64" alt="@xiaochuheying19901216" title="@xiaochuheying19901216"></a>
  <a href="https://github.com/xiahouzuoying"><img src="https://avatars.githubusercontent.com/u/57342415?v=4&amp;s=96" width="64" height="64" alt="@xiahouzuoying" title="@xiahouzuoying"></a>
  <a href="https://github.com/likemoonriver"><img src="https://avatars.githubusercontent.com/u/157043962?v=4&amp;s=96" width="64" height="64" alt="@likemoonriver" title="@likemoonriver"></a>
  <a href="https://github.com/lijianguoa"><img src="https://avatars.githubusercontent.com/u/52228119?v=4&amp;s=96" width="64" height="64" alt="@lijianguoa" title="@lijianguoa"></a>
  <a href="https://github.com/k1600639239"><img src="https://avatars.githubusercontent.com/u/301947158?v=4&amp;s=96" width="64" height="64" alt="@k1600639239" title="@k1600639239"></a>
  <a href="https://github.com/gongmeiyuan"><img src="https://avatars.githubusercontent.com/u/75189860?v=4&amp;s=96" width="64" height="64" alt="@gongmeiyuan" title="@gongmeiyuan"></a>
  <a href="https://github.com/chhhhai"><img src="https://avatars.githubusercontent.com/u/99796066?v=4&amp;s=96" width="64" height="64" alt="@chhhhai" title="@chhhhai"></a>
  <a href="https://github.com/chenchen199401-cmyk"><img src="https://avatars.githubusercontent.com/u/236738705?v=4&amp;s=96" width="64" height="64" alt="@chenchen199401-cmyk" title="@chenchen199401-cmyk"></a>
  <a href="https://github.com/chenhd3"><img src="https://avatars.githubusercontent.com/u/52345106?v=4&amp;s=96" width="64" height="64" alt="@chenhd3" title="@chenhd3"></a>
  <a href="https://github.com/btzheng"><img src="https://avatars.githubusercontent.com/u/15546828?v=4&amp;s=96" width="64" height="64" alt="@btzheng" title="@btzheng"></a>
  <a href="https://github.com/Winteric123"><img src="https://avatars.githubusercontent.com/u/122366825?v=4&amp;s=96" width="64" height="64" alt="@Winteric123" title="@Winteric123"></a>
  <a href="https://github.com/ShixiangWang"><img src="https://avatars.githubusercontent.com/u/25057508?v=4&amp;s=96" width="64" height="64" alt="@ShixiangWang" title="@ShixiangWang"></a>
  <a href="https://github.com/ScholarlyLuck"><img src="https://avatars.githubusercontent.com/u/267531500?v=4&amp;s=96" width="64" height="64" alt="@ScholarlyLuck" title="@ScholarlyLuck"></a>
  <a href="https://github.com/Junweichengang"><img src="https://avatars.githubusercontent.com/u/41681007?v=4&amp;s=96" width="64" height="64" alt="@Junweichengang" title="@Junweichengang"></a>
  <a href="https://github.com/JarningGau"><img src="https://avatars.githubusercontent.com/u/22016330?v=4&amp;s=96" width="64" height="64" alt="@JarningGau" title="@JarningGau"></a>
  <a href="https://github.com/Cloudy-Zhuang"><img src="https://avatars.githubusercontent.com/u/85553170?v=4&amp;s=96" width="64" height="64" alt="@Cloudy-Zhuang" title="@Cloudy-Zhuang"></a>
  <a href="https://github.com/245429488zc-svg"><img src="https://avatars.githubusercontent.com/u/250579619?v=4&amp;s=96" width="64" height="64" alt="@245429488zc-svg" title="@245429488zc-svg"></a>
  <a href="https://github.com/entpyf"><img src="https://avatars.githubusercontent.com/u/125380093?v=4&amp;s=96" width="64" height="64" alt="@entpyf" title="@entpyf"></a>
  <a href="https://github.com/jymtop"><img src="https://avatars.githubusercontent.com/u/248790276?v=4&amp;s=96" width="64" height="64" alt="@jymtop" title="@jymtop"></a>
  <a href="https://github.com/MenghanLi-lab"><img src="https://avatars.githubusercontent.com/u/309669757?v=4&amp;s=96" width="64" height="64" alt="@MenghanLi-lab" title="@MenghanLi-lab"></a>
  <a href="https://github.com/knifer510"><img src="https://avatars.githubusercontent.com/u/37789525?v=4&amp;s=96" width="64" height="64" alt="@knifer510" title="@knifer510"></a>
  <a href="https://github.com/pilaobanmust-sketch"><img src="https://avatars.githubusercontent.com/u/269459169?v=4&amp;s=96" width="64" height="64" alt="@pilaobanmust-sketch" title="@pilaobanmust-sketch"></a>
  <a href="https://github.com/JohnnyChen1113"><img src="https://avatars.githubusercontent.com/u/30077595?v=4&amp;s=96" width="64" height="64" alt="@JohnnyChen1113" title="@JohnnyChen1113"></a>
  <a href="https://github.com/chewice"><img src="https://avatars.githubusercontent.com/u/244145152?v=4&amp;s=96" width="64" height="64" alt="@chewice" title="@chewice"></a>
  <a href="https://github.com/XuuChen"><img src="https://avatars.githubusercontent.com/u/99383234?v=4&amp;s=96" width="64" height="64" alt="@XuuChen" title="@XuuChen"></a>
  <a href="https://github.com/fei0810"><img src="https://avatars.githubusercontent.com/u/17489298?v=4&amp;s=96" width="64" height="64" alt="@fei0810" title="@fei0810"></a>
  <a href="https://github.com/Zac-lzh"><img src="https://avatars.githubusercontent.com/u/223252975?v=4&amp;s=96" width="64" height="64" alt="@Zac-lzh" title="@Zac-lzh"></a>
  <a href="https://github.com/mugpeng"><img src="https://avatars.githubusercontent.com/u/52995448?v=4&amp;s=96" width="64" height="64" alt="@mugpeng" title="@mugpeng"></a>
  <a href="https://github.com/ChrisLou-bioinfo"><img src="https://avatars.githubusercontent.com/u/34942834?v=4&amp;s=96" width="64" height="64" alt="@ChrisLou-bioinfo" title="@ChrisLou-bioinfo"></a>
  <a href="https://github.com/georgeatparallel"><img src="https://avatars.githubusercontent.com/u/297992784?v=4&amp;s=96" width="64" height="64" alt="@georgeatparallel" title="@georgeatparallel"></a>
  <a href="https://github.com/liufahui005"><img src="https://avatars.githubusercontent.com/u/188657823?v=4&amp;s=96" width="64" height="64" alt="@liufahui005" title="@liufahui005"></a>
  <a href="https://github.com/portos-wang"><img src="https://avatars.githubusercontent.com/u/246403081?v=4&amp;s=96" width="64" height="64" alt="@portos-wang" title="@portos-wang"></a>
  <a href="https://github.com/shengxinzhuan"><img src="https://avatars.githubusercontent.com/u/54225560?v=4&amp;s=96" width="64" height="64" alt="@shengxinzhuan" title="@shengxinzhuan"></a>
  <a href="https://github.com/c020627"><img src="https://avatars.githubusercontent.com/u/251123242?v=4&amp;s=96" width="64" height="64" alt="@c020627" title="@c020627"></a>
  <a href="https://github.com/Emberwhirl"><img src="https://avatars.githubusercontent.com/u/5317953?v=4&amp;s=96" width="64" height="64" alt="@Emberwhirl" title="@Emberwhirl"></a>
  <a href="https://github.com/HanWang-kui"><img src="https://avatars.githubusercontent.com/u/306124623?v=4&amp;s=96" width="64" height="64" alt="@HanWang-kui" title="@HanWang-kui"></a>
  <a href="https://github.com/hero199409"><img src="https://avatars.githubusercontent.com/u/296349483?v=4&amp;s=96" width="64" height="64" alt="@hero199409" title="@hero199409"></a>
  <a href="https://github.com/hi-fei-cool"><img src="https://avatars.githubusercontent.com/u/167007368?v=4&amp;s=96" width="64" height="64" alt="@hi-fei-cool" title="@hi-fei-cool"></a>
  <a href="https://github.com/Hongweili0424"><img src="https://avatars.githubusercontent.com/u/139341349?v=4&amp;s=96" width="64" height="64" alt="@Hongweili0424" title="@Hongweili0424"></a>
  <a href="https://github.com/jxshi"><img src="https://avatars.githubusercontent.com/u/28937112?v=4&amp;s=96" width="64" height="64" alt="@jxshi" title="@jxshi"></a>
  <a href="https://github.com/Lin-medical"><img src="https://avatars.githubusercontent.com/u/309001021?v=4&amp;s=96" width="64" height="64" alt="@Lin-medical" title="@Lin-medical"></a>
  <a href="https://github.com/LiuXiao-888"><img src="https://avatars.githubusercontent.com/u/286878566?v=4&amp;s=96" width="64" height="64" alt="@LiuXiao-888" title="@LiuXiao-888"></a>
  <a href="https://github.com/qneurolab"><img src="https://avatars.githubusercontent.com/u/69098252?v=4&amp;s=96" width="64" height="64" alt="@qneurolab" title="@qneurolab"></a>
  <a href="https://github.com/ryys1122"><img src="https://avatars.githubusercontent.com/u/8609374?v=4&amp;s=96" width="64" height="64" alt="@ryys1122" title="@ryys1122"></a>
  <a href="https://github.com/Sanhang-learn"><img src="https://avatars.githubusercontent.com/u/175394997?v=4&amp;s=96" width="64" height="64" alt="@Sanhang-learn" title="@Sanhang-learn"></a>
  <a href="https://github.com/shevenlee"><img src="https://avatars.githubusercontent.com/u/49136350?v=4&amp;s=96" width="64" height="64" alt="@shevenlee" title="@shevenlee"></a>
  <a href="https://github.com/Shipeng-Guo"><img src="https://avatars.githubusercontent.com/u/16771195?v=4&amp;s=96" width="64" height="64" alt="@Shipeng-Guo" title="@Shipeng-Guo"></a>
  <a href="https://github.com/xiaochuheying"><img src="https://avatars.githubusercontent.com/u/304300062?v=4&amp;s=96" width="64" height="64" alt="@xiaochuheying" title="@xiaochuheying"></a>
  <a href="https://github.com/xwttracy-source"><img src="https://avatars.githubusercontent.com/u/243696678?v=4&amp;s=96" width="64" height="64" alt="@xwttracy-source" title="@xwttracy-source"></a>
  <a href="https://github.com/yeungyuenming"><img src="https://avatars.githubusercontent.com/u/231188244?v=4&amp;s=96" width="64" height="64" alt="@yeungyuenming" title="@yeungyuenming"></a>
  <a href="https://github.com/yikeshu0611"><img src="https://avatars.githubusercontent.com/u/33260177?v=4&amp;s=96" width="64" height="64" alt="@yikeshu0611" title="@yikeshu0611"></a>
  <a href="https://github.com/zhaoliang0302"><img src="https://avatars.githubusercontent.com/u/42333702?v=4&amp;s=96" width="64" height="64" alt="@zhaoliang0302" title="@zhaoliang0302"></a>
  <a href="https://github.com/Zulity"><img src="https://avatars.githubusercontent.com/u/33241990?v=4&amp;s=96" width="64" height="64" alt="@Zulity" title="@Zulity"></a>
  <a href="https://github.com/zuolan1999-jpg"><img src="https://avatars.githubusercontent.com/u/293331524?v=4&amp;s=96" width="64" height="64" alt="@zuolan1999-jpg" title="@zuolan1999-jpg"></a>
</p>

Windows installers are Authenticode-signed with an [SSL.com](https://www.ssl.com/)
Personal ID (IV) code signing certificate through eSigner. SmartScreen reputation
builds from download volume. Third-party notices live in
[development](docs/development.md).

## License

[AGPL-3.0-only](LICENSE), except where a directory notes otherwise. Earlier
releases keep the license published with them.

## Citation

[![DOI](https://zenodo.org/badge/1285857639.svg)](https://doi.org/10.5281/zenodo.22009273)

```bibtex
@software{xu2026wisp,
  author    = {Xu, Zhou-Geng},
  title     = {Wisp Science: a local-first AI research workbench},
  version   = {v1.5.0},
  year      = {2026},
  publisher = {Zenodo},
  doi       = {10.5281/zenodo.22009273},
  url       = {https://doi.org/10.5281/zenodo.22009273}
}
```
