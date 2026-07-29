# ACP 会话参与者 MVP 执行计划

关联 Issue：[#560](https://github.com/xuzhougeng/wisp-science/issues/560)

## 目标

在不改变普通 HTTP 会话、单 ACP 会话、Review 和 Delegation 语义的前提下，为后续“同一父会话中通过 `@` 点名不同订阅型 ACP Agent”建立一个可验证的最小纵向切片。

首个切片只支持：

- 从编辑器 `@` 菜单为一条用户消息附加一个显式 ACP participant reference；
- 使用稳定的 ACP profile ID 路由，不按 Codex、Claude、Kimi 等供应商名称特判；
- 生成有界的共享上下文增量，避免向同一 participant 重复发送完整父会话；
- 保存 participant/profile、显示名称、ACP session lineage 和同步 sequence；
- 为后续 Tauri/UI 接线提供纯函数与持久化边界。

## 非目标

- `@all`、并行回答或自动 roundtable；
- Agent 主动点名另一个 Agent；
- HTTP model participant；
- 浏览器 Cookie、网页私有接口或订阅 Token 管理；
- 一次性重写现有 ACP session 生命周期。

## 机械验收

1. 路由测试覆盖：
   - UI 使用稳定 profile ID 序列化唯一 participant reference；
   - 后端接受一个 participant，拒绝空 ID、重复或歧义选择；
   - participant reference 不会被误当成普通 artifact/context 引用。
2. 上下文增量测试覆盖：
   - 首次调用获得有界父会话快照；
   - 再次调用只获得 participant 尚未看到的消息；
   - participant 自己生成的回复不会作为“外部未读消息”重复注入。
3. SQLite 测试覆盖：
   - participant binding 可写入、读取和更新；
   - profile 删除后历史快照仍存在；
   - migration 重复运行保持幂等。
4. 相关 Rust 单元测试通过。
5. `cargo fmt --all -- --check` 通过。
6. 若本切片接入 UI/Tauri，则补充 WASM check 与 mocked Playwright 测试；否则在 PR 中明确列为下一切片。

## 状态

- [x] Issue 查重与范围冻结
- [x] 独立工作树建立
- [x] 现有消息与 ACP session 边界审计
- [x] 纯路由/上下文模型实现
- [x] 持久化实现
- [x] Tauri/UI 最小接线
- [x] 测试与文档
- [x] Draft PR：[#562](https://github.com/xuzhougeng/wisp-science/pull/562)
