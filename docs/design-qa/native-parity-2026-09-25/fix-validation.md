# 修复后的 Computer Use 验收

2026-09-25，约 18:52–19:00（Asia/Shanghai）。使用 CUA 原生 Accessibility/截图和键盘鼠标操作。应用为 `target/native-macos-qa/Wisp Science QA.app`，版本 1.14.0，构建标记 `65feaa8357ea78132bfdfe84297deebef601ae97` / dirty。这份记录只覆盖已构建的项目批，不将它当作随后订阅/ACP 修改的验收。

数据库为 `/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`。项目、消息、文件均为审计合成 fixture。没有发送模型请求、登录外部账户或连接远程计算资源。

| 场景 | 操作与观察 |
| --- | --- |
| 最后窗口 | 打开 HTTP 富文本对比，输入中文草稿；Cmd+W 关闭后通过 CUA 应用 reopen 恢复相同项目/会话/草稿。红叉关闭再 reopen 同样通过。重开前后 QA shell PID 28239、helper PID 28275 不变；仅一个该 QA 包的 helper。未将 CUA reopen 声称为分别点过 Finder 与 Dock。 |
| 发送偏好 | QA 设置中启用“使用 ⌘Enter 发送”，保存后返回，提示切换为“⌘Enter 发送 · Enter 换行”；按 Enter 后追加第二行，原草稿和已存消息保留。恢复偏好 off 并保存，Shift+Enter 可追加第三行。没有按发送快捷键；真实 IME 与发送路径仍待测。 |
| Escape | 设置保存尚在完成时立即返回，出现未保存提示；立即 Escape 仅关闭该提示、保留设置页。ZIP 导出的系统 Save Panel 打开后立即 Escape，只关闭系统对话框，父级导出 sheet 保留。同步 sheet 的 Escape 返回项目列表。 |
| Markdown | 实机截图确认标题、粗体/斜体、代码、表格与任务状态有结构。公式仍显示源文本，与剩余范围一致。 |
| ZIP 导出 | 系统 Save Panel 选择临时目录，完成状态显示 `/private/tmp/wisp-parity-fixes-20260925/cua-project-export-20260925.zip`。独立 ZIP CRC 校验通过，包含 manifest、项目数据库和工作区文件，共 8 个条目。 |
| 目录导出 | 完成状态显示 `/private/tmp/wisp-parity-fixes-20260925/cua-project-directory-20260925`；目录内包含 `manifest.json`、`metadata/project.sqlite`、`workspace/` 及合成结果文件。 |
| 目录导入保护 | 首次选择器没有选中子目录，父目录被校验拒绝；随后用 Go to Folder 进入确切导出根，UI 可见 manifest/metadata/workspace。打开时识别同一项目，显示“这个项目已经在这台设备上”，没有重复登记。尚不将此计为新项目成功登记的 CUA 证明。 |
| 项目同步 | 合成项目启用文件夹快照，显示本地版本已保存、revision；再次手动同步显示“已经是最新版本”。关闭面板后项目卡片显示“已保存到项目文件夹”和 18:59 同步时间。未执行冲突覆盖。 |

实机发现刷新后的旧导入错误仍显示，而且错误面板把操作失败误称为列表数据过期。已修改：成功刷新后清除已关闭导入表单的旧错误；只有列表读取失败时才显示过期说明。保留仍打开的导入表单错误、失败刷新时的错误和“不自动重试”行为；新增回归测试。此项需要新 QA 包复验。

自动化补充：订阅 12 项隔离认证测试，ACP 8 项交互/渲染测试，scoped Rust resolver 3 项；最新全套 Swift 254 passed、DTO 61 passed、WebView Playwright 870 passed / 2 skipped。Rust workspace 前次终端启动超时的全套结果仍须重跑，不能用定向通过替代。

## 干净构建后的复验

约 19:08–19:10，标准 QA 构建与严格签名验证通过，版本 1.14.0，revision `e3a35fcdf843f6248a2a5c109d384d2e8a3937bc`，dirty=false。日志 `/tmp/wisp-parity-auth-acp-build.log`。退出旧 shell 并停止本次 QA helper 后，从原隔离数据库重启新包。

- 模型设置的快速接入区显示 ChatGPT Plus / Pro；打开后可见模型 ID、显示名称、浏览器/设备码方式、开始登录与禁用的保存按钮。未登录时没有伪造已登录状态。
- 打开后立即 Escape 只关闭订阅 sheet，模型设置页保留。另一次展开登录方式菜单后 Escape 只关闭菜单，订阅 sheet 保留；再次 Escape 才返回模型设置页。没有发起真实认证、保存账户或读取明文凭据。
- 再次选择同一导出目录，重复登记提示仍正确。关闭导入 sheet 后首页保留操作错误，但不再声称项目列表过期；点击刷新成功后该错误消失，3 个项目保持不变。
- ACP 最新交互代码已打包；真实进程会话创建/恢复/发送尚未接入，因此未声称完整 ACP 运行通过 CUA。

## 历史恢复闭环

约 19:11–19:13，新建隔离合成工作区 `/private/tmp/wisp-parity-fixes-20260925/cua-history-recovery`，包含一份有效的两条消息归档和一份无效归档。

1. CUA 选择“从旧工作区恢复历史”；预览准确显示 1 个会话、2 条消息、1 份有效归档、跳过 1 份无效归档。
2. 不移动焦点立即 Escape，只关闭恢复预览，父级导入选项保留。读取源目录确认预览没有创建 `.wisp/project.toml`。
3. 再次预览，命名“CUA 历史恢复验收”并确认；应用打开新会话，显示合成用户问题及“样本 A 计数为 12”的回答，工作区路径与选定目录一致。
4. 恢复前后 `session.json` 的 SHA-256 相同。没有模型调用、远程任务或源归档覆盖。
