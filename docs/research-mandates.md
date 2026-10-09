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
- **汇报间隔**：每隔多少天写一份汇报，默认 7 天，范围 1–90 天。
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

### 等待一个 Run

回合提交了长时间计算后，可以在 `end_round` 里给出那个 Run 的 ID（`wait_for_run_id`）。Run 一结束（成功、失败、取消、超时或失联），下一轮立刻开始，不需要 Agent 反复轮询；它选的下一轮时间作为最晚时间保留。Run 已经结束、不存在或属于别的项目时不等待，Agent 会被告知。

## 需要你

登录、身份确认、补资料、付款授权、发布，以及本该由你做的判断，Agent 不该猜，也不该反复重试。遇到这类情况它调用 `request_assistance`，写清三件事：需要你做什么、为什么卡住、你完成后它怎样继续。

发出请求后：

- 职责状态变为「等你处理」，**不再触发任何回合**，等待期间不消耗模型。
- 请求会出现在职责卡片上，同时发到科研助理的对话；已启用的微信（科研助理入口）和飞书机器人的所有者也会收到同一段文字。微信受 iLink 回复窗口限制，窗口过期时消息保留在当前连接里，等你下一条消息到达后送出。隐私模式隐藏的项目不发任何通知。
- 同一份职责同一时间只有一个未答复的请求；它再次求助时，旧请求自动撤回。

在任意一处回复都会让职责继续：

- 职责卡片上的回复框；
- 直接在该项目的职责对话里发消息（桌面，或通过项目机器人 `/session` 切到这条对话后发送）。

回复即答复：请求关闭，状态回到「进行中」，带着你的回复的这一轮马上开始，Agent 在简报里看到「研究者已答复」以及你的原话。原定的下一轮顺延一个默认间隔，避免紧接着再跑一轮。请求一旦答复，后来的消息只是普通反馈，不会改写答复内容。

不想回答、想让它先做别的：把卡片上的开关关掉再打开，状态回到「进行中」，回合照常进行，请求仍留在简报里。

`request_assistance` 和 `end_round` 一样只改动职责自身，不需要审批。

## 汇报

每到汇报间隔，Wisp 为上次汇报以来的这段时间写一份汇报：

- **已完成**：这段时间各回合完成的工作和产出。
- **进展**：每个 KPI 相对目标的位置，以及记录里能解释差距的原因。KPI 的当前值 / 目标值另外按写汇报时的数值原样列出，不经过模型。
- **阻塞**：未解决的问题、失败的运行，以及仍在等你处理的请求。
- **下一步**：最近几轮计划接着做什么。

每一条都注明它依据的记录：职责自己的回合（第 N 轮），以及这段时间项目里登记的运行、产出、研究记录和会话。模型引用了不存在的记录时，该引用会被丢弃。

汇报由 **设置 → 专家 → Recap** 绑定的模型起草，输入是这段时间的账本和研究历程摘要，不读取完整对话。两种情况不调用模型：

- **安静的周期**：这段时间没有任何回合。汇报直接说明「本期没有回合记录」并列出 KPI；职责仍在等你时会写明等的是什么。
- **模型不可用或返回无效内容**：改用账本原文整理，每条引用对应的回合，并标注「由账本整理」。

所以每个周期都有一份汇报，等待中的职责也会按期提醒你。汇报出现在职责卡片的「汇报」里（卡片显示最近 5 份，点击展开），同时发到科研助理的对话，已启用的微信和飞书也会收到文字版。隐私模式隐藏的项目不发送。

卡片上的「立即生成汇报」马上为上次汇报以来的时间写一份，不改变原定的汇报时间。已暂停和已结束的职责不再自动汇报。

## 管理

每份职责一张卡片，显示状态、所属项目、下一轮时间、截止日期、目标、KPI 当前值 / 目标值、最新一轮的账本记录和历次汇报。

- 开关：暂停 / 恢复。暂停期间到点的回合在恢复后的下一次轮询补跑一次。
- 「立即运行一轮」：马上运行，不改变原定的下一轮时间。已结束的职责不能运行。
- 编辑：修改目标、KPI、约束、截止日期和节奏。项目不可更改。正在运行的回合按开始时的简报跑完。
- 删除：再次点击确认。只删除职责，它的对话作为历史留在项目里。

状态：进行中、已暂停、等你处理（有未答复的请求）、已结束。隐私模式隐藏的项目，其职责也不显示。

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
- **Waiting on a run**: `end_round` takes a `wait_for_run_id`. The next round starts as soon as that run reaches any final state, with the chosen time kept as the latest start, so the agent never polls. A run that has already ended, does not exist or belongs to another project is not waited on, and the agent is told.
- **Asking for help**: for a sign-in, missing materials, a judgement that is yours, a payment or a release, the agent calls `request_assistance` with what it needs, why, and how it will continue, instead of guessing or retrying. The mandate becomes **Needs you** and no round runs — waiting costs no model calls. The request appears on the card and in the research assistant's conversation, and is sent to the assistant's WeChat and the Feishu bot's bound owner when those are enabled; WeChat delivery follows iLink's reply window. A hidden project sends nothing. Only one request is open per mandate; a newer one withdraws it. Reply on the card or by writing in the mandate's conversation: the reply closes the request, the mandate is active again, and the turn carrying your words is the next round. The scheduled round moves one default interval out. Later messages are ordinary feedback and never rewrite the answer. Toggling the card's switch off and on resumes rounds without answering. Like `end_round`, `request_assistance` needs no approval.
- **Reports**: on the report cadence (7 days by default, 1–90) Wisp writes a report for the time since the last one: what was done, where each KPI stands and what the records say explains the gap, what is blocked (including a request still waiting on you), and what comes next. Every item cites the records behind it — the mandate's own rounds and the project's recorded runs, outputs, notes and conversations — and a citation of a record that does not exist is dropped. KPI values are listed as they stood, without going through a model. The Recap specialist's model drafts it from the period's ledger and research-journey digest, never the full transcript. A period with no rounds is reported without a model call, and a model failure falls back to the ledger's own words, marked *from the ledger* — so every period has a report and a waiting mandate keeps reminding you. Reports stay on the card (the latest five, expandable) and go to the assistant's conversation and the enabled WeChat/Feishu channels. **Write a report now** covers the time since the last report and leaves the cadence alone. Paused and closed mandates do not report.
- **Manage**: each card shows status, project, next round, end date, goal, each KPI's current value against its target, the latest ledger entry and past reports. Pause or resume with the switch; **Run a round now** leaves the next scheduled round untouched; edit everything except the project; delete with a second click (the conversation stays in the project).
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
- 请求表 `mandate_requests`（`open` / `answered` / `withdrawn`）。`request_assistance` 写入请求并把职责置为 `waiting`；到点扫描只取 `active`，所以等待期间不会认领回合。工具通过宿主注入的 `AssistanceNotifier` 回调通知，`wisp-app` 自身不依赖任何通道。
- 通知：`mandates::announce` 先核验项目可见性，再用 `dispatch::post_reply` 把文字作为独立的助理回复追加到助理对话（与派活完成回报同一条路径），并调用 `channels::notify_owner` 推送到助理微信绑定和飞书所有者（`FeishuRest::send_text_to_user`，按 `open_id` 发送）。通知在后台发出，提出请求的回合不等待网络。
- 答复：`turn_context` 收到研究者本人发出的消息（非续跑、非 timer、非 `[Mandate round: …]`）且职责处于 `waiting` 时调用 `answer_request`，然后才生成简报。卡片的 `reply_to_mandate` 先同步答复，再把文字作为普通消息发进职责对话。`answer_mandate_request` 用 `UPDATE … WHERE status='open' RETURNING` 保证只答复一次。
- 汇报：`mandate_report.rs`。`build_report` 读取周期内的账本（最多最近 40 轮）和 `research_recap::day_digest` 的同期摘要，回合用 `T{seq}` 句柄、项目记录沿用回顾的 `R/O/N/S` 句柄；模型输出交给 `research_recap::to_recap` 解析并剔除未知引用，所以汇报正文就是一份 `ResearchRecap`（`findings` 作「进展」、`issues` 作「阻塞」）。模型调用以闭包注入，测试不触网。`ledger_body` 是无模型时的确定性正文。表 `mandate_reports` 把整份 `MandateReport` 存成一列 JSON。到期扫描 `mandates_due_for_report` 只取 `active` / `waiting`，`claim_mandate_report` 原子推进 `next_report_at`；进程内的 `Reporting` 守卫避免轮询与「立即生成」同时写同一周期。
- 等待 Run：`mandates.wait_run_id`。调度轮询在到点扫描之前调用 `wake_on_finished_runs`，把已结束或已不存在的 Run 对应的职责的 `next_run_at` 提前到当前时刻。Run 状态由 `wisp-runs` 的后台对账更新。
- 命令：`list_all_mandates`、`create_mandate(draft)`、`update_mandate(id, draft)`、`set_mandate_status(id, status)`、`delete_mandate(id)`、`run_mandate_now(id)`、`reply_to_mandate(id, text)`、`report_mandate_now(id)`。

手动检查：从「文献追踪」模板为第二个项目创建职责，确认卡片上的项目、下一轮时间和 KPI；约 30 秒内该项目出现以职责命名的对话并开始第一轮；回合结束后刷新，卡片出现「第 1 轮」记录、KPI 当前值和 Agent 选择的下一轮时间；在对话里让 Agent 把下一轮安排在 1 分钟后，确认时间被调整到最短间隔；编辑目标后「立即运行一轮」，确认新一轮使用新目标并在简报里看到上一轮账本；勾选「先问我」后让 Agent 写一个文件，确认出现审批，而 `end_round` 不需要审批；暂停、恢复、删除。打开表单后立即按 Escape，只关闭表单，自动化页保留。

手动检查协助请求：在职责对话里让 Agent「需要我登录某个系统时向我求助」并运行一轮；确认卡片变为「等你处理」并显示请求，助理对话出现「职责需要你」，已启用的微信 / 飞书收到同一段文字；等待两分钟确认没有新回合；在卡片上回复，确认状态回到「进行中」、对话里立即开始新一轮并引用你的答复。

手动检查汇报：新建职责后立即点「立即生成汇报」，确认得到「本期没有回合记录」且没有模型调用；跑完一两轮后再生成，确认各段内容来自这几轮、每条后面标着「第 N 轮」，助理对话里出现「职责汇报」文字版；把 Recap 专家绑到一个无效模型后再生成，确认仍有汇报并标注「由账本整理」。

自动测试：汇报生成用注入的假模型验证，包括安静周期不调用模型、未知引用被丢弃、模型失败或返回空内容时回退到账本。`wisp-app` 用脚本化 Provider 让真实的 Agent 循环跑三轮，断言账本连续、下一轮时间跟随 Agent 的请求、越界时间被夹回边界；同样通过真实循环调用 `request_assistance`，断言宿主只被通知一次、等待期间没有到点的职责、答复后恢复且不能重复答复；等待 Run 的唤醒用存储里的 Run 记录验证，不需要真实计算。无头 eval 套件的 `mandate-rounds` 用例覆盖账本路径。
