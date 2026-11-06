# SwiftUI 同步台账

WebView（`ui/src/`）与 SwiftUI（`apps/macos`）是两套界面。只在 WebView 落地的改动，
过去常常只同步了 `native-english.json`，SwiftUI 实际并没有跟上（见
[2026-10-19 分析](superpowers/plans/2026-10-19-macos-swiftui-ui-analysis.md)）。

规则：PR 改了 `ui/src/`，却没有改 `apps/macos/Sources/` 或 `apps/macos/Tests/`
（只重新同步 `Resources/` 不算），就必须在下表加一行。CI 的 `Native Apps /
parity-ledger` 检查这一点（`scripts/check_native_parity.py`）。

状态取值：

- **不需要**：改动与 SwiftUI 无关，例如 WebView 专属的修复、只影响 Leptos 的重构、纯测试。
- **待补**：SwiftUI 应该跟上但尚未实现。补上的 PR 把状态改为 **已补** 并写上 PR 号。
- **已补**：SwiftUI 已有对应实现。

新行加在表格最上面。逐页差异的完整记录见
[2026-10-29 对比](superpowers/plans/2026-10-29-swiftui-webview-ui-diff.md)。

| WebView 改动 | SwiftUI 状态 | 说明 |
| --- | --- | --- |
| DOCX 预览显示原生插入／删除修订；本地可写文档支持逐条接受、拒绝，以及按删除／改写批量接受，保存前核对版本并保留备份 | 已补 | SwiftUI `NativeDocxReviewPreview` 接入原生文件审阅命令，复用 WebView 的保存、备份与版本校验；列表和 WebKit 正文同时刷新。远程文件与历史产物快照只读；已补共享 DTO、原生模型及 WebKit 渲染／按钮测试。 |
| 侧栏底部：「能力」「反馈」「设置」改为一行图标按钮（悬停提示与无障碍名称保留文字），位于技能 / MCP / 记忆统计行下方；紧凑侧栏中竖排。侧栏「更多」改用圆形省略号图标，右侧箭头平时更淡 | 不需要 | WebView 侧栏外观（`sidebar.css` 的 `.side-foot-actions`，新图标 `circle-ellipsis`）；SwiftUI 侧栏使用原生列表与工具栏，没有这一组底部按钮 |
| 界面降噪：右侧「文件」面板位置选择与路径合为一行（根目录不再显示 `.`），操作收成一行图标按钮（上传保留文字，说明改为悬停提示），文件行改为无边框紧凑列表；右侧标签条被滚出的一侧渐隐；侧栏、中间顶栏、右侧标签条统一 48px 高度，底部分割线连成一条；中间与右侧标签按内容宽度左对齐，选中标签改为无阴影的细边框；打开文件时的路径子栏改为无边框图标按钮，「在文档旁边对话」开启时为浅色底而非实心按钮；顶栏「切换面板」开启时为中性底色；只有对话标签时显示为标题；欢迎语改用界面无衬线字体；侧栏当前项改为平铺底色；右侧面板宽度拖动后记住；输入框占位提示缩短 | 不需要 | 均为 WebView 外观与布局（`explorer.css`、`right-pane.css`、`--toolbar-h`、`rightPaneWidth`）。SwiftUI 的文件页已隐藏「文件位置」标签并使用原生列表与工具栏；占位提示改的是 `composer.placeholder`，SwiftUI 输入框用自己的「请输入问题…」 |
| 示例只读会话仅保留复制消息；能力弹窗每次打开重新读取当前工作区快照 | 待补 | WinUI 示例已限制为复制，本轮统一 WebView 并修正 WinUI 本地路径样式、右栏启动器、空状态和子智能体模型选择。SwiftUI 仍需核对示例消息操作与能力快照刷新；资源同步不代表 macOS 已验证。 |
| 右侧面板按对话分别记忆：每个对话有自己的开关状态、工具标签和当前标签，切换对话时恢复该对话的右侧栏；新对话的右侧栏默认关闭（未开对话时打开的右侧栏归属于由此新建的对话）。切换对话不再自动切到「产物」 | 待补 | WebView 内存中按 frame id 暂存（`RightPaneState` / `right_pane_after_switch`），与中间标签、侧边对话的暂存方式一致；SwiftUI 的检查器需按对话保存同样的状态 |
| 右侧「文件」面板：从访达拖入文件时读取系统拖拽剪贴板中的路径并复制到当前目录；读不到路径时提示改用「上传」（#1674） | 不需要 | 修的是 WebView 主窗口关闭 Tauri 原生拖拽后 DOM drop 拿不到路径；新命令 `read_drag_file_paths`，文案键 `files.drop_paths_unavailable`。SwiftUI 直接接收 AppKit 拖拽，不受影响 |
| 右侧面板：关闭后再打开，恢复上次打开的工具标签和当前标签，只有没有打开任何工具时才显示导航首页；切换会话时只在「产物」已打开时才切到它 | 待补 | WebView 启动器的重开行为（`right_tab_to_reopen`）；SwiftUI 尚无导航首页，补导航首页时一并按此行为实现 |
| `/share` PNG：在 #1678 按 DOM 行盒逐行光栅化的基础上，改为按词 / 中文单字落在各自的 DOM 位置、按字体基线对齐，跳过 KaTeX 的隐藏 MathML 副本，代码长行在块内折行；新增「出图方式」单张长图 / 多张 3:4 分页，分页只在行间断开（#1677） | 待补 | 光栅化修复（含 #1678）只在 WebView（`ui/src/api.js`），SwiftUI 用 `ImageRenderer` 出图，不受影响；多张分页需要 SwiftUI 分享页加同样的选项，保存走新命令 `save_share_images`（`pngsBase64` + `defaultName`，返回各页路径），文案键 `share.layout_*` / `share.pages_hint` / `share.export_pages` / `share.saved_pages` |
| 接入 Magpie 时，输入框、侧边对话、审查模型和子智能体模型的选择只列出 `Magpie · <模型>`（ACP Agent 保留），隐藏 Fast；设置 › 模型顶部提示个人模型断开后才生效 | 待补 | 读 `magpie_model_info`；SwiftUI 的模型选择尚未识别 Magpie，仍列出个人模型。文案键 `composer.magpie_managed`、`models.magpie_connected` |
| 技能安装：位置读取失败时显示错误、禁用安装，并忽略过期的位置响应（#1680） | 待补 | SwiftUI 仍需核对本地和项目两种安装位置的失败提示与重试。 |
| 项目、工作区与设置的 Windows 原生布局对齐；WinUI 与 WebView 共用会话分享渲染器（#1680） | 不需要 | 此轮对照针对 Windows。SwiftUI 不使用这个浏览器渲染器；共享资源更新不代表已验证 macOS 界面。 |
| 浏览器离线横幅 / 扩展更新横幅：点「一键安装引导」或需手动重载时，横幅内联显示扩展路径和分平台的「加载已解压的扩展程序」步骤（macOS 为 ⌘⇧G 前往文件夹） | 待补 | 文案键 `browser.offline.step_*`；SwiftUI 横幅尚无步骤列表 |
| 设置 › 远程接入 › 网页远程访问：联机链接二维码 | 待补 | `RemoteAccessStatus` 新增 `qr`（SVG data URL），SwiftUI 读同一字段即可 |
| 右侧面板：仅显示实际打开的工具标签，不再预置四个默认标签 | 不需要 | 修复 WebView 启动器的标签初始化与重开行为 |
| 右侧面板：浏览器式工具导航首页、常用入口与更多工具分组 | 待补 | WebView 已支持导航首页与返回，SwiftUI 尚未同步 |
| Agents › 工作流：实时任务进展统计、分段进度条与状态动画 | 待补 | WebView 已基于现有任务快照展示，SwiftUI 尚未同步 |
| 本地环境：安装到 Wisp（uv、Python、R、Node 一键安装、进度、停止/重试）（#1663） | 待补 | 命令 `get_toolchain_status` / `install_toolchain` / `cancel_toolchain_install`，事件 `toolchain-install` |
| 会话菜单 › 重载项目规则：提示改为同时提到 `~/.wisp/AGENTS.md`（#1662） | 不需要 | 读取 `~/.wisp` 在后端，两端共用；SwiftUI 没有这个入口，只同步了 `native-english.json` |
| 设置 › 专家：默认子智能体模型（#1638） | 已补 | #1660 |
| 设置 › 模型：决策模型标签页 | 已补 | #1656 |
| 右侧面板：运行（#1547）、备忘录（#1567） | 已补 | #1658 |
| 设置 › 插件：启用开关 | 已补 | #1655 |
| 设置 › 插件："启用并使用" | 待补 | |
| 设置 › 技能：社区技能、从 GitHub 添加、标签筛选、批量启停 | 待补 | |
| 设置 › 远程接入：网页远程访问、Wisp Lab 课题组 | 待补 | |
| 设置 › 用量：仪表盘（热力图、环形图、调用排行） | 待补 | |
| 设置 › 模型：ChatGPT Codex (legacy) 账号、每账号网络代理 | 待补 | |
| 工作流全屏编辑器（库 / 画布 / 配置面板） | 待补 | SwiftUI 目前是设置列表 + 画布弹窗 |
| 顶栏：回退历史、视图切换（完整记录 / 模型视图） | 待补 | |
| 侧栏：已收起的对话、子会话折叠、分组拖动排序 | 待补 | |
| 输入框选项：先计划、子智能体模型、计算环境 | 待补 | |
