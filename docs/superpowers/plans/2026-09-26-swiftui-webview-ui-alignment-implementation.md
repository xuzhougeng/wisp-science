# SwiftUI / WebView UI 对齐实施记录

基于 `b242fcbd`，执行 [Computer Use 对齐计划](2026-09-26-swiftui-webview-ui-alignment.md)。本轮是可见布局、阅读与正文产物的实现批次，不将长期功能 parity 标记为完成。

## 本轮改动

- 工作区：248 pt 侧栏、40 pt 普通导航行、白色选中会话；64 pt 标题栏、标签样式标题；改名/置顶/删除归入原生会话菜单。保留既有操作的后端边界与确认流程。
- 输入：占位提示、圆形附件入口、右侧模型与发送操作；64–160 pt 内容自适应。高度测量与真实 NSTextView 完全分离，保持草稿、光标、IME 和发送键策略。
- 阅读：增加段落/列表间距、代码容器、表格留白和引用左边框；代码/表格可见复制按钮具有辅助功能标签，保留原生跨块选择及右键菜单。图标从 `compose_icon()` 同步，未引入第二套图标。
- 产物：`NativeTranscriptArtifact` 是当前消息页的只读表格/块公式投影，不新增数据库表、不登记假文件、不扩展 IPC DTO。面板计数合并登记文件与消息投影；表格可预览，公式明确标注为 LaTeX 源码；预览支持立即 Escape。
- 设置：首页文档入口及最近会话徽标；常规页区分工作区交互和通知更新，发送快捷键改为明确的选择项，增加取消入口，环境置于网络前。设置文案进入已有本地化资源导出。

## 已执行的实机检查

应用：`target/native-macos-qa/Wisp Science QA.app`；隔离数据库 `/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`。使用 `cua_repl`，没有发送模型请求或主动执行导入、同步、删除、保存设置。QA 项目本身已启用文件夹快照，宿主生命周期可更新其快照时间。

1. 打开“HTTP 富文本对比”，新工作区布局和占位提示显示正常。
2. 打开会话菜单，立即按一次 Escape，仅菜单关闭，会话与侧面板保留。
3. 同一合成正文的面板从 0 项变为 2 项：表格 1、公式 1。打开表格预览，表格内容为 A=12/B=24；立即 Escape 关闭预览，面板保留。最终包的公式卡片也已实际打开，显示 `y = \alpha + \beta x` 源码，立即 Escape 正常。
4. 初轮实机发现输入高度测量会污染文本容器宽度，造成空输入区难以点击、多行草稿逐字换行。已修复并增加独立测量与宿主视图布局测试；重建签名包后实际点击空输入框、输入 ASCII、粘贴中文八行草稿通过。随后全选删除测试草稿，恢复为空。中文直接 `typeText` 未完整输入，因此这里只认定中文粘贴通过，不认定 IME 实机验收通过。
5. 辅助功能树可见“复制代码”和“复制表格”按钮；代码/表格复制的内容和选择保持由自动化验证。
6. 调整窄窗后，实际最小尺寸约 960×746（受应用最小尺寸限制），标题截断，菜单、发送、面板关闭仍可达；没有宣称完成 900×700 窗口条件。
7. 设置分组、发送方式选择和取消入口可见；本轮没有改动用户偏好来验证保存。

实机截图位于本次任务的 Computer Use 工具记录。以下归档图片为真实 SwiftUI 组件的离屏 fixture 渲染，不替代实机截图：

- [浅色 Markdown](../../design-qa/native-ui-alignment-2026-09-26/markdown-light.png)
- [深色窄窗 Markdown](../../design-qa/native-ui-alignment-2026-09-26/markdown-dark-narrow.png)
- [窄窗输入与审批](../../design-qa/native-ui-alignment-2026-09-26/conversation-narrow.png)

## 验证结果

- Swift 最终全套：289 passed（UI 274 + 基础 15），包含全部 opt-in render，日志 `/tmp/wisp-ui-alignment-swift-complete.log`。
- `cargo fmt --all -- --check`、wasm check、设计资源/设置契约同步检查及契约脚本测试通过。
- 标准 QA shell/helper 构建通过；后续仅 Swift 改动刷新 shell 与资源并重新签名，最终 `codesign --verify --deep --strict` 通过。构建标识为 `b242fcbd` + dirty 工作树；不是已发布版本。最终签名 shell 可执行文件 SHA-256：`d6b159ef4c657658643dc7794481802043e3a9063262b69816794f29cea6bb3e`。
- WebView Playwright 全套：870 passed、2 skipped（21.5 分钟）；已执行 `npm ci`，日志 `/tmp/wisp-ui-alignment-playwright.log`。测试生成的既有研究历程图片已恢复，未混入本次改动。
- Rust workspace 全套：2361 passed、0 failed，含文档测试，命令 `WISP_CATALOG_OFFLINE=1 cargo test --workspace` 退出码 0；日志 `/tmp/wisp-ui-alignment-workspace.log`。

本轮曾出现旧引用样式断言失败、测试中的 CGFloat 类型歧义及输入测量测试失败；这些已修复并全套复测，不能把早期失败日志计为通过。

## 明确保留的后续工作

- 全量同 revision、同 fixture 的 WebView/SwiftUI 明暗、中英配对截图与 35-turn 性能基线；本轮 WebView 源码未修改，但运行中的对照包 revision 未完全核验。
- 公式排版、代码语法高亮、真实任务复选框以及完整消息动作行。
- 输入区 Local/Python/R 状态与 Agent/Fast/@/#/slash 的能力接入；不放置无行为按钮。
- 正文 CSV/FASTA/文件引用产物、完整分类/预览器与远程来源统一。
- 其余设置页、首页示例项目/所有项目动作、深色/英文全流程实机验收，以及既有第二阶段可靠性收尾。
