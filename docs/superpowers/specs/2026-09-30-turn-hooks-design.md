# 回合钩子（Turn Hooks）

把"回合结束后自动做点什么"的零散系统收拢到一个后端入口：一个触发点、一套开关读取、一个模型后端解析、一种错误隔离方式。

## 现状

| 系统 | 触发位置 | 开关 | 旁路模型 | 结果 |
| --- | --- | --- | --- | --- |
| 自动审稿 | 后端，`agent_turn.rs` 两处（ACP、原生）；实现 `lib.rs::automatic_review` / `automatic_review_acp` 两份 | `frame_auto_review:{id}` → `auto_review_default_enabled` | Reviewer 专家，`resolve_review_backend` + `generate_review_with_backend` | 落库 + `Review*` 事件；有发现则纠正一轮再复审 |
| 失败分析 / 显式记忆提议 | 前端 `Done` 回调（`ui/src/main.rs`）调用 `propose_turn_memory(automatic)` | `memory_enabled` + `auto_failure_analysis`(JSON) | 复用 Reviewer 后端，但 `memory_commands.rs::generate_turn_memory_candidate` 又写了一遍 HTTP/ACP 分派 | invoke 返回值 → 前端弹窗 |
| 追问建议 | 前端 `Done` 回调调用 `generate_follow_up_questions` | `follow_up_questions`（前后端各查一次） | 会话/专家模型，第三份 `build_provider_config` | invoke 返回值 → 前端 |
| 手动审稿 | `review_session` 命令 | — | 同自动审稿 | 同自动审稿 |

问题：

1. **触发点分裂**。审稿在后端，记忆和追问在前端 `Done` 回调里。`send_message_inner` 的五个调用方（桌面、队列、IM、调度、委派回调）里，只有窗口在场时后两者才会跑；前端还得自己用 `turn_memory_loading`、`follow_up_generation` 做去重和过期判断。
2. **原生 / ACP 双份实现**。`automatic_review` 与 `automatic_review_acp` 流程相同，只差"读 transcript"和"追加一轮纠正"两个动作，却各写了一遍（约 230 行）。
3. **门控不一致**。原生路径只在 `AgentLoopOutcome::Completed` 时审稿；ACP 路径对任何 `Ok(stop_reason)` 都审（含 `max_tokens`、`refusal`）。前端记忆提议又用 `stop_reason is none or "end_turn"`。
4. **旁路模型调用三份**。审稿、记忆、追问各自解析专家 → 11 元组 → `build_provider_config` → `complete`。
5. **顺序和并发靠巧合**。记忆/追问看到的是纠正后的答案，只因为审稿恰好在 `Done` 之前 await；互斥分别靠 `state.reviewing`、前端 loading 集合、前端 generation 计数。

## 设计

新模块 `src-tauri/src/turn_hooks.rs`。放在 src-tauri 而不是 wisp-core：所有现有钩子都依赖专家配置、store 设置和 `AgentEvent`，CLI 目前一个都没有。

### 两个阶段

| 阶段 | 时机 | 能做什么 | 成员 |
| --- | --- | --- | --- |
| `Stop` | 循环结束后、`Done` 发出前，await | 可以让回合**再跑一轮**（至多一次） | `auto_review` |
| `AfterTurn` | `Done` 发出后，spawn | 只读 transcript，结果以事件推给前端 | `memory_proposal`、`follow_ups` |

分成两段是因为追问和记忆不能拖慢 `Done`：用户看到回合结束就该能继续输入。`Stop` 固定先于 `AfterTurn`，所以后者总是看到纠正后的答案——这个顺序从巧合变成了约定。

### 原生 / ACP 的唯一差异：`TurnDriver`

两种实现、集合封闭，所以用枚举而不是 trait：

```rust
pub(crate) enum TurnDriver<'a> {
    Native { agent: &'a mut Agent, output: &'a TauriOutput, model_label: &'a str },
    Acp { state: &'a AppState, app: &'a AppHandle, project: &'a ActiveProject, frame_id: &'a str },
}
// transcript()：原生读 agent.ctx，ACP 读 store
// continue_turn(prompt)：原生 inject_user + run_resume；ACP run_acp_internal_turn
// emit(event)、model_label()
```

`automatic_review` 与 `automatic_review_acp` 合并为 `run_stop` 里的一份逻辑。

### 钩子是一个封闭枚举

```rust
pub(crate) enum HookId { AutoReview, MemoryProposal, FollowUps }

impl HookId {
    fn as_str(self) -> &'static str;                            // HookFailed 事件里的名字
    async fn enabled(self, store: &Store, frame_id: &str) -> bool; // 读现有 setting key，不迁移
}
```

| 钩子 | 开关（`enabled`） | 是否值得跑 |
| --- | --- | --- |
| `AutoReview` | `frame_auto_review:{id}` → `auto_review_default_enabled` | `review::should_auto_review` |
| `MemoryProposal` | `memory_enabled` | 失败率达阈值（`auto_failure_analysis`）或用户明确要求记住 |
| `FollowUps` | `follow_up_questions`（默认开） | 本轮以答案结束：`attempt_completion` 结果或助手正文，而不是停在工具调用上 |

不做 trait 对象注册表：三个内置钩子在编译期全部已知，枚举 + `match` 更好读。

### 统一的回合事实：`TurnEnd`

```rust
pub(crate) struct TurnEnd<'a> {
    frame_id: &'a str,
    project_id: &'a str,
    stop_reason: Option<&'a str>, // 原生 Completed 为 None；ACP 为自己的原因
    resume: bool,
    reviewer_session: bool,
    turn_start: usize,
}
// completed() = stop_reason 为 None 或 "end_turn"，原生与 ACP 同一口径
```

门控：

- 两个阶段都要求 `completed()`。取消、`max_tokens`、`refusal`、`max_iterations` 都不跑钩子。此前 ACP 路径对这些结果也会审稿，现在已修正。
- `Stop` 另外跳过 `resume`（委派回调的续跑）和 Reviewer 专家自己的会话，与原行为一致。
- `AfterTurn` 不跳过它们。此前前端对这些回合的 `Done` 也会请求记忆和追问，现在保持不变。

### 执行

```rust
pub(crate) async fn run_stop(state, app, end: &TurnEnd, driver: &mut TurnDriver, cancel: &AtomicBool);
pub(crate) fn spawn_after_turn(app: &AppHandle, end: &TurnEnd);
```

- **互斥**：继续用 `state.reviewing`，手动审稿与自动审稿共用。
- **纠正预算**：审稿在结构上只调用一次 `continue_turn`，随后复审一次。
- **错误隔离**：钩子失败永远不把用户的回合变成 Error。审稿失败沿用 `ReviewFailed`；AfterTurn 失败发 `HookFailed`。
- **作废**：`AfterTurn` 使用每会话的 generation（`state.after_turn_generations`）。每次 spawn 递增 generation，并在执行前、发送前各检查一次：generation 已变或该会话又在运行时，直接丢弃。前端的 `follow_up_generation` 因此删除。前端只在收到 `FollowUps` 时再判断一次会话是否在运行，以覆盖"刚点发送、后端还没标记运行"这段竞态。

### 旁路模型：`side_complete`

```rust
pub(crate) enum SideModel {
    Reviewer { reviewer: Specialist, backend: Option<ReviewBackendConfig>, project_root: Option<PathBuf> },
    Session { max_tokens: u64 },
}
impl SideModel { async fn reviewer(state, frame_id) -> Result<Self, String>; } // 解析会话对应的 Reviewer 后端

pub(crate) async fn side_complete(
    state, frame_id, purpose: &str, model: SideModel, system: &str, user: &str, cancel,
) -> Result<SideCompletion, String>; // { text, backend, model, effort }
```

审稿、记忆候选、追问、Reviewer 后端测试都改为调用它；`log_dev_llm_dispatch` 也只在这里打（`{purpose}_http` / `{purpose}_acp`）。ACP 后端的提示为 `system + "\n\n" + user`。审稿的 transcript 在两种后端上都包在"不可信、只读证据"的说明里：此前只有 ACP 这样做，现在 HTTP 审稿也加上了。

### 事件

已落库的 `ReviewStarted` / `Review` / `ReviewFailed` / `CorrectionStarted` 不变，历史会话要靠它们重放 `ChatItem::Review`。在 `wisp-dto` 与后端 `AgentEvent` 中各新增三个**只推送、不落库**的事件，生命周期与原先 invoke 的返回值一致：

```rust
AgentEvent::MemoryProposal { frame_id: String, proposal: TurnMemoryProposal },
AgentEvent::FollowUps { frame_id: String, questions: Vec<String> },
AgentEvent::HookFailed { frame_id: String, hook: String, message: String },
```

前端对 `HookFailed` 只在 `memory_proposal` 失败时给出状态提示，行为与原先自动请求失败时相同。追问失败保持静默。不引入通用的 `Hook { id, payload }` 信封：前端对每种结果的渲染各不相同。

### 命令变更

- `propose_turn_memory` 去掉 `automatic` 参数，只保留手动入口。自动入口是 `memory_commands::automatic_turn_memory_proposal`。
- `generate_follow_up_questions` 命令删除。

## 不做（及何时做）

- **`TurnStart` 阶段**：目前只有全局记忆注入，两处各一行调用同一函数。出现第二个注入方时再加。
- **`PreToolUse` / `PostToolUse`**：审批、资源租约、溯源已有各自管线，没有零散消费者。要做用户可否决工具调用的钩子时再加。
- **用户自定义钩子**（`.wisp/hooks.toml`，类似 Claude Code hooks）：`HookId` 增加 `Custom(name)` 变体即可接入同一 runner。届时须走 `wisp-runs` 而不是 shell 工具超时；须遵守探索分支只读锁与 IM 来源的强制询问；不得读 keyring。
- **统一钩子设置页**：先保留现有 setting key 与各自开关。要做"本会话启用了哪些钩子"的总览时，加一个只读 `get_session_hooks(frame_id)` 聚合 `HookId::enabled`。

## 验证

- Rust：`turn_hooks::tests` 覆盖原生/ACP 共用的完成口径、Stop 对 resume 和 Reviewer 会话的跳过，以及"以答案结束"的判断；`dto_contract_tests::after_turn_hook_events_roundtrip_to_ui` 覆盖三个新事件的前后端形状；`persisted_ui_events_ignore_ephemeral_reviewer_handoffs` 确认新事件不落库。
- Playwright：mock 在 `Done` 后按后端同样的规则推送 `FollowUps` / `MemoryProposal`；追问、记忆提议、仅工具结束、ACP 取消等用例改为验证事件驱动的界面。
- 审稿流程本身（有发现 → 一次纠正 → 复审）需要真实模型，没有自动化用例；用例覆盖的是它依赖的纯函数（`review::tests`）。
