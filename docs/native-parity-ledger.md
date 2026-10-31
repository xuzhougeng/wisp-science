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
| 右侧面板：浏览器式工具导航首页、常用入口与更多工具分组 | 待补 | WebView 已支持导航首页与返回，SwiftUI 尚未同步 |
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
