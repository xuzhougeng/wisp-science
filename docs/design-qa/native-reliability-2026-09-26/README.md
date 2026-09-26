# 原生可靠性第二阶段：实机验收记录

本阶段排除 ACP 专项工作。**本轮四项验收证据已收齐**：同步冲突、目录导入、快捷键/快速粘贴与普通审批等待草稿已通过；完整自动化回归及 16 张配对条件截图已收齐，并记录打开/切换/滚动与限定进程范围的 idle CPU。真实中文 IME 由用户完成两种偏好下的人工操作并确认未误发，随后通过 CUA 和数据库核对保留文本及单次显式发送；英文工作区标签与消息 Usage JSON 的显示差异保留为后续问题。没有为了收尾而修改产品行为，也没有把旧分支结果当作本次结果。

## 构建与隔离条件

- 产品代码基线：main `f684c799`（已含 #1361、#1362）；两客户端均构建于 clean `82256473838ddb281e8eb6bd1a758286fc356e0c`，与基线仅阶段计划文档不同。
- Native shell/helper：1.14.0，标准 `scripts/build_native_macos.sh --qa` 构建、严格签名验证通过。WebView UI 与宿主从同一 revision 重建，使用隔离 bundle ID 并通过严格签名验证。见 [builds.json](builds.json)。
- macOS 26.6.2 (25G83)，Apple M1 Max，32 GiB，MacBookPro18,2。机器与采样条件见 [environment.json](environment.json)。
- 原生数据库：`/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`；WebView 数据目录：`/private/tmp/wisp-parity-20260925-data/webview`。各自 QA Application Support 指向隔离目录。未使用真实用户项目、模型服务、云盘或 SSH。
- 输入服务仅绑定 `127.0.0.1`，使用合成凭据，记录最后一条合成用户输入，不保存认证头或系统提示。记录见 [http-inputs.jsonl](http-inputs.jsonl)。

## 1. 同步冲突闭环：通过

项目 `252be348-e105-4391-a530-68db9859102e`，工作区 `/private/tmp/wisp-parity-fixes-20260925/webview-folder-import`。操作通过原生实际菜单与确认框完成；快照检查使用只读 SQLite 和 SHA-256。

| 操作 | 实际结果 |
| --- | --- |
| 冲突 → 采用远端 → 立即 Escape | 只关闭确认，父同步窗口保留 |
| 冲突 → 保留本地 → 取消 | 返回父窗口；两次取消后发布目录文件的 SHA-256 均与备份一致 |
| 暂时移走 peer 快照 → 确认远端 | 明确显示 `project_folder_waiting: the latest version has not finished downloading`，保留当前选择与确认窗口 |
| 恢复缺失文件 | 错误与选择仍保留，没有自动重试 |
| 显式重试 | 成功生成 `f2ea1abc-088a-4736-8ea0-316cb85ae7e6`，只有一个 DAG tip，父节点覆盖两条分支 |
| 不同内容分叉 → 确认保留本地 | 生成 `c361b8d1-b41a-4264-b3a3-113f002a28cd`；description 仍为空，丢弃合成远端标记 |
| 再次不同内容分叉 → 确认采用远端 | 生成 `5f0a0752-ca9b-409e-848e-566ac6a58cc5`；description 为 `REMOTE phase2: adopted via explicit confirmation`，两分支收敛 |

失败截图 [sync-remote-waiting.jpg](sync-remote-waiting.jpg) 显示完整错误，成功截图见 [sync-remote-resolved.jpg](sync-remote-resolved.jpg)。独立读取见 [sync-results.json](sync-results.json)：后两次发布数据库的内容与声明哈希一致；第一次成功的旧 SQLite 在后续操作后已不在 revisions 中，保留其 JSON 父节点记录，不声称可以重新读取已不存在的文件。

同内容分叉只用于取消与缺失文件恢复；不同内容分叉才用于验证本地/远端决策。合成 peer 沿用当前 tip 的父节点、改写测试 description，并重新计算数据库和 portable state hash；计算算法先与生产快照交叉检查。**这不代表真实双设备或云盘并发已验收。**

## 2. 旧格式目录和立即 Escape：通过

有效 fixture 使用 `archive_kind=wisp-project`、`archive_version=1`，包含 `manifest.json`、`metadata/project.sqlite` 和空 `workspace/`。项目 ID 为 `phase2-reliability-20260926-v2`，两个根会话分别有 1 轮和 35 轮消息。

- 从原生导入窗口打开目录选择器，不先移动焦点，立即按 Escape：第一次只关闭选择器，父导入窗口保留。此前旧包的两次 Escape 现象在本次稳定包未复现，未据此编造修复。
- 再选有效旧格式目录并打开：项目登记成功，两会话出现在列表，历史文本、表格、代码可读。截图见 [native-legacy-readable.jpg](native-legacy-readable.jpg)。源 `metadata/project.sqlite` 的 SHA-256 不变。
- WebView 在独立数据目录中导入同一 fixture 的另一份副本，显示导入完成并可读 35-turn 会话。
- 两端 live store 的 35-turn `(seq, role, content)` 各 70 行相同；按 seq 排序，用 Python `json.dumps(rows, ensure_ascii=False)` 的默认分隔符编码为 UTF-8 后计算 SHA-256，得到 `23c9c732b398cfa4a2217c42cbc4ecfb7d816d60c4f44e26c46b8c5a0944757d`，见 [transcript-hash.txt](transcript-hash.txt)。

首次构造的 fixture 根会话 `parent_frame_id` 为 NULL，导致列表为空；这是测试数据构造错误。修正为 `parent_frame_id=root_frame_id=id` 后以全新 project/session ID 导入，未修改产品查询或用无效 fixture 判定产品故障。

### 重复使用相同 fixture

[legacy-35-turn-fixture.zip](legacy-35-turn-fixture.zip) 只含合成项目、72 条合成消息、2 个根会话及 schema migration 记录。它保存本次实际导入的原始文件，不含模型配置或输入 smoke 会话。

1. 为两个客户端分别准备空的隔离 QA 数据目录；不要指向真实 Application Support。
2. 将 ZIP 分别解压到两个全新目录（本项测试选择解压后的**目录**，不是 ZIP 导入入口）。
3. 先运行 `shasum -a 256 <目录>/metadata/project.sqlite`，应等于 [legacy-fixture.json](legacy-fixture.json) 的 `4110168c8978a27a98829f37114e654c805041bb773c3406f542627be1cf8b0c`。
4. 从两个客户端的导入菜单各选一个目录；已有相同 project ID 时应换空 QA store，不要破坏重复登记保护。
5. 打开“旧格式导入验收”，再打开“35-turn 可靠性基线”。第 35 轮应显示 S35、reads=35000、ratio=0.95、`print("S35")`。

归档已重新解压验证：源 SHA-256 一致、foreign_key_check 无错误、两个根会话及 72 条消息完整。

## 3. 输入与草稿：通过（IME 为协作人工验证）

在原生主输入框和本地 HTTP echo 服务之间实际发送，而不是仅检查编辑框显示。

| 项目 | 证据 / 状态 |
| --- | --- |
| 快速全选 → 粘贴新文本 → 立即发送 | 6 次成功请求，编号 02–07，各一次，均为新文本，无旧草稿或重复请求 |
| Enter 发送偏好 | Shift+Enter 换行后 Enter 发送，HTTP 正文精确包含 `第一行\n第二行` |
| Cmd+Enter 发送偏好 | 在实际设置中保存；Enter 只换行，Cmd+Enter 发送，HTTP 正文保留换行 |
| 切换会话 | 返回原会话后 `切换会话保护草稿` 保留 |
| Cmd+W 关闭最后窗口后重新打开 | 同一进程恢复会话，`窗口重开仍需保留的验收草稿` 保留；不是进程重启后的磁盘草稿持久化 |
| 中文 IME 候选确认 | **人工确认通过**：用户在 Enter 发送偏好下按 Enter、在 Cmd+Enter 发送偏好下按 Cmd+Enter，均反馈文字留在框中且未发送；工具随后观察到 `shi`，请求/消息数未增加，显式按钮发送只新增一次。候选栏本身未被独立捕获 |
| 普通工具审批等待期间草稿保护 | **通过**：真实 HTTP 工具调用触发 shell 审批；等待时编辑草稿、切换会话再返回、反馈框立即 Escape、拒绝审批后草稿均保留；数据库确认未发送 |

HTTP 共 8 个成功请求，隔离会话数据库中为 8 user + 8 assistant（另有 1 system）。初次编号 01 因 QA profile 没有合成 key 而被配置校验拒绝，没有 HTTP 请求，未计入成功样本。Markdown 阅读视图将软换行显示为空格，但实际发送正文换行完整，不将阅读样式误判为输入丢失。

解锁后的补验采用用户手动输入和确认、工具只读核对的协作方式，避免工具接管打断候选。两种偏好下用户均反馈未发送；确认后观察到 `shi` 仍在框中，HTTP 请求保持 11、user 消息保持 10。最后显式点击发送，准确新增 1 条 `shi`，HTTP 12、user 11，无重复。见 [IME 结果](ime-result.json)、[Enter 人工操作后截图](native-ime-manual-enter.jpg)、[组合键人工操作后截图](native-ime-manual-modifier.jpg) 与 [最终请求记录](http-inputs-final.jsonl)。普通 HTTP 审批场景已补验完成。QA 发送偏好已恢复初始 Enter，语言恢复中文，原生主题恢复跟随系统。

补充普通审批实机证据：[等待截图](native-approval-draft-waiting.jpg)、[拒绝后截图](native-approval-draft-denied.jpg)、[独立数据库核对](approval-draft-result.json)。隔离项目将 shell 策略收紧为 ask，本地服务返回 `echo wisp-phase2-approval` 工具请求，最终选择拒绝，命令未执行。草稿为“审批等待时保留的草稿，不自动发送”。在审批结束的核对时点，输入会话新增 1 条 user，共 9 条；模型的工具结果续接另有一次 HTTP 请求，不能把它算成重复用户发送。后续请求记录见 [http-inputs-with-approval.jsonl](http-inputs-with-approval.jsonl)。

## 4. 稳定回归与性能基线


| 检查 | 本次结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| Swift 定向 | 29 passed（同步、导入、输入），包含 opt-in render |
| Swift 全套 | 289 passed，0 failures（UI 274 + 基础 15），包含全部 opt-in render |
| 标准 Native QA 构建与严格签名 | 通过 |
| WebView UI/宿主重建与严格签名 | 通过；首次 Trunk 因继承 `NO_COLOR=1` 解析失败，改为 `NO_COLOR=true` 后构建通过 |
| Rust workspace 全套 | 2361 passed、0 failed，41 条标准结果汇总，包含最后的 doc-tests；日志 `/tmp/wisp-phase2-full-rust.log` |
| wasm check | 退出 0；日志 `/tmp/wisp-phase2-wasm.log` |
| `npm ci` | 通过，lockfile 未改 |
| Playwright 全套 | 870 passed、2 skipped，退出 0，19.0 分钟；日志 `/tmp/wisp-phase2-playwright.log`；生成的 5 张无关 research-journey 截图已恢复 |

完成记录和日志指纹见 [completed-checks.json](completed-checks.json)。原 Rust exec session 在后续轮次已不可查询退出码；以上依据完整日志的全部结果、最终 doc-test 和进程结束核对，不伪造 session 退出码。wasm/Playwright 的退出码由顺序执行器保存。

既有 workspace 全套会自然执行 ACP 测试；这不扩展本阶段 ACP 专项实现/实机验收范围。没有沿用前一阶段的 2361 Rust 或 870 Playwright 结果宣称本轮全绿。

### 已记录的 35-turn 切换数据

两端同一合成历史，浅色中文、侧栏展开、右面板关闭，窗口拖动目标为 1100×800；CUA 返回的两张截图均为 1056×768，未经编辑。机器仍有其他用户应用，采样时没有运行重型构建/测试。Native 最新页按 20 user turns 分页，因此**同一个 35-turn 历史不意味着两端同时挂载相同数量的消息节点**。

从 1-turn 会话切换到 35-turn 会话，用 `Date.now()` 测量 CUA 点击开始到 AX 中目标 S35 可见的时间。每端 3 次有效 warm-switch 样本：

| 客户端 | 样本 ms | 中位数 ms | 范围 ms |
| --- | --- | --- | --- |
| Native | 1130, 1050, 1106 | 1106 | 1050–1130 |
| WebView | 1367, 1347, 1398 | 1367 | 1347–1398 |

原始记录见 [cua-latencies.json](cua-latencies.json) 与 `*-switch-*.txt`。首次试采样中 Native 一条记录未确认 S35 可见，保留为 `visible=false`，不混入上述统计。该端到端指标包含 Computer Use 调用和自动等待开销，不能当作 paint、帧率或单独的渲染性能，也不据此宣称 Native 更快。首页打开、滚动和限定进程范围的 idle CPU 已在完整回归结束后补采，见下表。

### 补充打开、滚动和 idle CPU

[cua-more-latencies.json](cua-more-latencies.json) 保留每次原始 AX 文本。首页打开是已运行进程中的最近会话入口 → 首次 AX 观察到目标历史，**不是冷启动**。原生三次首次观察到 S33/S34，S35 尚不在该 AX 观察中；保留 `visible=false` 与 `history_visible=true`，不把它们混成“S35 已可见”。滚动为从末尾向上 1 页 → AX 返回，包含自动化开销，不能代表滚动帧率。

| 客户端/指标 | 样本 ms | 中位数 ms |
| --- | --- | --- |
| Native 首页打开历史 | 1737, 1790, 1722 | 1737 |
| WebView 首页打开历史 | 1283, 1552, 1695 | 1552 |
| Native 向上滚动一页 | 965, 987, 1048 | 987 |
| WebView 向上滚动一页 | 832, 1313, 1301 | 1301 |

[idle-cpu.json](idle-cpu.json) 使用 `ps time` 累计 CPU 差值，连续 3 个约 10 秒窗口；采样时两端均在 35-turn 历史、无构建/测试/CUA 操作。

| 进程范围 | 单核百分比，3 次 |
| --- | --- |
| Native shell PID 21759 | 1.697%, 1.896%, 1.997% |
| Native helper PID 39125 | 1.198%, 1.098%, 1.198% |
| WebView 主进程 PID 28337 | 0.998%, 0.998%, 1.098% |

WebKit 的 GPU/Networking/WebContent 是独立 XPC 进程，本轮无法可靠归属全部辅助进程；因此上述 CPU **不是两种完整产品的总 CPU 对比**。`launchctl procinfo` 需要 root，本轮未提升系统权限。冷启动、帧率、全进程 CPU 和更多重复样本留作专项性能测量，当前数据用于同条件复验，不能据此宣布性能胜负。

### 配对截图：8 个条件、两端共 16 张

图片未经编辑；常规窗口图像为 1056×768，窄窗为 800×768。每张均目视核对 S35 的表格、代码、引用和输入区域。

| 偏好/窗口 | Native | WebView |
| --- | --- | --- |
| 中文、浅色、常规 | [截图](native-zh-light-wide.jpg) | [截图](webview-zh-light-wide.jpg) |
| 中文、浅色、窄窗 | [截图](native-zh-light-narrow.jpg) | [截图](webview-zh-light-narrow.jpg) |
| 中文、深色、常规 | [截图](native-zh-dark-wide.jpg) | [截图](webview-zh-dark-wide.jpg) |
| 中文、深色、窄窗 | [截图](native-zh-dark-narrow.jpg) | [截图](webview-zh-dark-narrow.jpg) |
| 英文、浅色、常规 | [截图](native-en-light-wide.jpg) | [截图](webview-en-light-wide.jpg) |
| 英文、浅色、窄窗 | [截图](native-en-light-narrow.jpg) | [截图](webview-en-light-narrow.jpg) |
| 英文、深色、常规 | [截图](native-en-dark-wide.jpg) | [截图](webview-en-dark-wide.jpg) |
| 英文、深色、窄窗 | [截图](native-en-dark-narrow.jpg) | [截图](webview-en-dark-narrow.jpg) |

`en` 表示保存了英文偏好，**不表示原生工作区英文完整通过**：设置页已有英文标签，但工作区仍保留“新建会话、设置、发送”等中文，见 [设置 AX 记录](native-en-settings.txt)。这是一项实际差异，应单独修复语言覆盖与切换后的刷新。

原生窄窗仍展开完整侧栏，WebView 自动收为图标栏；WebView 英文窄窗的部分侧栏图标还有裁切现象。当前配对还可见表格内边距、代码语法高亮、引用间距、消息操作区和顶部工具栏的差异。输入 smoke 重复观察到原生正文显示 Usage JSON，见审批截图；根因尚未定位。上述问题不通过临时隐藏内容或修改截图掩盖，列为后续独立修复。

配对保证目标会话内容一致：原生侧栏另有输入 smoke 会话，两端 QA 模型标签不同。没有为了截图删除会话或草稿。原生分页、两端渲染节点数量和语言覆盖差异也应在后续性能对比中保留说明。

## 证据边界与后续

IME 的一次早期交接未成功：用户反馈候选栏出现，但随后 CUA 观察到的仍是原审批草稿，没有拼音/候选；工具 Return 发送了该测试草稿，新增 1 个请求。这次不计入成功 IME 样本，也不能在缺少同帧候选状态的情况下直接归因于产品缺陷。之后由用户完整手动操作并分别确认两种偏好均未发送；CUA 观察到保留的 `shi`，数据库/HTTP 记录支持没有额外发送，最后按钮发送仅一次。候选存在这一前提依赖用户实机反馈，未录得候选栏视频，不将其包装成全自动 IME 验证。

本轮完成的是约定范围内的可靠性验收和基础质量记录，不是全部原生 parity 或正式发行验收。性能记录的暖进程、自动化开销、分页及进程归属限制仍适用。

后续独立修复优先处理原生工作区语言覆盖与 Usage JSON 正文泄漏，再评估窄窗和视觉对齐；不扩展 ACP。当前没有产品代码改动，尚未发布或推送本阶段 PR。
