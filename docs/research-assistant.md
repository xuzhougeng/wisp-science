# 科研助理 / Research assistant

首页右上角「科研助理」打开一条**不属于任何项目、永不新建的对话**。它像一位科研秘书：汇报你做过什么、记住你打算做什么、把具体工作交给项目里的对话去完成。它自己不写代码、不改文件、不运行命令或分析，也不检索文献。

## 能做什么

选择项目后发送问题，会读取该项目已保存的对话作为上下文；发送时重新核验隐私可见性。发送失败会保留草稿和顶部项目选择，不会额外生成重复的项目附件卡片。

- **汇报**：「我昨天做了什么？」「这周 RNA-seq 项目有什么进展？」——读取各项目已登记的研究活动（Run、产出、研究记录、会话中的请求）和每日回顾，与[研究日历](research-journey.md#首页研究日历--home-research-calendar)同源。一次最多读 7 天；只说记录里有的内容。
- **记计划**：「记住我今天要写完 Methods、催合作者的数据」——计划逐条保存，带日期和状态（待办 / 完成 / 放弃）。之前没完成的条目会自动出现在今天的列表里。计划存在对话之外，长对话压缩后也不会丢。
- **选择对话和服务器**：「查看 RNA-seq 项目下有哪些对话」——列出最近对话及其 ID，可按标题搜索，同时读取已配置服务器的名称和 ID。
- **派活**：「在 QC 对话下发送：重新做样本质控，注意绑定服务器 CPU2」——继续指定的已有会话，保留原标题和历史，先将 CPU2 选为该会话的默认执行环境，再发送完整指令。也可以要求新建对话。项目 Agent 实际接收指令后，助理回复任务已开始、结束后会提醒，然后结束本轮；不会反复查询等结果。派出的工作记入今天的计划，项目中的操作仍按项目规则审批。正在执行的对话会提示忙碌，不会中途改绑服务器。
- **完成回报**：项目 Agent 的本轮工作结束后，助理使用自己的模型阅读本轮结果，对照原任务汇总成果、输出文件及未完成事项，再回复到同一条助理对话。失败、取消和达到轮次上限会明确报告，不把“回合结束”视作“科研任务成功”。微信发起的工作会向原绑定账号回传摘要。
- **代为判断审批**：助理派发的项目任务遇到确认时，会在后台对照你的原始请求和具体操作判断。授权范围内、详情充分的操作批准一次；拿不准、详情不足或判断失败时，会在助理对话和原微信入口显示项目、对话、操作和原因，由你回复 `yes` / `no` 或点击桌面按钮。选择按原审批 ID 转交；已在项目里处理的请求会自动撤销，不会误批下一步。项目自己的完全权限可用于普通操作，助理完全权限不会继承到项目，也不创建新的项目或全局放行规则。
- **跟进**：「刚才那个任务跑完没？」——也可随时主动查看派出会话是否仍在运行，以及它的最终回答。

## 使用

1. 首页点击「科研助理」。打开后显示之前的全部对话（按页加载），并聚焦输入框。
2. 按 Escape 或右上角关闭按钮回到首页；助理正在进行的回复会在后台继续。
3. 每次发送时，助理会获知当前的本地日期和时间（不写入对话记录），据此理解「今天」「昨天」。
4. 模型使用该对话在输入框中选择的模型。

科研助理页采用三栏布局，首页的项目制入口保持不变：

- 左侧列出可见项目。选择项目会为后续提问附上该项目的上下文，输入框上方显示可取消的项目标记；不会切换项目或另开对话。「全部项目」取消这一限定。
- 中央保留同一条持续对话。顶部两个侧栏按钮可独立隐藏或展开项目列表、研究日历；桌面窗口记住展开偏好。隐藏侧栏不会清空草稿、重载对话或重置日历日期。
- 右侧显示月历、所选日期的已保存计划与研究记录。计划来自助理保存的条目，标明待办、完成或放弃；早于所选日期的未完成条目保留原定日期。记录沿用首页日历的数据、错误提示和分页。
- 「安排这一天」把带日期的提问添加到草稿；项目记录旁的提问按钮同样只准备问题，不自动发送或创建计划。助理回合结束后会刷新项目、计划和记录，也可手动刷新日历。
- 窄于 960px 的窗口用抽屉显示侧栏，默认收起。Escape 先关闭最上层菜单或抽屉，再关闭助理；隐藏项目及其计划不会显示，无法确认项目可见性时暂停读取日历。

只有一条对话：没有会话列表，不能新建、分支或开启探索。对话很长时沿用常规的归档式压缩，旧消息不会被静默丢弃。隐私模式隐藏的项目对助理不可见，也不能接收派活。

## 微信远程接入

科研助理页顶部的「远程接入」提供独立的微信 iLink 入口，默认关闭。目前仅支持微信：

1. 打开「远程接入」，点击「扫码绑定」，用所有者微信扫码并确认。
2. 绑定后打开「启用」开关；保持桌面 Wisp 运行且能联网。
3. 在微信中直接提问，例如「这周所有项目进展如何」「在 RNA-seq 项目里安排差异分析」。消息进入桌面科研助理的同一条长期对话，使用其模型与可见项目权限，无需用 `/project` 切换。

此处与**设置 → 远程接入**中的项目机器人分别保存绑定、开关、连接状态和消息游标。原入口的 `/project`、`/session`、`/new` 工作方式保持不变。一个机器人不能同时绑定两个入口：可使用不同机器人，或先在原入口解除绑定，再绑定到科研助理。解除助理绑定不会删除助理对话、计划，也不会关闭原项目机器人。

只处理扫码所有者的一对一文本消息（以及微信提供转写的语音）。助理入口的 `/help` 提供说明，`/status` 显示助理接入、当前模型和完全权限状态，`/stop` 停止助理当前回复；这些命令不会修改项目机器人的目标。助理不会通过 `/new` 新建第二条对话。

- `/model`：列出可用于对话的模型并标出当前模型；`/model <编号或名称>` 切换。名称不区分大小写，可以是模型配置的名称、模型 ID 或其中唯一匹配的一段；匹配到多个时请改用编号。切换只影响科研助理这条对话（与桌面助理输入框里的模型选择是同一个设置），下一轮回复起生效，正在进行的回复仍用原模型。
- `/resume`（或直接回复 `resume`）：重跑上一轮失败的请求。助理回复「处理失败」后发送即可，常与 `/model` 配合：模型或服务不可用时先切换模型，再 `/resume`。已经写入对话的请求会从失败处继续，不会重复发送；连对话都没进入的请求（例如模型没有配置密钥）会按原文重发。上一轮没有失败时只会提示，不会发给模型；助理正在回复时请等这一轮结束。

保存计划、派活等需要确认的操作会同时显示在桌面助理对话和微信中。收到「科研助理等待审批」后，直接回复以下完整文本（不区分大小写，无需斜杠或编号）：

- `yes`：仅批准助理当前操作一次，继续执行。
- `no`：拒绝助理当前操作。
- `full`：批准当前操作，并开启助理会话的完全权限，后续普通工具操作免审批；没有待审批请求时也可开启。
- `full off`：关闭完全权限，后续操作恢复审批。
- `/approval`：重新显示助理当前待审批请求。

`yes` / `no` 没有待审批请求时只会提示，不会发给模型；`yes please` 等较长文本仍作为普通对话处理。完全权限与桌面助理共用，仅限这条助理会话，重启 Wisp 后失效，不创建全局或项目的持久放行规则。显式禁止的工具仍不可用。项目审批仍属于原项目；对于助理派发的任务，助理先判断、拿不准再转给你。多个转交请求依次显示，不覆盖彼此。

关闭配置弹窗只结束本次扫码轮询，已启用的接入会继续运行，重启 Wisp 后自动恢复；登录过期后需要重新绑定并启用。

按 Escape 会先关闭远程接入弹窗，保留助理页和已打开的侧栏。

## English

**Research assistant** on the home screen opens one conversation that belongs to no project and never splits into new ones. It reports recorded activity across projects (the same records and daily recaps as the research calendar, up to 7 days per read), keeps a dated plan (open / done / dropped; unfinished items carry forward to today), lists project conversations, and dispatches work into an existing or new conversation. A requested server such as CPU2 is enabled and set as that conversation's default before dispatch. Busy conversations are rejected without changing their server. After the project agent accepts the instruction, the assistant acknowledges startup and finishes its turn. A background completion callback then uses the assistant's model to review the actual turn result and append a separate summary, including failures and unfinished work. WeChat-originated tasks also send the summary to the original binding. No assistant polling is needed. It cannot read or write files, run code or commands, or search literature — it organizes, project conversations do the work. Escape closes it; a running reply continues in the background. Projects hidden by privacy mode are invisible to it.

The assistant page has independently collapsible project and calendar sidebars around its existing conversation. Selecting a project attaches context to future messages without navigating away. The calendar separates saved plans from recorded activity, retains its date when hidden, and refreshes after an assistant turn. Planning and activity buttons append questions to the draft for review before sending. Desktop sidebar preferences persist; narrow windows use drawers, with Escape dismissing the topmost surface first.

**Remote access** in the assistant header connects WeChat directly to this same conversation and all visible projects. Scan to bind, then enable the connection; keep Wisp running. This binding, switch, credentials and cursor are independent from the project bots in Settings. Existing project slash commands keep their behavior. A bot can belong to only one entry at a time. Assistant messages use natural language; `/help`, `/status` and `/stop` stay within the assistant. `/model` lists the chat models and `/model <number or name>` switches the assistant conversation's model from its next turn, the same setting as the desktop assistant's model picker. `/resume` (or the exact reply `resume`) reruns the last failed request: a turn that started continues without a duplicate message, and a request that never reached the conversation is sent again. It only replies when nothing failed, and waits in the turn queue like a message. Reply `yes` to approve the assistant's pending operation once, `no` to reject it, or `full` to approve it and enable Full Permission for subsequent ordinary tools in the assistant conversation. These exact replies ignore case and bypass the waiting turn queue. `full` also works without a pending request; `full off` revokes it, `/approval` repeats the pending request, and `/status` shows the permission state. Full Permission is shared with the desktop assistant, resets when Wisp restarts, and does not override explicit tool denials or extend to project sessions. For assistant-dispatched work, a tool-free reviewer compares each proposed operation with the researcher’s original request and approves sufficiently clear, authorized operations once. Uncertainty, incomplete details, or review failures produce a confirmation in the assistant and the original WeChat binding; yes/no forwards to the exact original project request. Requests resolved in the project are withdrawn automatically. ACP requests without one-shot choices still need the project permission UI. Unbinding preserves conversation history and plans. Escape dismisses the connection dialog before the assistant or its drawers.

The top toolbar provides the project and calendar visibility toggles. Sidebar headings show only their titles, without duplicate collapse buttons.

## 实现说明

- 对话存放在隐藏项目 `assistant:research` 的固定会话 `research-assistant` 中，不出现在项目列表、最近会话、搜索或用量统计里，也不会被清理。
- 该会话的回合使用独立的系统提示词和 6 个工具（`research_projects`、`research_activity`、`project_conversations`、`research_plan`、`dispatch_to_project`、`project_session_result`），不加载文件/Shell 工具、Python/R 运行时、MCP、Skill 或 ACP 外部 Agent。
- 计划保存在全局表 `assistant_tasks`（迁移 `0063_assistant_tasks`，幂等），不属于任何项目，不随项目导入导出。
- 命令：`open_research_assistant`（绑定当前窗口，记住要恢复的项目）、`close_research_assistant`（恢复）；`get_research_assistant_projects` 和 `get_research_assistant_plan` 在服务端核验隐私设置后提供侧栏数据。为该项目新建会话、分支或开启探索的请求会被后端拒绝。
- 微信状态命令：`assistant_weixin_status`；绑定、启用和解除绑定复用微信命令并传 `destination: "assistant"`，缺省仍为原项目入口。助理配置使用 `assistant_weixin_*` 设置键，token 单独存在系统 keyring；助理消息不写入原 IM 共享路由，也不改变桌面当前项目。
- 微信审批复用原生确认通道和内存中的会话完全权限；`yes` / `no` 同时核验助理项目、固定会话和审批 ID，避免桌面先处理后误批下一条请求。`research_dispatch_approval.rs` 使用无工具模型调用判断项目审批；严格解析判断结果，失败默认转交用户。转交卡片复用助理确认槽位，并仅响应对应的原生或 ACP 一次性请求；取消和原请求失效时清理卡片。
- `/model` 调用与桌面 `set_active_model` 相同的 `models::set_session_model`，写入助理会话的模型并使缓存的 Agent 失效。`/resume` 依据会话最近一次回合结束事件（`Store::last_turn_outcome`，`Done` / `Error`）判断是否有失败回合，再以 `resume` 方式调用 `send_message_inner`，与桌面错误卡片的「继续执行」是同一路径；回合开始前就失败的请求没有写入对话，其原文只保存在内存中，重启 Wisp 后不能再重跑。`/resume` 会启动回合，因此走微信的回合队列而不是即时控制通道。
- `research_dispatch.rs` 管理启动确认和后台结果回传。结果在项目回合释放锁之前快照，避免后续对话覆盖；摘要通过无工具的模型调用生成，并在助理回合锁内写入消息和 UI 事件，供历史回放和后续对话使用。输出前重新检查项目隐私和微信绑定身份。

### 手动验证微信审批

1. 绑定并启用助理微信，在微信请求保存一条计划；收到审批提示后回复 `yes`，确认桌面审批卡消失且计划保存。
2. 触发另一条审批，回复 `no`，确认操作未执行；没有审批时再回复 `yes`，应提示没有待审批请求。
3. 触发审批并回复 `full`，确认当前操作继续，后续保存计划不再询问；`/status` 应显示完全权限已开启。
4. 回复 `full off`，确认下一次需要确认的操作重新请求审批。开启后重启 Wisp，也应恢复审批。
5. 在未由助理派发的项目里留下待审批请求，确认助理的 `yes`、`no`、`full` 不处理这条无关请求。

### 手动验证模型切换与重跑

1. 微信发送 `/model`，确认列表与桌面模型设置中的对话模型一致，当前模型有标记；`/status` 显示同一个模型。
2. 发送 `/model 2`（或名称的一部分），确认回复新模型；再提问一次，桌面助理这条回复的模型标记应为新模型。重新打开桌面助理，输入框的模型选择也应是新模型。
3. 把助理切到一个不可用的模型（例如接口地址填错），微信提问，应收到「处理失败」及 `/resume` 提示。`/model` 切回可用模型后发送 `/resume`，确认助理回答的是刚才的问题，且对话里这条问题只出现一次。
4. 没有失败回合时发送 `/resume`，应提示没有需要重跑的请求，不产生新的回复。
5. 助理正在回复微信消息时发送 `/resume`，它应排在当前回合之后处理，不打断回复；当前回合成功时提示没有需要重跑的请求。桌面端发起的回复进行中时，应提示稍后再发。

### 手动验证派发与回报

1. 让助理列出目标项目的对话，再指定已有对话发送任务并绑定 CPU2；确认项目对话标题和历史保留，默认执行环境变为 CPU2。
2. 确认项目开始接收指令后，助理先回复启动通知，输入框恢复可用；不出现反复调用 `project_session_result`。
3. 项目完成后，确认助理自动出现独立的结果摘要；重新打开助理仍能看到摘要。微信发起的任务应收到同一摘要。
4. 分别测试配置错误、停止任务和任务失败，确认没有误报成功或引用已有对话上一轮的结果。忙碌对话的派发不应修改服务器。
5. 关闭项目完全权限，派发一个明确范围的任务。确认助理能放行详情充分的范围内操作；对不明确的删除或缺少内容的修改，应显示转交卡片。分别通过微信 `yes` / `no` 和原项目桌面按钮处理，确认只结算原请求，陈旧卡片随即关闭。

## 限制

- 微信切换模型后，已打开的桌面助理页不会立即刷新输入框里的模型名称，重新打开助理后显示新模型；回合实际使用的模型以切换结果为准。
- `/resume` 只重跑助理自己的回合，不重跑已派发到项目里的任务；项目任务失败后请让助理重新派发，或到对应项目处理。

- 晨报仍需要你开口问；已派发工作的完成摘要会自动回报。
- 自动回报依赖 Wisp 持续运行；应用退出会中断内存中的派发和通知等待，不自动重跑任务。已保存的摘要仍在助理历史中。
- 微信受 iLink 回复窗口限制（约最后一条消息后 30 分钟）。窗口失效时摘要先保存在桌面，在同一连接中等待你再次发消息后补发；关闭接入或重新绑定会丢弃尚未发送的内存通知。摘要模型不可用时回传状态和已有结果文本。
- 自动回报以项目 Agent 的本轮结束为边界；项目另外提交到调度器的长任务是否完成，以回传结果为准。不会仅因 Agent 回复结束就把计划标为完成。
- 审批判断不是给整个项目开启完全权限。模型不可用、操作详情不全时会转交给你；ACP 若只提供持久选项而没有一次性选项，仍需在原项目的权限界面处理。
- 已保存计划显示在助理页的日历侧栏；首页独立研究日历仍展示研究活动。
