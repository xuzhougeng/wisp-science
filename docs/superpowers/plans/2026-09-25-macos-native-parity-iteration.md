# macOS SwiftUI / WebView 差异审计与下一轮迭代

日期：2026-09-25。源码基线：`4272578e7780066c7048ade9652b8cb9f111bfd4`（#1360，workspace 1.14.0）。

本文是 Computer Use 对比与实现计划，不是功能交付记录。证据分为 **实机观察**、**源码确认**、**待验证**；存在入口不等于完整工作流已经对齐。

结论：原生的界面入口和基本导航已覆盖很多日常功能，下一步的主要差距在回答的表达质量、输入行为一致性，以及已有入口后的完整操作链。建议先修复窗口恢复、发送偏好与 block Markdown，再推进项目文件夹、认证/ACP 和论文证据操作。

## 基线和方法

- 本机安装的 WebView 为 1.12.0，原生预览二进制构建于 9 月 23 日；这些旧应用只用于定位，不作为 HEAD 的验收依据。
- 为本次比较重建独立 identifier 的 SwiftUI 与 WebView QA 应用，使用相同内容的两份隔离数据库和工作目录。测试模型只指向本地无服务端口，ACP 命令指向不存在的测试路径，不调用真实模型、SSH 或云服务。
- 使用 CUA 操作真实 macOS 应用，结合 accessibility tree 和截图观察。源代码审阅用于定位原因，以及标记本次无法现场验证的差异。
- 当前标准原生构建在 `sync_native_design.py --check` 阶段失败：`native-english.json` 缺少新增的 19 个 ChatGPT 订阅登录文案键。QA 构建临时跳过该检查，使用 HEAD 已签入资源；未修改产品源码。因此 QA 包可用于中文功能对比，不能算标准构建链通过。
- `sync_native_settings_contract.py --check` 通过。主原生 app 的 Info.plist 仍为 0.1.0；helper 才写入产品版本，不能用 About 版本判断代码新旧。
- 完整 QA 两包均构建并通过严格签名验证。数据库最终位于 `/private/tmp/wisp-parity-20260925-data/{native,webview}/wisp.sqlite`。初次放在 Documents 时，GUI 派生的 service 阻塞于系统 `open()`，shell 读取正常；迁移后正常。该现象疑似 macOS 文件访问授权上下文差异，作为测试环境记录，不当成已确诊的产品缺陷。
- 两版使用同一组合成会话/论文。首次启动会自动增加默认工作区，WebView 另有示例项目和 onboarding；不以首页项目数/活动排序变化推断缺陷。本次使用各自默认窗口尺寸、中文浅色，未完成窄窗、深色、英文或性能评测。
- [截图与基线记录](../../design-qa/native-parity-2026-09-25/README.md)保留了配对界面、构建标识和检查步骤。

## Computer Use 实测结果

| 编号 | 操作 | 实际观察 | 判定 |
|---|---|---|---|
| M01 | 打开相同 HTTP 富文本会话 | 原生保留 `#`、表格分隔符、任务列表和公式源码，fenced code 变为一行；WebView 有标题层级、真正表格、代码块、任务勾选和公式 | **实测差异，P1** |
| M02 | 原生常规设置查看发送偏好，再回输入框输入两行 | “使用 ⌘Enter 发送”为 off；输入第一行后按 Enter 仍换行，第二行留在同一草稿中，未发送 | **实测缺陷，P1**；测试草稿已清空 |
| M03 | 打开原生 ACP 富文本会话 | 显示“该会话为只读…请在 WebView 中继续”，模型/附件禁用 | **原生实测 + WebView 源码确认，P1**；未启动真实 ACP |
| M04 | 两版打开导入入口，均取消 | 原生直接进入归档文件选择器，目录无法作为导入对象；WebView 提供项目文件夹、ZIP、历史恢复三个选项 | **实测差异，P1**；未执行真实导入 |
| M05 | 打开已有两论文、每篇两 revision 的项目 | 原生只显示选中论文、revision 和三条结构文本；WebView 提供论文/版本选择、创建版本、添加条目/证据、定稿检查和版本记录 | **实测差异，P2**；未操作冻结/证据写入 |
| M06 | 打开模型设置 | 原生只有添加 API 接入；WebView 多出 ChatGPT Plus/Pro 按钮 | **实测差异，P1**；未登录外部账户 |
| M07 | 关闭完整原生包最后窗口，Finder 双击同一 app，再用 CUA 定位 | 原生仍无可访问窗口，定位超时；Cmd+Q 后重新启动恢复首页。WebView 点击关闭后由 Finder 打开仍可显示首页 | **本机复现，P1**；应增加 macOS 应用生命周期 smoke，不能只测视图 |
| M08 | 原生 Cmd+K，立即 Escape；添加 API 编辑器打开后立即 Escape，再按一次 Escape | 搜索正确关闭；第一次只退出编辑器，设置仍在；第二次退出设置，父论文页面仍在 | **这些路径通过**，不将 Escape 栈笼统列为缺失 |
| M09 | WebView 会话操作菜单 | 有重命名、置顶、跨项目复制/移动、导出、删除；原生相应缺口由源码交叉确认 | **WebView 实测 + 原生源码确认，P2** |

M07 的复现路径为 `Cmd+W → Finder 打开 app → CUA getApp 超时 → Cmd+Q → 重新启动成功`。源码对应：SwiftUI `WindowGroup` 且替换了默认 new-item；WebView 有关闭隐藏与 `RunEvent::Reopen` 处理。未将根因仅归咎于缺少某个 delegate 方法，也未声称已覆盖所有 macOS 版本或 Dock 路径。

## 已有能力，避免重复规划

原生已具备首页/项目/保存会话导航、搜索、设置 19 个大类、HTTP 对话与审批、附件和单条后续排队，以及轨迹、通知、研究归档、分享、终端、文件与环境/Run/Agent 面板。Notebook、Highlights、Provenance、SideChat、研究日历、收藏库、研究历程、论文证据基础入口也已有实现。

这里的“已有”是实现范围说明，不代表本次逐项完成真实服务验收。`native-conversations.md` 的早期“附件/排队未实现”、`native-project-browser.md` 的早期“sidebar slots disabled”等历史描述已不能代表 HEAD。

## 源码确认的剩余差异

| 范围 | SwiftUI 当前行为 | WebView 当前行为 | 用户影响 |
|---|---|---|---|
| 回答渲染 | 正文仅 inline Markdown；工具正文为纯文本 | block Markdown、表格、任务列表、数学及代码/表格复制 | 科研答案结构和可读性下降 |
| 发送快捷键 | 设置可编辑 `send_with_modifier`，主 composer 固定 Cmd+Enter | 按偏好使用 Enter 或 modifier+Enter | 设置与实际行为不一致 |
| ACP 主会话 | read-only，模型列表仅 HTTP | ACP 对话、停止与问题回复 | 部分已有会话无法迁移到原生继续 |
| 项目可移植性 | 导入只接受 ZIP；项目卡片缺目录导出与同步状态入口 | ZIP/项目文件夹导入，目录导出与网盘快照状态 | 新的 workspace-owned project 工作流无法完整使用 |
| ChatGPT 订阅 | 原生模型设置无新增订阅登录入口 | #1360 已接入登录/状态/重新登录 | 新增认证能力尚未跨客户端 |
| 会话管理 | 选择、分组、移动已具备；无会话重命名/删除入口 | 重命名、删除/批量管理、分支标记 | 日常整理需回 WebView |
| 论文证据 | 首篇创建；已有论文以标题、版本、条目文本展示 | 论文/版本选择、证据管理、检查/冻结 | 有入口但不能完成投稿准备链 |
| 问题卡片 | 点选覆盖现有草稿；未接入 pending/answered/expired 和内联自由输入 | 保留草稿，携带说明，展示问题状态 | 用户输入可能被覆盖，旧选项仍可能看似可操作 |
| 计划反馈 | 通用允许/拒绝，未发送已有 `feedback` 字段；plan 消息无专用展示 | 修改反馈、结构化计划模式决策 | 用户难以局部修订计划 |
| 高级输入/上下文 | 主 composer 为文本、附件、模型、排队/停止；无同等 @/#/slash 和上下文操作 | 引用、命令、上下文信息与手动压缩 | 高频操作路径较长或缺失 |
| 文件预览 | 文本等宽原文，binary 使用 Quick Look | 依类型使用不同 viewer | Markdown/表格与科学文件的阅读体验不同 |

关键证据（路径均相对于仓库）：

- `apps/macos/Sources/WispProjectBrowserUI/NativeConversationView.swift:93,119,154,193,199`：正文、问题卡、ACP 提示、快捷键。
- `apps/macos/Sources/WispProjectBrowserUI/NativeSettingsView.swift:154` 与 `ui/src/main.rs:4978`：发送偏好的读写/应用差异。
- `apps/macos/Sources/WispProjectBrowserUI/ProjectBrowserModel.swift:308`、`ProjectBrowserView.swift:312`；`ui/src/app_support/projects.rs:906`、`ui/src/app_overlays.rs:284`：项目导入/导出。
- `apps/macos/Sources/WispProjectBrowserUI/NativeModelSettings.swift:19,94`；`ui/src/settings_view.rs:4493`：订阅登录。
- `apps/macos/Sources/WispProjectBrowserUI/ProjectWorkspace.swift:299`；`ui/src/sidebar.rs:385,554`：会话管理。
- `apps/macos/Sources/WispProjectBrowserUI/NativePublication.swift:46,140`；`ui/src/publication.rs:483,642`：论文工作区。
- `apps/macos/Sources/WispProjectBrowserUI/NativeConversationModel.swift:249`、`crates/wisp-dto/src/native_conversations.rs:284`、`src-tauri/src/approval_commands.rs:240`：反馈字段已经存在于后端，Swift UI 未使用。

不要把代理工作流审批与主对话 plan mode 混为一谈：前者原生已经支持带 `expected_version` 的审批。两端的公共 question schema 都没有结构化多选，本次不将多选列为原生独有缺口。

## 下一轮 PR 顺序与验收

每个 PR 只交付一个可验证行为。下表为建议顺序，并不要求把整批放进一个版本。

| 顺序 | PR 范围 | 完成标准 |
|---|---|---|
| 0 / P1 | 修复共享资源漂移与构建标识 | 两项同步检查通过；标准脚本完整构建；主 app 和 helper 显示一致版本并可追溯 commit；更新当前能力矩阵 |
| 0b / P1 | 最后窗口关闭后的恢复 | Cmd+W/红叉后从 Finder、Dock 重开有可用窗口；恢复合理项目/会话；不重复启动 host；Cmd+Q 能正常退出；用应用级 smoke 覆盖 |
| 1 / P1 | 主 composer 尊重发送偏好 | Enter / Cmd+Enter 两种设置与实际发送一致；Shift+Enter、IME 候选确认不误发；草稿切换保留；提示文案同步 |
| 2 / P1 | 原生 block Markdown v1 | 同 fixture 的标题/列表/引用/表格/代码结构正确，代码/表格可复制，长行不撑破窗口；选择引用与收藏保持正确；公式/图片可后续独立 PR |
| 3 / P1 | 问题卡片保护已有草稿 | 点选不吞掉未发送文字；回答/过期状态正确；自由输入与问题关联；测试旧问题与迟到响应 |
| 4 / P1 | 计划拒绝反馈 | 原生提供修改意见，复用既有 feedback DTO；拒绝原文可达 agent；过期 ID 不误批；无自动重试；Escape 只关闭最上层反馈编辑 |
| 5 / P1 | 项目文件夹导入 | 与 ZIP 并列提供文件夹选择，验证清单和冲突，取消无写入；新旧项目格式兼容；先不混入导出/网盘逻辑 |
| 6 / P1 | 项目目录导出与状态 | 导出目录单独 PR；之后再做网盘快照状态。只使用本地临时目录测试，不要求真实云盘 |
| 7 / P1 | 原生 ChatGPT 订阅入口 | 使用已有认证和 keyring 路径；覆盖未登录/待确认/成功/过期/取消；不增加新的 token 存储方式；自动化使用 fake transport |
| 8 / P1 | ACP 主会话最小闭环 | 分为“会话创建/恢复”“发送/停止”“审批/ask-user”小 PR；不可用功能提供具体原因；fake ACP 进程覆盖重连和取消 |
| 9 / P2 | 会话重命名与管理 | 先重命名，再独立处理删除/批量动作与分支导航；保持当前会话、分组、排序一致，失败保留状态 |
| 10 / P2 | 论文/版本导航 | 首先能切换已有论文和 revision；之后分别做条目/证据编辑、检查、冻结；不在一个 PR 重建整个 publication workbench |
| 11 / P2 | 输入引用、上下文与文件阅读 | @/#/slash 分别按类型实施；先只读上下文快照后压缩动作；Markdown/CSV 预览优先，科学 viewer 单独定义支持表 |
| 12 / 正式交付前 | 原生壳的安装、首次使用与更新 | 单独规划签名/公证/安装包、主壳与 helper 版本一致性、整体升级与失败恢复；定义新用户引导和高级数据库选择入口，不沿用开发预览作为发布验收 |

建议第一批先完成构建修复、窗口恢复、发送偏好、block Markdown 这四个小 PR；问题卡片和计划反馈紧随其后。第二批做项目迁移与模型接入；最后推进管理与论文证据的完整操作链。若实际主要依赖 ACP，则将 8 提前到第二批首位。

## 持续验收约束

- 每项行为配套共享 DTO/模型测试，UI/Tauri 改动执行仓库要求的 Rust、wasm 与 Playwright 检查，并运行相关 Swift 测试。
- 自动化不得依赖真实 API key、SSH、GPU、WSL、调度器或云盘。使用现有 toolbar fixture、mock bridge、fake transport/ACP 和临时目录。
- 真实 AppKit smoke：打开后立即 Escape，无需先移入焦点；一按仅关最上层；测试编辑器内菜单、文件 sheet、IME 候选层。
- 同 fixture、同内容尺寸保留两版配对截图；至少覆盖桌面/窄窗、浅/深色、中/英文。现有 `WISP_NATIVE_SNAPSHOT_DIR` 可复用，但 CI 默认跳过；PNG 字节数检查不能替代布局/操作可达性验收。
- 单独建立关闭最后窗口、重新激活、恢复项目/会话的生命周期测试；覆盖本次 M07 复现，并进一步验证 Dock 与不同 macOS 版本。
- 性能暂不宣称胜负。先对相同 35-turn fixture 重复记录首次打开、切换、滚动与 idle CPU，再按基线制定阈值。

## 平台差异与正式交付边界

原生标题栏、系统菜单、文件选择器、字体栅格化与 Quick Look 可以保留 macOS 的交互方式，不需要逐像素复刻 WebView。优先统一内容层级、状态语义和操作结果。本次原生侧栏、设置页与首页已具有共同信息架构；正文和论文证据的差异则超出了外观风格。

WebView 的首次启动引导已实际浏览（欢迎、能力说明、模型配置、可选本地环境）；原生以数据库浏览预览直接启动。当前原生仍是本地预览包，设置中的更新作用于 desktop host，不能等同于完整 SwiftUI 壳更新。这些正式交付差异由 `docs/native-project-browser.md` 和 `docs/native-settings.md` 交叉确认，应在准备原生正式发布时单独安排，不与本轮高频交互修复混成一个大 PR。

## 边界

本轮仅新增审计文档、截图和构建记录，不实现功能、不发布版本、不创建远端 PR。未执行完整 Rust/Playwright/Swift 测试套件；实际执行了隔离构建、签名校验、资源/契约检查和上述 Computer Use smoke。未经实际操作确认的能力保留“源码确认/待验证”标签。没有真实模型轮次、OAuth 登录、SSH、Run 执行或完整科学分析，因此本报告不代表这些流程已通过端到端验收。
