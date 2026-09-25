# macOS parity 修复进度

目标：完成 [Computer Use 审计](2026-09-25-macos-native-parity-iteration.md)发现的问题。审计文档与截图保留为改动前的证据；本文记录实现和验证，不把入口存在视为闭环完成。分支：`codex/macos-native-parity-fixes`。

## 当前批次

| 项目 | 实现 | 验证 |
| --- | --- | --- |
| 共享资源漂移 | 同步新增的 19 个订阅登录文案 | 两项资源/契约检查通过 |
| 构建版本 | shell/helper 统一产品版本，记录完整 git revision 与 dirty 标记；增加脚本测试及 CI | Python 测试通过；标准 QA 构建与严格签名验证通过，两包均为 1.14.0 |
| 最后窗口恢复 | 单 workspace scene；Finder/Dock reopen 请求复用 scene/model | delegate 测试通过；Mac 锁屏，真实关闭/重开仍待验收 |
| 发送偏好 | 共享 AppKit 输入框；读取保存的偏好；Enter、Cmd/Ctrl+Enter、Shift 与 IME 分开处理 | 原生键盘事件、IME 和打开时序测试通过；Mac 锁屏，实机仍待验收 |
| block Markdown v1 | 标题/列表/引用/代码/原生表格；代码及表格右键复制；跨块选择/收藏 | 布局/复制/跨块选择测试通过；浅色及深色窄窗截图目视检查通过 |
| 问题卡片 | 保留已有草稿与描述、可编辑选项/自由回答、状态判定与导航隔离 | 5 项 fake snapshot 测试通过；ACP 实时回复仍属于下批范围 |
| 审批反馈 | 复用反馈字段；当前 session/approval ID 检查；窗口级 Escape | 反馈失败不重试、过期请求拒绝、实际反馈视图立即 Escape 测试通过 |
| 项目目录导入 | 新旧目录格式共用 WebView 校验与原地登记；ZIP 保留；取消不请求 | Swift 导入流程及 Rust 新旧格式/重复/无效路径测试通过；CUA 文件选择器待验收 |
| 历史恢复 | 只读预览、会话/消息/无效/重复数量、命名与确认；保持源归档 | 恢复/取消/未知结果不重试/嵌套 Escape，以及后端只读预览/源归档字节不变测试通过 |
| ZIP/目录导出 | 显式项目与目标、复用独占锁/运行检查/校验与发布，显示确认后的目标 | Swift 5 项、Rust transfer 12 项通过；包含两种格式往返与目标保护；系统 Save Panel 待 CUA 验收 |
| 项目同步 | 卡片快照状态、启用文件夹快照、手动同步；显式本地/远端冲突决策 | Swift 状态 2 项及操作 7 项通过；Rust 项目测试 13 项通过，含同步范围与策略校验；真实交互待验收 |

## 后续范围（全部保留，不能因第一批完成而结项）

- [x] 内置问题卡片：不覆盖草稿、pending/answered/expired、自由输入暂存、导航后旧回调隔离。ACP request ID 已解析，实时响应留在 ACP 项。
- [ ] 计划与审批：反馈已完成；plan mode 决策与普通工具授权范围仍待实现；维持现有 Agent workflow 审批。
- [ ] 项目：目录导入、ZIP/目录导出、历史恢复、快照状态与同步操作均已实现；收尾后端检查、完整回归及 CUA 导入/导出/同步闭环验收。
- [ ] ChatGPT 订阅：已有 keyring/认证路径的登录、状态、取消、过期与重新登录。
- [ ] ACP：会话创建/恢复、发送/停止、审批与问题回复；fake ACP 验证。
- [ ] 会话：重命名、删除/批量整理、置顶、跨项目复制/移动、导出、分支导航。
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
- 最新 Swift 全套新增“启用时发现冲突”测试：UI 218 项通过，但两个既有 process transport 测试在启动临时脚本时超时；日志 `/tmp/wisp-parity-project-final-swift.log`。同时观察到 Git/Rust 子进程启动延迟和 `syspolicyd` 高负载，尚不据此确定因果；待运行中的 Rust 任务结束后定向复测，不放宽超时。
- WebView 新一轮全套使用 2 workers，在多项子进程启动延迟后出现连续超时，已发送 SIGINT 停止这次并发复测：312 passed、2 failed、2 interrupted、556 did not run；日志 `/tmp/wisp-parity-projects-playwright.log`。待 Rust 结束后串行重跑。依赖安装已完成且 lockfile 未变；恢复本次生成的 research-journey 图片的 `git restore` 也正在等待启动，执行 session 14293，需确认完成。
- Rust workspace 全套仍在执行：`/tmp/wisp-parity-rust-full.log`，执行 session 50690；已通过的输出没有失败，尚不能宣称全套通过。该 run 启动于本批项目改动前，不能将其结果当作新增同步/导出接口的完整回归。
- 上一批标准 QA 构建与严格签名验证通过。本批含目录/历史/导出/同步的 `scripts/build_native_macos.sh --qa` 重新构建中，日志 `/tmp/wisp-parity-projects-build.log`，执行 session 99690。应用：`target/native-macos-qa/Wisp Science QA.app`；真实复验需在解锁后重新启动更新的 shell/helper。
- CUA 实机复验被 Mac 锁屏阻止，已请求用户解锁；没有据此将窗口生命周期或真实键盘路径标为完成。
- 测试仅使用隔离合成数据库：`/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`。QA app 的 database preference 指向它；QA host 的 Application Support symlink 指向同一目录。之前的临时 QA 链接路径记录在 `/tmp/wisp-parity-fixes-previous-qa-link.txt`。
- 未登录真实外部账户，未执行远程科学任务，未推送分支或创建远端 PR。

## 下一步

1. 解锁后优先用 Computer Use 验收 Cmd+W/红叉 → Finder/Dock 重开、项目/草稿恢复、host 不重复启动；验证两种发送偏好、问题卡片和反馈 Escape。
2. 收尾 Rust 完整回归结果；保留全套 WebView 的原始失败与复测记录。
3. 收尾本批项目能力的构建/测试/实机闭环；随后接入订阅认证、ACP、会话管理、论文证据。全部剩余项仍属本目标范围，当前未结项。
