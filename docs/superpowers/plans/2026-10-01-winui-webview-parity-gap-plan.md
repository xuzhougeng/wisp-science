# WinUI 3 与 WebView 功能差异与补齐计划

日期：2026-10-01。基线：`main` `8d20bd25`（v1.16.0）。对照双方：安装版 WebView 宿主 `wisp-tauri.exe` 1.16.0（`%LOCALAPPDATA%\wisp-science`）与 `scripts/build_native_windows.ps1` 当日发布的 `target\native-windows\Wisp.Science.Preview.exe`。证据：实机同项目同会话（跨物种单细胞根 / FigS5）逐界面走查截图（`test-results/gui-compare/`），加两份全量代码清单。本轮只做差异审计与计划，不含实现。

## 结论

WinUI 3 预览已覆盖**设置（20 类全原生编辑）、会话核心闭环、工作区骨架与安全语义（不发重放）**；与 WebView 的差距集中在五块：**阅读渲染、输入区能力、面板深度、运行时控制台、全局外围能力**。宿主 `native_*` 命令白名单与 DTO 大部分已就绪，可以按"契约已具备 → 先做；需新协议 → 单独立项"的顺序拆小 PR，与 [SwiftUI 对齐计划](2026-09-28-swiftui-webview-next-ui-alignment.md)共用批次语义和[输入区接口核查](../specs/2026-09-28-native-reading-and-composer-contract.md)结论。

## 已对齐（不再立项）

- 设置全部 20 类原生编辑器、草稿保留/重复保存拒绝/未知字段保留、OAuth 取消、密钥走宿主 keyring 参数；排版（UI/代码字号）绑定原生控件。
- 会话闭环：创建/重命名/置顶/删除/存在性、发送、单个排队后续、停止、工具审批、模型切换、附件（staged + 随发）、草稿、不确定结果不重放、Escape 栈。
- 工作区骨架：会话分组/排序/多选移动、文件（浏览/新建/重命名/删除/编辑保存基线）、终端（本地/按环境，输出为纯文本）、大纲/分享/轨迹/归档/待查看 sheet、研究日历（隐私门）、研究历程、论文证据初始创建、收藏库、能力摘要跳设置、反馈预填、随手一聊。

## 差异矩阵（实机观察 + 源码确认）

| 页面 | WebView 实际能力 | WinUI 现状 | 差距定性 |
| --- | --- | --- | --- |
| 会话阅读 | 流式 Markdown、公式 KaTeX、表格、代码高亮+复制、任务列表、内联图片/生成图卡片、usage 紧凑行、思考折叠、压缩/回退标记 | Markdig 默认管线：表格退化为竖线文本、代码无高亮无复制、图片成 `[图片]` 字面量、usage 显示原始 JSON、无公式 | B1/B2 批：结构化渲染，最大可见差距 |
| 输入区 | @/#/ 结构化引用、slash 命令、发送选项（Cut-in/打断替换/侧聊/新分支）、Agent 选项（Plan-first/全权限/委派/完成策略/失败分析/专家/审阅模型/环境运行时）、后续问题建议、上下文占用表 | 附件 + 单排队 + 模型下拉 + 发送/停止 | C 批：契约大多缺失，见 spec 接口核查表 |
| 消息动作 | 复制/编辑/分支/撤销轮次/记忆/复审/开始探索 | 引用 + 收藏 | C 批附带：copy 与已有契约（rename/pin/delete）优先 |
| 面板 | 产物类型分组/计数/富预览（图片/PDF/DOCX/XLSX/PPTX/3D 分子/MSA）、notebook 单元输出、agents 委派与图编辑、hosts 运行记录 | 8 页签骨架；产物为文件名+SHA 文本行、无预览；agents 仅审批/运行/取消；provenance/hosts/sidechat 需手动展开 | D1/D2 批：先图像预览与分组，富 viewer 另议 |
| 运行时 | 运行控制台、runtime 启停/执行代码、plots | `NativeContextActivityClient` 契约在、无 UI | E 批：纯接线 |
| 研究页面 | 历程按日组织+日志编辑+运行下钻；论文证据锚定/定稿检查/复现/capsule | 历程为日期范围表单；论文证据明确只做初始创建（#1342 排除项） | F 批：历程深化先行；论文证据深化需产品决策 |
| 终端 | xterm.js 完整 VT、按会话页签 | 纯文本 + 字面转义码；`ResizeAsync` 未接 | G 批 |
| 全局 | 命令面板 Ctrl+P、chat 内查找 Ctrl+F、i18n、新窗口、会话导入（Codex/Claude/归档）、导出项目、分享 PNG、更新检查/自更新、桌宠窗口、通知、onboarding、隐私模式、示例项目、MCP App（Motif）、自定义 CSS | Ctrl+K 搜索（仅标题匹配）/Ctrl+R/Ctrl+O；无上述其余项 | H 批：逐项决策，部分明确不进原生 |
| 首页 | 相对时间、完整项目动作菜单、诊断 | 结构接近；卡片动作精简、无示例项目入口 | 低优先 |

## 补齐批次（每批一个可测试抽象，小 PR）

按"先可见正确性、后信息密度、外观最后"排序；契约状态依据 spec 接口核查表。

### 实施状态（2026-10-01 同日完成 W1–W5）

- **W1 已实现**：`TranscriptView.RenderMarkdown` 拆分流式块（表格独立 Grid、代码独立高亮块）；Markdig 启用 pipe tables/task lists/auto links；`NativeCodeHighlight`（纯 C# tokenizer，python/r/bash/c/cpp/rust/js/ts/cs/go/java/sql/json/toml/yaml/xml/html/diff，未知语言与超长输入回退纯文本）；代码块带语言标签与"复制"；usage 行走 `TranscriptPresentation.UsageSummary`（与 WebView 同措辞：`输入 x · 输出 y tokens · 缓存 z · 思考 w`）；markdown 本地图与 `view_image` 工具结果按路径内联渲染（不取网络图）。公式仍回退源码，另立选型决策。
- **W2 已实现**：产物按 `NativeArtifactGroups` 类型分组并显示计数；产物/文件预览渲染宿主返回的 base64 图片（`image/*`，异步加载 + 缓存）；笔记本单元显示源码与可折叠输出（错误默认展开）；agents 面板新增说明卡与委派状态卡（Get/Set 委派，失败不重试）；页签条改为可横向滚动。
- **W3 已实现**：消息"复制"动作；会话行"⋯"菜单接入 `native_conversation_rename/pin/delete`（trim 校验、失败不重试、删除需确认）；`BrowserSession` 补可选 `pinned`（wisp-service 项目级查询已带该字段），侧栏"已置顶"分组在 none/date/folder 三种模式下都置于最前；旧宿主无 pin 字段时置顶动作禁用。
- **W4 已实现**：`WorkspacePanelModel` 接入 `INativeContextActivityClient`：hosts 页签显示运行时状态卡（停止/重启/丢弃）与运行记录（取消/收取/详情 stdout/stderr），只读态隐藏写操作；执行面板（context+python/r+代码）走 `runtime_execute`，显示输出文本与本地 plot 图（绝对路径存在才渲染，相对路径留待后续）。查看状态只调 `activity` 读命令，不顺便启动运行时。
- **W5 已实现（第一批）**：输入区"环境"按钮跳转 hosts 面板（真实状态在 W4 的 hosts 页签内）；slash 命令子集客户端路由 `/upload /files /outline /share /trajectory /archive /library /calendar /journey /publication /settings /scratch`，仅映射已有原生界面；未识别命令保留草稿并提示可用命令。Plan/Fast/结构化引用仍按 spec 需先补 DTO，未实现。

每批交付物：C# 变更 + `Wisp.ProjectBrowser.ContractTests` 用例（`dotnet run --project apps/windows/Wisp.ProjectBrowser.ContractTests -- contracts/project-browser/v1/projects.json`）+ 实机 WinUI 验收记录 + 更新 `docs/native-windows-parity.md`。本轮新增 `NativeTranscriptTests`（usage 格式、tokenizer、图片路径）与 `NativeWorkspaceActionsTests`（产物分组、置顶分段、重命名/置顶/删除语义）。

### W6 决策（2026-10-01 记录）

| 决策项 | 决定 | 理由与后续 |
| --- | --- | --- |
| 终端 VT 渲染 | **复用 WebView2 受控渲染**，不原生重写 VT 解析 | xterm.js 与 terminal.js 已在仓库且随宿主分发；WebView2 运行时本就是设置宿主依赖。立项 W6-T1：WinUI 内嵌隐藏 WebView2 表面，桥接 `native_conversation_terminal_*` 契约 |
| MCP App / 科学 viewer（KaTeX、PDF、3D 分子、MSA） | **复用 WebView2 受控渲染**，不原生重写 | viewer 栈（pdf.js/3Dmol/RDKit/KaTeX）无法在纯原生侧以合理成本复刻；仓库已有沙箱桥协议。立项 W6-T2：WinUI WebView2 控件加载仓库静态 viewer 页，数据走本地桥；不连外网 |
| 全文搜索 | 暂缓，待宿主命令 | 需要新增原生白名单命令（`search_sessions` 入口）与 DTO；与 SwiftUI 共用契约后并入 W5 第二批 |
| 会话导入（Codex/Claude/归档） | 暂缓，待宿主命令 | 原生白名单无导入命令；属 SwiftUI 共用范围 |
| 分享 PNG 导出 | 暂缓 | 与 macOS 同为后续；可行路径是 WebView2 离屏渲染 share HTML（W6-T4） |
| 订阅账号登录 UI | 暂缓 | 宿主有 `codex_subscription_*` 命令但不在原生白名单；需要 DTO + 白名单 + 实机账号验收（专用测试账号） |
| 桌宠 / 桌面通知 | 暂缓 | 宠物窗口为独立 Tauri 窗口；原生通知可用 AppNotification，但先不做双实现 |
| 命令面板（Ctrl+P 动作面板） | 暂缓 | Ctrl+K 搜索已覆盖主要导航；动作面板需先统一 action 语义 |

排序建议不变：W6 各项按 T1→T2 优先（复用型、收益最大），其余待宿主/产品决策。

## 明确约束

- 不破坏"不确定结果不重放"语义；Escape 栈顺序（flyout → sheet → 父级）不回退。
- 图标沿用共享资源同步（`sync_native_design.py`），不新增字体/emoji 图标。
- Windows/macOS 行为差异显式化；测试不依赖真实 SSH/GPU/网络。
