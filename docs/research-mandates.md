# 研究职责 / Research mandates

定时任务回答「什么时候发一段提示词」。研究职责回答「这件事由谁负责到底」：你写清目标、怎样衡量、哪些事可以不问就做，项目里的 Agent 在自己的一条对话中一轮轮推进。

入口：科研助理 → 顶栏「自动化」→「研究职责」。

## 定义一份职责

点击「新建职责」，或从模板开始（文献追踪、投稿与返修跟进、长期计算任务、方法指标迭代）。模板只是预填，提交前每一项都可以改。

- **目标（Goal）**：为谁工作、希望达成什么结果、持续多久。名称留空时取目标的第一行。
- **KPI**：每行一个指标，写清名称、统计口径、目标值和统计周期。目标值和当前值都是数字时，卡片和 Agent 都能看到差距；口径写得越具体，两次统计越一致。没有名称的行会被忽略，最多 12 个。
- **工作约束（Constraints）**：
  - 可自行完成、需要先审核、何时请求协助，三段文字原样交给 Agent。
  - 「修改文件、运行命令或提交任何内容之前先问我」默认勾选。勾选后，这条对话里所有会改动状态的工具都要先经你审批，即使项目或会话设置为免审批。不勾选则沿用项目自己的审批设置。
- **截止日期**：到期当天结束后职责自动关闭。留空表示长期职责，按阶段检查。
- **工作节奏**：默认每隔多少小时进行一轮；最短间隔、最长间隔是 Agent 自行安排下一轮时间时的边界，每天最多回合限制自动触发的次数。最短间隔不低于 5 分钟，最长间隔不超过 30 天。

创建后立即开始第一轮。职责在所选项目里拥有**一条固定对话**（标题为职责名称），每一轮都在其中继续，沿用常规的归档式压缩。对话被删除或归档后，下一轮会新开一条并自动接上。

## 每一轮

到点后 Wisp 向职责对话发送一条 `[Mandate round: 名称]` 消息，像普通对话一样运行。无论是到点触发，还是你自己在这条对话里输入，Agent 每一轮都会在请求之前收到最新的职责简报：目标、周期、每个 KPI 的口径/目标/当前值、全部约束，以及最近 3 轮的账本。简报每轮现取，不写入对话历史，所以修改目标或 KPI 后下一轮立即生效，压缩也不会丢掉职责本身。

### 回合账本

一轮工作做完后，Agent 调用 `end_round` 收尾，写下四件事并安排下一轮：

- **已完成**：这一轮做了什么，附运行名称和产出路径。
- **KPI**：这一轮测得的指标当前值，按职责里的指标名称填写。卡片上的 KPI 随之更新；职责里没有定义的指标名不会被记录，Agent 会被告知。
- **阻塞**：卡在哪里、等谁。
- **下一步**：下一轮先做什么。
- **下一轮时间**：若干分钟后，或一个本地时间。快出结果时安排得近一些，只是在等待时安排得远一些。时间会被夹在职责的最短 / 最长间隔之内，也不会晚于截止日期；被调整时 Agent 会被告知。不指定则按默认间隔。

下一轮的简报来自这份账本，而不是整段对话历史。卡片显示最新一轮的记录。

`end_round` 只写职责自己的账本和时间，不改动项目，所以即使勾选了「先问我」也不需要审批，回合可以无人值守地收尾。一轮没有调用 `end_round` 就结束时（例如模型出错、被停止或达到轮次上限），Wisp 把这一轮的最终回答记入账本并标明「未提交回合报告」，下一轮时间保持默认间隔。

当天账本条目达到「每天最多回合」后，到点的自动回合顺延到本地次日零点。「立即运行一轮」不受此限制。

## 管理

每份职责一张卡片，显示状态、所属项目、下一轮时间、截止日期、目标、KPI 当前值 / 目标值和最新一轮的账本记录。

- 开关：暂停 / 恢复。暂停期间到点的回合在恢复后的下一次轮询补跑一次。
- 「立即运行一轮」：马上运行，不改变原定的下一轮时间。已结束的职责不能运行。
- 编辑：修改目标、KPI、约束、截止日期和节奏。项目不可更改。正在运行的回合按开始时的简报跑完。
- 删除：再次点击确认。只删除职责，它的对话作为历史留在项目里。

状态：进行中、已暂停、等你处理、已结束。隐私模式隐藏的项目，其职责也不显示。

## 边界

- 仅在 Wisp 运行时触发，没有系统级后台服务。关闭期间错过的回合在下次启动后补跑一次，之后的节奏从这次补跑重新计算。
- 职责不随项目导出、导入或同步，和定时任务一样属于本机状态。删除项目会一并删除它的职责。
- ACP 外部 Agent 的对话也会收到职责简报，但审批由外部 Agent 自己管理，「先问我」不对它生效；它也没有 `end_round` 工具，每一轮都由 Wisp 代记账本。
- 原生 macOS / Windows 客户端暂未提供职责界面。

## English

A scheduled task answers "when is this prompt sent". A **research mandate** answers "who sees this through": you state the goal, how it is measured and what may be done without asking, and a project agent carries it round by round in a conversation of its own. Open it from the research assistant → **Automation** → **Research mandates**.

- **Define**: **New mandate**, or start from a template (literature watch, submission follow-up, long-running compute, method metric iteration). A mandate has a **goal**; **KPIs**, each with a name, how it is counted, a target and the period the target applies to; and **constraints** — what the agent may do on its own, what you review first, and when it should ask for help. *Ask me before anything that changes files, runs commands or submits work* is on by default: with it, every state-changing tool in the mandate's conversation asks for approval even where the project or session would allow it. An optional end date closes the mandate when that day is over. The rhythm fields set the default hours between rounds and the bounds (soonest, latest, rounds per day) for rounds the agent schedules itself.
- **Rounds**: the first round starts right away. Each round sends `[Mandate round: name]` into the mandate's one conversation and runs as an ordinary turn. Every turn there — a due round or a message you type — is given the current brief (goal, period, KPI definitions, targets and current values, constraints, and the last three ledger entries) ahead of the request. The brief is read fresh each turn and is never saved into the transcript, so an edit applies from the next round and compaction cannot lose the mandate. A deleted or archived conversation is replaced on the next round.
- **Ledger**: a round ends by calling `end_round` with what it did, the KPI values it measured, what is blocked, the next step, and when the next round should happen (minutes from now, or a local time). The next round is briefed from this ledger rather than the whole transcript. The requested time is kept within the mandate's soonest/latest bounds and never past its end date, and the agent is told when it was moved; without one, the default cadence applies. KPI names the mandate does not define are not recorded. `end_round` writes only the mandate's own ledger and schedule, so it needs no approval even when the mandate reviews every change. A round that ends without calling it (a model error, a stop, the iteration cap) is recorded by Wisp from its final answer and marked as unreported. Once a day's ledger entries reach **Rounds per day at most**, due rounds wait for local midnight; **Run a round now** is exempt.
- **Manage**: each card shows status, project, next round, end date, goal, each KPI's current value against its target, and the latest ledger entry. Pause or resume with the switch; **Run a round now** leaves the next scheduled round untouched; edit everything except the project; delete with a second click (the conversation stays in the project).
- **Limits**: rounds run only while Wisp is open, and missed rounds collapse into one after the next launch. Mandates are machine-local like scheduled tasks: they are not exported, imported or synced, and are deleted with their project. Projects hidden by privacy mode hide their mandates. ACP conversations receive the brief but keep their own approvals and have no `end_round`, so Wisp records each of their rounds. The native macOS and Windows clients have no mandate surface yet.

## 实现说明 / Implementation

- 表 `mandates`（迁移 `0064_mandates`，每次打开时幂等执行），和定时任务一样随项目数据库路由。KPI 与约束以 JSON 列保存；`frame_id` 不是外键，职责不因对话被删而消失。
- 职责自带 `next_run_at`，不复用 `schedules` 行：每一轮的下次时间由上一轮决定，没有固定节拍可锚定；也避免职责出现在定时任务列表和原生客户端里。到点扫描 `due_mandates` 与原子认领 `claim_mandate_round` 和定时任务同构，由同一个 30 秒调度轮询驱动（`scheduler.rs` → `mandates::fire_due_mandates`）。
- 写入按职责拆分：`update_mandate` 只写定义（名称、目标、KPI、约束、周期、节奏），状态、对话绑定和回合时间各有独立写入，表单保存不会覆盖期间被认领的回合或被改变的状态。
- 账本表 `mandate_rounds`：每份职责内 `seq` 从 1 起连续编号，在写事务里取 `MAX(seq)+1`；`source` 为 `agent`（`end_round`）或 `host`（Wisp 代记）。
- 跨边界类型在 `wisp-dto::mandate`（`MandateRecord` / `MandateDraft` / `MandateOverview` / `MandateKpi` / `MandateConstraints` / `MandateRound`）。与宿主无关的规则在 `wisp-app::mandates`：草稿校验与归一化、每轮简报 `brief`、回合提示词、`end_round` 工具、`record_round`（写账本、更新 KPI、排下一轮）和 `next_run_at`（把请求的时间夹进边界与周期）。放在 `wisp-app` 是为了让桌面端和无头 eval 用同一份实现。
- 回合管线：`agent_turn.rs` 在每轮按 `(project_id, frame_id)` 查一次 `mandates::turn_context`，把简报作为运行时注入放在用户请求之前，并把 `review_mutations` 并入 `force_ask_mutations`（与 IM 回合的审批下限是同一个开关）。职责对话的 Agent 额外注册 `end_round`；对话成为或不再是职责对话时，缓存的 Agent 会重建。
- `end_round` 声明为 `read_only`：它不触碰项目状态，因此不受审批下限、计划模式和探索冻结的拦截。
- 回合结束后，`mandates::run_round` 检查本轮开始后是否有账本条目，没有则调用 `record_unreported_round`。每日上限按本地日内的账本条目数计算（`rounds_today`），超限时把 `next_run_at` 推到当日结束。
- 命令：`list_all_mandates`、`create_mandate(draft)`、`update_mandate(id, draft)`、`set_mandate_status(id, status)`、`delete_mandate(id)`、`run_mandate_now(id)`。

手动检查：从「文献追踪」模板为第二个项目创建职责，确认卡片上的项目、下一轮时间和 KPI；约 30 秒内该项目出现以职责命名的对话并开始第一轮；回合结束后刷新，卡片出现「第 1 轮」记录、KPI 当前值和 Agent 选择的下一轮时间；在对话里让 Agent 把下一轮安排在 1 分钟后，确认时间被调整到最短间隔；编辑目标后「立即运行一轮」，确认新一轮使用新目标并在简报里看到上一轮账本；勾选「先问我」后让 Agent 写一个文件，确认出现审批，而 `end_round` 不需要审批；暂停、恢复、删除。打开表单后立即按 Escape，只关闭表单，自动化页保留。

自动测试：`wisp-app` 用脚本化 Provider 让真实的 Agent 循环跑三轮，断言账本连续、下一轮时间跟随 Agent 的请求、越界时间被夹回边界；无头 eval 套件的 `mandate-rounds` 用例覆盖同一条路径。
