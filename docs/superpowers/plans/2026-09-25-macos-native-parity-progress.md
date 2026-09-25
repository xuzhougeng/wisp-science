# macOS parity 修复进度

目标：完成 [Computer Use 审计](2026-09-25-macos-native-parity-iteration.md)发现的问题。审计文档与截图保留为改动前的证据；本文记录实现和验证，不把入口存在视为闭环完成。分支：`codex/macos-native-parity-fixes`。

## PR 汇总状态（2026-09-25）

本轮改动已通过 [PR #1361](https://github.com/xuzhougeng/wisp-science/pull/1361) 合并到 main，合并提交为 `70a95141`。这次合并不是全部 parity 目标的结项，也不是版本发布。下一阶段执行 [可靠性与实机验收收尾计划](2026-09-25-macos-native-parity-phase2.md)。本文后半部分按时间保留历史验证记录，早期的“运行中”“尚待构建”以本节及后续结果为准。

- 本轮最后验证的产品代码为 `092aae1a`，后续实现以合并后的 main 为起点。ACP 首轮前 agent 选择持久化已实现；标准 QA 包构建与严格签名验证通过，shell/helper 均为 1.14.0，源码标识 clean。新包的宿主重启 smoke/CUA 尚未执行。
- Swift 全套 281 passed（含全部 opt-in render）；WebView 全套 870 passed、2 skipped；ACP 存储全套 211 passed，补充断言后定向 8 passed，项目查询 9 passed，ACP 宿主定向 19 passed。最新源码的 wasm、格式及资源/契约同步检查通过。
- 合并前源码 `092aae1a` 的 Rust workspace 全套已完成，退出码 0：2361 passed、0 failed，包含存储 211 项、桌面宿主 1009 项，另含 ACP process harness 与 process-tree 集成目标。日志 `/tmp/wisp-parity-pr-workspace.log`。本次覆盖删除与 ACP 持久化，但不能算作合并后 main 的完整验证。
- 收尾优先级：未命名已保存 ACP 空会话的标题 fallback（保留真实标题与首条消息命名语义）、首轮前选择重启验证、同步冲突确认/取消/解决、目录选择器立即 Escape、真实 IME/快速输入。真实 OAuth、性能和正式分发未验收。
- 后续按会话/审批、论文证据、输入/阅读、正式交付与质量基线分批推进；以下未完成清单全部保留。

## 当前批次

| 项目 | 实现 | 验证 |
| --- | --- | --- |
| 共享资源漂移 | 同步新增的 19 个订阅登录文案 | 两项资源/契约检查通过 |
| 构建版本 | shell/helper 统一产品版本，记录完整 git revision 与 dirty 标记；增加脚本测试及 CI | Python 测试通过；标准 QA 构建与严格签名验证通过，两包均为 1.14.0 |
| 最后窗口恢复 | 单 workspace scene；Finder/Dock reopen 请求复用 scene/model | delegate 测试及 CUA Cmd+W/红叉关闭→应用 reopen 通过，同一会话和草稿保留；QA host PID 未变 |
| 发送偏好 | 共享 AppKit 输入框；读取保存的偏好；Enter、Cmd/Ctrl+Enter、Shift 与 IME 分开处理 | 原生键盘事件、IME 和打开时序测试通过；CUA 已验偏好保存/返回后生效、Enter/Shift+Enter 换行；实机 IME/发送仍待验收 |
| block Markdown v1 | 标题/列表/引用/代码/原生表格；代码及表格右键复制；跨块选择/收藏 | 布局/复制/跨块选择测试通过；浅色及深色窄窗截图目视检查通过 |
| 问题卡片 | 保留已有草稿与描述、可编辑选项/自由回答、状态判定与导航隔离 | 5 项 fake snapshot 测试通过；ACP 实时回复仍属于下批范围 |
| 审批反馈 | 复用反馈字段；当前 session/approval ID 检查；窗口级 Escape | 反馈失败不重试、过期请求拒绝、实际反馈视图立即 Escape 测试通过 |
| 项目目录导入 | 新旧目录格式共用 WebView 校验与原地登记；ZIP 保留；取消不请求 | Swift 导入流程及 Rust 新旧格式/重复/无效路径测试通过；CUA WebView 创建的当前格式项目原地登记通过；旧格式及立即 Escape 仍待复验 |
| 历史恢复 | 只读预览、会话/消息/无效/重复数量、命名与确认；保持源归档 | 恢复/取消/未知结果不重试/嵌套 Escape，以及后端只读预览/源归档字节不变测试通过；CUA 预览/立即 Escape/确认恢复/消息可读及源 SHA-256 不变通过 |
| ZIP/目录导出 | 显式项目与目标、复用独占锁/运行检查/校验与发布，显示确认后的目标 | Swift 5 项、Rust transfer 12 项通过；包含两种格式往返与目标保护；CUA ZIP/目录导出完成、ZIP CRC 通过，立即 Escape 仅关闭 Save Panel |
| ACP 实时交互 | 新增会话范围的权限选项/取消与问题回复，复用现有 resolver；保留草稿、阻止重复提交 | 8 项 Swift（含明暗窄窗渲染）、3 项 Rust scoped resolver 测试通过；会话创建/发送/停止、权限回复、进程退出与 host 重启恢复已通过隔离真实宿主协议验收；CUA 普通回复、权限、运行/初始化 Stop 和草稿保留通过 |
| ChatGPT 订阅 | 复用现有 keyring 认证、浏览器/设备码登录、状态轮询、手动回调、取消/过期/重登与已存账户 | 12 项隔离 transport 测试通过，涵盖旧回调、取消、保存结果不确定和嵌套 Escape；CUA 入口及菜单/sheet 两层 Escape 通过；未登录真实账户 |
| 项目同步 | 卡片快照状态、启用文件夹快照、手动同步；显式本地/远端冲突决策 | Swift 状态 2 项及操作 7 项通过；Rust 项目测试 13 项通过，含同步范围与策略校验；CUA 启用/同步及首页状态通过，冲突交互仍待实机验收 |

## 后续范围（全部保留，不能因第一批完成而结项）

- [x] 内置问题卡片：不覆盖草稿、pending/answered/expired、自由输入暂存、导航后旧回调隔离。ACP request ID 与实时回复已接入，完整 ACP 会话生命周期留在 ACP 项。
- [ ] 计划与审批：反馈已完成；plan mode 决策与普通工具授权范围仍待实现；维持现有 Agent workflow 审批。
- [ ] 项目：目录导入、ZIP/目录导出、历史恢复、快照状态与同步操作均已实现；收尾后端检查、完整回归及 CUA 导入/导出/同步闭环验收。
- [ ] ChatGPT 订阅：已有 keyring/认证路径的登录、状态、取消、过期与重新登录已实现；标准 QA 构建和 CUA 入口/两层 Escape 已通过；真实 OAuth 过程未发起，生命周期由隔离 transport 测试覆盖。
- [ ] ACP：权限选项/取消与问题回复已实现；会话创建/恢复、发送/停止已接入；fake ACP process CUA 已通过。首次发送前选择的持久化已实现并通过自动化及 QA 构建，宿主重启 smoke/CUA 和空标题显示仍待收尾。
- [ ] 会话：重命名、置顶、删除/批量删除入口与会话范围操作已实现，删除后端及单个/批量 CUA 已通过；跨项目复制/移动、导出、分支导航仍待完成。
- [ ] 论文：论文/版本导航、条目/证据管理、检查、冻结。
- [ ] 输入与上下文：@/#/slash、上下文快照与手动压缩。
- [ ] 阅读：数学/图片、Markdown/CSV 预览、科学 viewer 支持表与对应实现。
- [ ] 正式交付：首次使用、原生壳与 helper 整体升级/失败恢复、签名/公证/安装流程；实际发布另行授权。
- [ ] QA：布局与可操作性断言、应用生命周期 smoke、同 fixture 明暗/中英/窄窗截图、35-turn 性能基线。

## 验证记录

- Python 原生脚本测试：2 passed；design/settings contract 同步检查通过。
- Swift 全套：新增项目同步后的 232 tests，0 failures，启用 `WISP_NATIVE_SNAPSHOT_DIR`，包含所有 opt-in render。日志：`/tmp/wisp-parity-sync-full-swift.log`。此前 226 项全套曾有一次既有 Agent 面板 renderer `InvalidTransition`，该项定向复测及后续两次全套通过；保留失败日志 `/tmp/wisp-parity-projects-full-swift.log`，不抹掉偶发失败记录。
- 项目后端：`native_projects::tests` 12 passed（目录导入和历史恢复）；`project_transfer::tests` 12 passed（共享导出与运行状态检查）。日志 `/tmp/wisp-parity-recovery-rust.log`、`/tmp/wisp-parity-export-rust.log`。首次目录导入测试用旧 store 导致失败，改为应用级 store 后通过；新增同步范围测试在 `/tmp/wisp-parity-sync-rust.log`。
- 项目 DTO 4 tests 通过；增加同步 allowlist 后的 native DTO 29 项通过，日志 `/tmp/wisp-parity-sync-dto.log`。
- `cargo fmt --all -- --check` 通过；同步 allowlist 更新后的 `ui` wasm check 通过，日志 `/tmp/wisp-parity-sync-wasm.log`。
- WebView Playwright 全套：867 passed、2 skipped、3 failed。两项启动超时单独复测通过；原有首页文档按钮测试在 hydration 前读取空按钮列表，改为等待期望顺序。修复后的相关 4 项复测全部通过；没有把这次定向复测冒充整套绿色。日志：`/tmp/wisp-parity-playwright.log`、`/tmp/wisp-parity-playwright-verified.log`。
- 最新 Swift 全套：245 tests，0 failures（UI 230 + 基础 15），启用全部 opt-in render，包含 12 项订阅认证测试。日志 `/tmp/wisp-parity-auth-final-swift.log`。之前两个 process transport 超时已在本次全套通过，未放宽超时。Agent 面板首次渲染失败定位为测试未创建输出目录，已修复；全新目录中复测生成全部 7 张图片，日志 `/tmp/wisp-parity-agent-fresh-check.log`。
- WebView 新一轮全套使用 2 workers，在多项子进程启动延迟后出现连续超时，已发送 SIGINT 停止这次并发复测：312 passed、2 failed、2 interrupted、556 did not run；日志 `/tmp/wisp-parity-projects-playwright.log`。依赖安装已完成且 lockfile 未变；已恢复本次生成的 research-journey 图片。Rust 重负载任务结束后的新一轮 2 workers 全套已完成：870 passed、2 skipped（20.3 分钟），日志 `/tmp/wisp-parity-auth-playwright.log`；生成的 5 张研究历程截图已恢复。
- Rust workspace 全套已结束：Tauri 1000 passed、1 failed，失败为已有 terminal manager 测试等待 shell 输出的 5 秒超时；日志 `/tmp/wisp-parity-rust-full.log`。该项不改超时定向复测 1 passed（0.02 秒），日志 `/tmp/wisp-parity-terminal-rust-retry.log`。全套未全绿，后续 workspace 测试未全部执行，不能用定向复测替代全套结果。
- 含目录/历史/导出/同步的标准 QA 构建与严格签名验证已通过，日志 `/tmp/wisp-parity-projects-build.log`；后续订阅/ACP/首页错误修复的稳定重建也已通过：`/tmp/wisp-parity-auth-acp-build.log`，revision `e3a35fcd`，dirty=false。应用：`target/native-macos-qa/Wisp Science QA.app`；真实复验需在解锁后重新启动更新的 shell/helper。
- 订阅认证共享 DTO 全套 60 passed，日志 `/tmp/wisp-parity-auth-dto.log`；wasm check 通过，日志 `/tmp/wisp-parity-auth-wasm.log`。
- CUA 已于 18:52 恢复可用；项目批 QA 包版本 1.14.0 / revision `65feaa83` / dirty。关闭重开、草稿、换行偏好、导出、重复导入保护和同步已验；19:08 重建后订阅入口/嵌套 Escape 及首页错误修复 CUA 复验通过；ACP 完整进程仍待接入。详见 `docs/design-qa/native-parity-2026-09-25/fix-validation.md`。
- 测试仅使用隔离合成数据库：`/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`。QA app 的 database preference 指向它；QA host 的 Application Support symlink 指向同一目录。之前的临时 QA 链接路径记录在 `/tmp/wisp-parity-fixes-previous-qa-link.txt`。
- 未登录真实外部账户，未执行远程科学任务，未推送分支或创建远端 PR。

## 下一步

1. 继续用 Computer Use 验收问题卡片和反馈；补齐真实 IME 与发送 smoke，完成目录新登记和同步冲突交互。
2. 收尾 Rust 完整回归结果；保留全套 WebView 的原始失败与复测记录。
3. 收尾本批项目能力的构建/测试/实机闭环；订阅认证已接入，继续 ACP、会话管理、论文证据。全部剩余项仍属本目标范围，当前未结项。

- ACP 交互批最新全套 Swift 253 passed，包含全部 opt-in render（`/tmp/wisp-parity-acp-full-swift.log`）；scoped Rust resolver 3 passed（`/tmp/wisp-parity-acp-native-rust.log`）；DTO 全套 61 passed、wasm check 通过（`/tmp/wisp-parity-acp-dto.log`、`/tmp/wisp-parity-acp-wasm.log`）。

- CUA 发现并修复首页旧导入错误/过期数据提示；新增“保留打开表单错误、失败刷新不清除、成功刷新清除、不重试导入”测试。最新全套 Swift 254 passed，日志 `/tmp/wisp-parity-cua-fix-swift.log`。

- Rust workspace 全套重跑已通过，退出码 0：标准测试结果汇总 2347 passed、0 failed；另含 ACP process harness 成功与 process-tree 集成目标。日志 `/tmp/wisp-parity-final-workspace.log`，原执行 session 88359。Tauri 1006 项包含 ACP 创建/路由回归；这次构建不含随后添加的初始化取消/actor 清理/死进程恢复锁修复，因此后者仍须单独验证并纳入下一次全套。

- ACP 会话批：Swift 全套 258 passed（UI 243 + 基础 15，含全部 opt-in render），日志 `/tmp/wisp-parity-acp-turn-full-swift.log`；Rust native conversation 定向 4 passed（`/tmp/wisp-parity-acp-turn-rust.log`）；离线 QA peer 子进程测试 1 passed；共享 DTO 全套 62 passed（`/tmp/wisp-parity-acp-turn-dto.log`）。`cargo fmt --all -- --check` 和最新 wasm check 通过（`/tmp/wisp-parity-acp-turn-wasm.log`）。实际 QA 包正在构建，不能把 fake transport 测试当作完整界面验收。

- ACP CUA 构建 `5719574a` / clean 通过严格签名检查；实际创建菜单与新会话可用，但首轮启动/Stop 未通过，详情见验收记录。已修复空原生草稿误走历史查询而报 Session not found 的问题，导航 8 项及 Swift 全套 258 项通过（`/tmp/wisp-parity-native-draft-navigation-swift.log`、`/tmp/wisp-parity-native-draft-full-swift.log`）。ACP 初始化取消与 actor 清理修复正在编译验证；未提交此后端修复、未宣称 ACP 全流程通过。

- ACP 初始化取消修复：native startup 路由测试 1 passed（`/tmp/wisp-parity-acp-startup-routing-rust.log`）；ACP lib 全套 3 passed（`/tmp/wisp-parity-acp-startup-lib-verified.log`）；真实 fake-agent process harness 通过，含初始化不回复时取消后心跳停止（`/tmp/wisp-parity-acp-startup-process.log`）。最初以远端 SDK handler 的退出作为清理信号的测试超时，保留 `/tmp/wisp-parity-acp-startup-cancel.log`；改为直接观察传输 drop，并另用真实进程心跳证明子进程清理。还修复 dead-runtime 恢复时 if-let 暂存 guard 导致的重复加锁，恢复 CUA 仍待验。
- QA peer 新增 `--stall-initialize`，可稳定复现初始化等待，两个 Python 子进程测试通过。Mac 再次锁屏，已请求手动解锁；不把无法执行的 CUA 写成通过。

- 会话重命名：顶部编辑入口、显式项目/会话参数、成功后刷新标题、失败保留输入、不自动重试、导航隔离、窗口级立即 Escape。新增 5 项 Swift 回归通过；全套 Swift 263 passed（UI 248 + 基础 15，含所有 opt-in render），日志 `/tmp/wisp-parity-session-rename-full-swift.log`；DTO 63 passed（`/tmp/wisp-parity-session-rename-dto.log`）；wasm check 通过（`/tmp/wisp-parity-session-rename-wasm.log`）。宿主 QA 重建中，尚未 CUA 验收。

- `fca972e3` QA 重建与签名验证通过；隔离真实宿主协议验收全部通过：发送、权限选择、dead process 恢复、运行中 Stop、初始化 Stop（约 0.283 秒且子进程退出）、host 重启后的持久绑定恢复。重命名拒绝跨项目/空名称，名称经只读查询确认持久保存。日志 `/tmp/wisp-parity-native-acp-host-smoke.log`、`/tmp/wisp-parity-native-acp-host-resume.log`。这些是 API/进程验收，不能代替仍待解锁的 CUA。

- 会话置顶：复用持久状态与共享 pin 图标，置顶分区不重复列出分组中的会话；未知状态禁用、请求失败不重试、导航后旧回调隔离。侧栏元数据刷新保留会话选择、草稿和历史页游标。定向 Swift 26 passed（`/tmp/wisp-parity-session-pin-navigation-swift.log`）；全套 Swift 271 passed（UI 256 + 基础 15，含全部 opt-in render，`/tmp/wisp-parity-session-pin-full-swift.log`）；wasm check、设计/契约同步检查、格式检查通过。Rust 项目查询首轮 7 passed、DTO 全套 64 passed（`/tmp/wisp-parity-session-pin-rust.log`）；旧数据库 pin 字段缺失时保持历史可读，新增兼容测试另轮运行中（`/tmp/wisp-parity-session-pin-compat-rust.log`）。稳定代码的 workspace 全套运行中（`/tmp/wisp-parity-session-pin-workspace.log`）；新版 QA 包/CUA 待完成。
- ACP/重命名之后的 workspace 全套未完成：编译期间修改了 DTO，旧 `RecentSession` 编译产物与新增 `pinned` 调用不一致，编译退出 101，未进入完整测试；保留 `/tmp/wisp-parity-acp-rename-workspace.log`。稳定代码后重新执行全套，不把此前全绿结果当作本批验证。
- 置顶修复已提交 `a4455d0b`；新增旧库兼容测试后的项目查询 8 passed、0 failed（`/tmp/wisp-parity-session-pin-compat-rust.log`），确认缺少 pin 字段不触发迁移、不阻断历史读取，主查询错误仍正常暴露。Swift 271、DTO 64 及 wasm 已通过。workspace 全套与 QA 重建仍在运行，未宣称完成；Mac 仍待解锁后的 CUA 验收。空闲的隔离 QA host 43632 已停止，更新后需重启；未停止其他应用或宿主。
- 置顶真实宿主协议验收通过：true/false/true 状态及保存列表一致、跨项目和非布尔请求被拒绝、host 重启后仍置顶（`/tmp/wisp-parity-native-pin-host-smoke.log`、`/tmp/wisp-parity-native-pin-host-restart.log`）。QA 构建及签名检查通过（`/tmp/wisp-parity-session-pin-build.log`），revision `60eec4d8` / dirty=true；构建期间开始了删除批次，当前宿主未包含随后新增的 existence 命令，不把此包用于完整删除验收。
- 删除/批量删除：列出目标后确认、复用宿主停止/分支/归档检查、串行执行遇到首个错误停止、不自动重试、迟到响应不关闭新弹窗、已确认删除移除本地草稿。未命名草稿的未知删除结果由同项目只读 existence 查询核对，查询失败保留草稿，仍存在时在后续刷新继续检查。定向 Swift 19 passed；全套 Swift 279 passed（UI 264 + 基础 15，含所有 opt-in render，`/tmp/wisp-parity-session-delete-full-swift.log`）；wasm、格式及资源同步检查通过。Rust native conversation 定向运行中（`/tmp/wisp-parity-session-delete-rust.log`）；之前启动的 workspace 全套只覆盖置顶及此前改动，不包括删除批次。
- 删除批 Rust native conversation 5 passed、0 failed（同上日志），含删除请求参数边界以及存在性查询的所有权/不存在检查。稳定 QA 重建、真实宿主删除/停止/未知结果恢复与 CUA 仍待验收；DTO 全套复测进行中（`/tmp/wisp-parity-session-delete-dto.log`）。
- 删除批提交 `029b0a7c`；DTO 全套复测 64 passed（同上日志），工作树干净。稳定源码的 QA 重建进行中（`/tmp/wisp-parity-session-delete-build.log`）。为替换应用，已确认隔离宿主 48979 空闲并停止；构建结束后需重新启动 helper。真实宿主删除验收脚本已准备，仅操作现场新建的合成会话，不删除既有 ACP/历史恢复验收记录；尚未运行，不能计为通过。

- `70857150` 稳定 QA 构建及严格签名通过，真实宿主删除 smoke 通过：范围拒绝、精确删除、丢失响应后只读确认、运行中 ACP 清理（`/tmp/wisp-parity-native-delete-host-smoke.log`）。
- 20:58 后 CUA 已恢复：重命名及立即 Escape、取消/恢复置顶、草稿保持、ACP 创建/回复/权限/运行 Stop/初始化 Stop、单个及批量删除通过。保存 4 张实际截图，详见 fix-validation。多选辅助功能标记发现问题并修复，Swift 定向 6 项、全套 280 项通过（`/tmp/wisp-parity-session-selection-a11y.log`、`/tmp/wisp-parity-selection-a11y-full-swift.log`）；新标记尚待重建 CUA。重负载期间延迟及快速输入现象保留，性能与 IME 不算通过。
- 置顶及此前改动的 workspace 全套退出 0：2354 passed、0 failed，含 Tauri 1008 项与 ACP process harness（`/tmp/wisp-parity-session-pin-workspace.log`）。该轮开始于删除批之前，删除后改动仍需后续稳定全套验证。
- 21:21–21:25，WebView 新建隔离项目 → 原生目录登记 CUA 通过：两端 project ID/路径一致，project.json 哈希不变，首页显示第五个项目；目录选择器首次 Escape 无效、第二次仅关闭最上层，立即 Escape 验收保留为待复验。截图 native-fixed-folder-import.jpg。同步冲突仍待 CUA。

- `f4b279e8` clean QA 重建及严格签名通过；多选辅助功能 CUA 已验证：两行勾选各自 selected、未勾选活动行不 selected、退出恢复仅活动行 selected。
- 同步冲突 CUA 已显示拦截及两种选择，使用同内容的合成 revision 分叉，仅验证界面路由，未运行真实双设备/网盘。随后 Mac 锁屏，确认/取消/解决仍待继续；fixture 保留。修复现场发现的错误文字省略，新增明暗渲染测试，Swift 全套 281 passed（`/tmp/wisp-parity-sync-error-wrap-full.log`），4 张图目视检查通过。最新运行包尚不含换行修复。
- 为会话管理后的 UI/Tauri 改动启动新一轮 npm ci + WebView 全套（2 workers，`/tmp/wisp-parity-session-management-playwright.log`），仍在运行；不可用先前 870 项绿色代替此次结果。删除之后的稳定 Rust 全套待此轮结束后运行，以减少测试负载互扰。

- ACP 首轮前选择持久化正在实现：新增可空 frames.acp_agent_selection 和幂等迁移；空的已选择/已连接会话纳入项目列表，普通未命名草稿仍隐藏，旧库只读查询不迁移。连接成功时在同一事务保存 binding 并清除待连接选择，拒绝不同 profile 的覆盖；native Record 不再保存唯一的选择副本。共享发送及模型读取复用持久选择，模型切换不能悄悄回退到 HTTP。项目快照导入导出和空会话复制保留选择，复制仍不携带外部 ACP session binding。
- 首轮存储定向 8 项通过（`/tmp/wisp-parity-acp-choice-store-final.log`），覆盖宿主数据重开、旧库兼容、作用域拒绝、未发送消息/未伪造连接、连接前后可见性、同库及 workspace-owned 跨项目复制、项目导出导入。之后新增 profile 不匹配时事务回滚断言，须复测。存储全套、项目查询和 ACP 宿主测试依次运行中（`/tmp/wisp-parity-acp-choice-{store-full,app,tauri}.log`）；wasm、设计资源和设置契约检查已通过。当前源码未完成稳定宿主重建和重启前后真实协议 smoke，不能宣称此项闭环完成。

- 本轮 WebView 全套完成：870 passed、2 skipped、退出 0（20.3 分钟，`/tmp/wisp-parity-session-management-playwright.log`）；已恢复测试生成的 5 张研究历程图片。ACP 选择持久化的 Store 全套 211 passed；之后补充的 profile 冲突回滚及跨库 move 断言定向 8 passed（`/tmp/wisp-parity-acp-choice-guard.log`）；项目查询 9 passed，ACP 宿主定向 19 passed（包含共享发送选择恢复测试），wasm/格式/资源同步检查通过。真实宿主重启 smoke 尚待新包，不将上述测试标记为 CUA 完成。
