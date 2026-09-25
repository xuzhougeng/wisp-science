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

## 后续范围（全部保留，不能因第一批完成而结项）

- [x] 内置问题卡片：不覆盖草稿、pending/answered/expired、自由输入暂存、导航后旧回调隔离。ACP request ID 已解析，实时响应留在 ACP 项。
- [ ] 计划与审批：反馈已完成；plan mode 决策与普通工具授权范围仍待实现；维持现有 Agent workflow 审批。
- [ ] 项目：目录导入、目录导出、历史恢复、快照/同步状态。
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
- Swift 全套：213 tests，0 failures，启用 `WISP_NATIVE_SNAPSHOT_DIR`，包含所有 opt-in render。最终日志：`/tmp/wisp-parity-final-swift.log`。
- `cargo fmt --all -- --check` 通过；`ui` wasm check 通过。
- WebView Playwright 全套：867 passed、2 skipped、3 failed。两项启动超时单独复测通过；原有首页文档按钮测试在 hydration 前读取空按钮列表，改为等待期望顺序。修复后的相关 4 项复测全部通过；没有把这次定向复测冒充整套绿色。日志：`/tmp/wisp-parity-playwright.log`、`/tmp/wisp-parity-playwright-verified.log`。
- Rust workspace 全套仍在执行：`/tmp/wisp-parity-rust-full.log`，执行 session 50690；已通过的输出没有失败，尚不能宣称全套通过。
- 标准 `scripts/build_native_macos.sh --qa` 完整构建通过；后续 Swift 变更已增量编译并重新打包、严格签名校验。应用：`target/native-macos-qa/Wisp Science QA.app`。
- CUA 实机复验被 Mac 锁屏阻止，已请求用户解锁；没有据此将窗口生命周期或真实键盘路径标为完成。
- 测试仅使用隔离合成数据库：`/private/tmp/wisp-parity-fixes-20260925/native/wisp.sqlite`。QA app 的 database preference 指向它；QA host 的 Application Support symlink 指向同一目录。之前的临时 QA 链接路径记录在 `/tmp/wisp-parity-fixes-previous-qa-link.txt`。
- 未登录真实外部账户，未执行远程科学任务，未推送分支或创建远端 PR。

## 下一步

1. 解锁后优先用 Computer Use 验收 Cmd+W/红叉 → Finder/Dock 重开、项目/草稿恢复、host 不重复启动；验证两种发送偏好、问题卡片和反馈 Escape。
2. 收尾 Rust 完整回归结果；保留全套 WebView 的原始失败与复测记录。
3. 继续目录导入/导出/快照状态，然后订阅认证、ACP、会话管理、论文证据；以上未完成项仍是本目标范围。
