# ACP profile 会话默认配置实施计划

## 基线与边界

- 原始补丁基线：官方 `origin/main` 的 `cb02a0e9a754fa1f44f80841dc0506a547bbacb4`。
- 发布前重放基线：官方 `origin/main` 的 `21fa518d6dddd308b84a3a509e2567f3df86a69e`，产品版本仍为 `1.4.0`。
- 分支：`codex/acp-profile-config-defaults`。
- 不修改或替换 `/Applications/wisp-science.app`，不写真实 Wisp DB、真实项目数据或远端。
- 不复用旧 `my-build`、Plan B 或 `codex/acp-pre-first-turn-config` 实现。
- 不在本补丁加入 composer 内的完整 quick-switch UI；该能力仍由 Issue #889 跟踪，后续需单独处理 session lifecycle 与 `configOptions` availability。

## 设计选择

采用 ACP profile 持久化 `config_defaults`，不在选择 Agent 时 eager-create session。

1. profile 以通用 JSON object 保存 ACP option ID 到期望值的映射。
2. fresh `session/new` 返回 `configOptions` 后，Wisp 只匹配 Agent 实际公布的 option ID 和合法值。
3. 合法 defaults 在首条 `session/prompt` 前依序调用 `session/set_config_option`。
4. 未知 ID、未知 option type、非法值或 Agent 拒绝均跳过，保留 Agent 默认，不阻断首条 prompt。
5. defaults 只作用于 fresh session；resume/load 不重复应用，因此会话内手动选择可覆盖 profile default。

## 修改范围

- ACP profile DTO、持久化兼容和设置表单。
- fresh ACP session 的 default 解析/应用 helper 与 runtime 接入。
- mock ACP/UI 测试、Rust 单元/进程测试及用户文档。

## 验收

- fresh session 的默认值在第一条 prompt 前生效。
- profile JSON 往返及重启后重新读取仍保留 defaults。
- 未知/不支持 option fail gracefully。
- 不硬编码 Claude、Codex、Kimi 的 option ID 或能力；只消费 Agent 返回的 `configOptions`。
- 会话内手动 `set_config_option` 仍能覆盖已应用的 profile default。

## 实施状态与验证

- 已完成：profile DTO/持久化、设置表单、fresh session 默认值应用、delegated ACP fresh session 一致行为、文档与测试。
- `cargo test -p wisp-tauri acp::tests`：13 passed。
- `ui/ cargo test acp_config_defaults_tests`：2 passed。
- `cargo fmt --all -- --check`：passed。
- `cargo test --workspace`：passed。受限 sandbox 内首轮有 4 个 `wisp-llm` 本地 HTTP mock tests 因端口操作返回 `Operation not permitted`；在受限环境外重跑同一命令后全套通过，其中 `wisp-tauri` 为 798 passed，另有 1 个既有 doc-test ignored。
- `ui/ cargo check --target wasm32-unknown-unknown`：passed。
- `ui-tests/ npm ci`：passed，0 vulnerabilities。
- `ui-tests/ npx playwright test`：411 passed，1 skipped，0 failed；全程使用 mocked Tauri bridge。
- `git diff --check`：passed。

未执行：真实 Claude/Codex/Kimi adapter 端到端、安装 App、修改真实 profile/DB。
