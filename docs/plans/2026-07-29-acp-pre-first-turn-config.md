# ACP 首轮前模型配置修复计划

## 问题

空白会话选择 ACP Agent 时，前端目前只保存临时选择；真正的 ACP
session 要到第一条消息发送时才创建。因此 Agent 在 `session/new` 返回的
`configOptions`（例如 `Model`、`Reasoning effort`）在发送前不会显示。

## 验收标准

1. 空白会话选择 ACP Agent 后，Wisp 创建或复用一个空 frame，并在不发送
   prompt 的情况下初始化 ACP session。
2. Agent 返回的 `configOptions` 和 `modes` 在第一条消息发送前即可从发送
   按钮旁的模型菜单调整。
3. 初始化不会清空编辑器草稿，也不会生成用户或助手消息。
4. 从已有对话切换到 ACP 时，仍创建新的 ACP session，并保留原对话。
5. UI mock 测试、相关 Rust 测试、WASM check 和 Playwright 测试通过。

## 状态

- [x] 复现并定位延迟初始化问题
- [x] 冻结最小修复边界
- [x] 后端增加无 prompt 的 ACP prepare 命令
- [x] 前端在选择 ACP 时 prepare 并显示配置
- [x] 自动化与真实 macOS 界面验收

## 验收记录

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：通过；沙箱内 Keychain 用例因 macOS 权限失败后，
  已在沙箱外完整重跑并通过。
- `cd ui && cargo check --target wasm32-unknown-unknown`：通过。
- `cd ui-tests && npx playwright test`：232 项通过，1 项按测试配置跳过。
- macOS 调试应用真实验收：空白会话选择 `Codex ACP` 后，在未发送草稿
  `今天是几号` 的情况下显示 `Model`、`Reasoning effort`、`Mode`、
  `Collaboration mode` 和 `Fast mode`；模型列表与推理强度列表均可展开。
