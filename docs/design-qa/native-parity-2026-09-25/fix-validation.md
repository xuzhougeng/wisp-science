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

## ACP 创建与启动边界（19:31–19:40）

构建为 `5719574aa008d788ca992923b03c5502864c5045` / dirty=false，1.14.0；标准 QA 构建与严格签名验证通过，日志 `/tmp/wisp-parity-acp-turn-build.log`。

- 在 QA 设置中添加 `QA Native ACP`，命令为本机 Python，脚本 `scripts/qa_native_acp.py`，限定目录为上述历史恢复测试目录，事件日志为 `/private/tmp/wisp-parity-fixes-20260925/acp-native-events.jsonl`。没有真实模型、网络或研究数据操作。配置保存曾长时间显示处理中；只读 SQLite 检查确认已保存，因此没有重复提交。随后 UI 回到列表显示“已保存”和两个 ACP 配置。此延迟仍需复验，不认定原因。
- 在原 HTTP 历史会话输入合成草稿，模型菜单可见 `ACP · 新会话` 及 `QA Native ACP`；选择后出现新的空会话 `51e67984-2fd1-42ff-982e-f923f383f425`，模型标签正确。尚未切回原会话核验草稿，因此不把自动化的草稿保持测试算作本次 CUA 结论。
- 新建后显示 `Session not found in project`。定位为原生草稿调用旧 saved-history 查询：未命名空会话原本不在该列表中。修复让已知原生草稿由实时 conversation snapshot 读取，导航回归 8 项通过。此修复尚未重建 CUA。
- Enter 发送 `hello native ACP smoke` 后显示处理中；ACP Python 子进程启动，但没有创建事件日志或保存 ACP binding。进程采样显示 Python 阻塞于启动导入阶段的 `open` 系统调用（`/tmp/wisp-parity-acp-peer-sample.txt`），尚不能认定为协议错误，也未确认文件访问等待的根因。
- 点击 Stop 后最终显示“正在停止…”，未自行结束。源码确认 ACP 初始化回复前尚无 `AcpRuntime` 可供 `cancel_frame` 关闭，启动 future 也未观察会话取消标记。已开始修复初始化取消与 actor drop 清理；未以此失败场景宣称发送/停止通过。
- 退出 QA 壳后，核对 PID/命令，终止此次隔离 host 和合成 Python 子进程。没有终止其他应用或 Rust 全量测试。下一次需重新构建并验收普通回复、权限选择、停止、已绑定会话重启恢复；脚本可复制到同一 `/private/tmp` QA 目录，避免验收依赖仓库脚本所在目录的访问状态。

## ACP 与重命名的真实宿主协议验收（20:08–20:10）

Mac 锁屏期间执行本地协议验收，**这不是 CUA 验收**。使用新构建 `fca972e3532f1c9c7589826e1cb6e41e4a4c3a6b` / dirty=false / 1.14.0，严格签名验证通过（`/tmp/wisp-parity-acp-cancel-rename-build.log`）。只运行隔离 QA helper 和本地合成 Python peer，无真实模型/账户/远程任务。

- 将已知 QA peer 的副本放入 `/private/tmp/wisp-parity-fixes-20260925/qa_native_acp.py`，SHA-256 `c302b37211aac6cb5011f21cf7b32841382102fc828303b5d4cb878a34e2acb1`。不再依赖该测试进程读取 Documents 下的仓库脚本；这不证明此前 open 阻塞的根因。
- 使用 authenticated loopback `/invoke` 和明确 project ID，创建合成配置与会话 `b6f70910-0a9c-44ef-8e2f-d3126da6688a`。普通发送完成，snapshot 含回复与持久 ACP profile binding；权限卡可见，选择 exact `allow` 后正常结束，回复写入 transcript。
- 只终止命令行和父 PID 均匹配此次 log/peer 的合成子进程，然后对同一会话继续发送。宿主成功清除 dead cache 并 `session/resume` 到同一 ACP session ID；没有卡在重复加锁处。
- 运行中的合成 wait 回合可停止。另用 `--stall-initialize` 创建 `5917509e-d6c7-423f-925b-a0fcdcbda2f5`，等日志确认 initialize 已收到但没有回复后 Stop；snapshot 恢复 idle，未产生错误 binding，子进程退出，耗时约 0.283 秒。
- 重命名接口接受已归属会话的名称，拒绝另一 project 和空白名称；用只读 `wisp-service list_sessions` 确认保存的名称为 `QA ACP protocol smoke`，首轮消息没有覆盖手动名称。
- 确认所有回合结束后重启隔离 QA helper（43274 → 43632），再次读取同一会话，历史消息/权限结果仍在，发送成功并走 `session/resume`。真实外部 ACP 产品的账户和实现未被测试。

脚本与日志：`/tmp/wisp-parity-native-acp-host-smoke.py`、`/tmp/wisp-parity-native-acp-host-smoke.log`、`/tmp/wisp-parity-native-acp-host-resume.py`、`/tmp/wisp-parity-native-acp-host-resume.log`。peer 事件日志为隔离目录下 `qa-parity-fba149f10b-normal.jsonl` 与 `qa-parity-fba149f10b-startup-stall.jsonl`，仅记录方法和 ACP session ID，不记录 prompt 或 token。QA helper 保持可用供后续 CUA 连接；仍需实机验证输入、卡片操作、重命名入口及窗口恢复。

## 置顶的真实宿主协议验收（20:49–20:50）

仍未进行 CUA。标准 QA 构建及严格签名检查通过（`/tmp/wisp-parity-session-pin-build.log`），shell/helper 均为 1.14.0、revision `60eec4d8`、dirty=true。构建过程中开始了删除批次；此包未包含随后加入的 `native_conversation_exists`，因此只用于本节置顶验收，删除仍须稳定重建。

- 对上述合成会话 `b6f70910-0a9c-44ef-8e2f-d3126da6688a` 设置置顶 true → false → true，每一步都通过单独的只读 `wisp-service` 查询确认保存状态，标题保持 `QA ACP protocol smoke`。
- 使用不同 project ID 取消置顶，以及缺少 pinned / 字符串 pinned，均被拒绝，保存状态仍为 true。
- 确认会话空闲后重启此次 QA helper（48350 → 48979），再次读取仍为 true；没有发送模型消息或修改用户数据。

脚本 `/tmp/wisp-parity-native-pin-host-smoke.py`；日志 `/tmp/wisp-parity-native-pin-host-smoke.log`、`/tmp/wisp-parity-native-pin-host-restart.log`。置顶分区、按钮及导航保留仍需解锁后的 CUA 验收。
