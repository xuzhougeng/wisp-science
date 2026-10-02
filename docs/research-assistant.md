# 科研助理 / Research assistant

首页右上角「科研助理」打开一条**不属于任何项目、永不新建的对话**。它像一位科研秘书：汇报你做过什么、记住你打算做什么、把具体工作交给项目里的对话去完成。它自己不写代码、不改文件、不运行命令或分析，也不检索文献。

## 能做什么

选择项目后发送问题，会读取该项目已保存的对话作为上下文；发送时重新核验隐私可见性。发送失败会保留草稿和顶部项目选择，不会额外生成重复的项目附件卡片。

- **汇报**：「我昨天做了什么？」「这周 RNA-seq 项目有什么进展？」——读取各项目已登记的研究活动（Run、产出、研究记录、会话中的请求）和每日回顾，与[研究日历](research-journey.md#首页研究日历--home-research-calendar)同源。一次最多读 7 天；只说记录里有的内容。
- **记计划**：「记住我今天要写完 Methods、催合作者的数据」——计划逐条保存，带日期和状态（待办 / 完成 / 放弃）。之前没完成的条目会自动出现在今天的列表里。计划存在对话之外，长对话压缩后也不会丢。
- **派活**：「在 RNA-seq 项目里开个对话，用 batch 作协变量重跑差异分析」——助理在该项目中新建一条带标题的会话，把完整指令发过去，由项目自己的 Agent 在后台完成；需要审批的操作仍在那个项目里等你确认。派出的工作会记入今天的计划。
- **跟进**：「刚才那个任务跑完没？」——查看派出会话是否仍在运行，以及它的最终回答。

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

只处理扫码所有者的一对一文本消息（以及微信提供转写的语音）。助理入口的 `/help` 提供说明，`/status` 显示助理接入，`/stop` 停止助理当前回复；这些命令不会修改项目机器人的目标。助理不会通过 `/new` 新建第二条对话。

微信请求保留远程操作的审批约束：保存计划、派活等需确认的助理操作在桌面助理对话中审批，派发后的任务在对应项目中审批。助理入口暂不提供文本审批；派发完成后可继续问助理查看结果。关闭配置弹窗只结束本次扫码轮询，已启用的接入会继续运行，重启 Wisp 后自动恢复；登录过期后需要重新绑定并启用。

按 Escape 会先关闭远程接入弹窗，保留助理页和已打开的侧栏。

## English

**Research assistant** on the home screen opens one conversation that belongs to no project and never splits into new ones. It reports recorded activity across projects (the same records and daily recaps as the research calendar, up to 7 days per read), keeps a dated plan (open / done / dropped; unfinished items carry forward to today), dispatches work by starting a titled conversation in a project and sending it a self-contained instruction, and checks a dispatched conversation's status and final answer. It cannot read or write files, run code or commands, or search literature — it organizes, project conversations do the work. Escape closes it; a running reply continues in the background. Projects hidden by privacy mode are invisible to it.

The assistant page has independently collapsible project and calendar sidebars around its existing conversation. Selecting a project attaches context to future messages without navigating away. The calendar separates saved plans from recorded activity, retains its date when hidden, and refreshes after an assistant turn. Planning and activity buttons append questions to the draft for review before sending. Desktop sidebar preferences persist; narrow windows use drawers, with Escape dismissing the topmost surface first.

**Remote access** in the assistant header connects WeChat directly to this same conversation and all visible projects. Scan to bind, then enable the connection; keep Wisp running. This binding, switch, credentials and cursor are independent from the project bots in Settings. Existing project slash commands keep their behavior. A bot can belong to only one entry at a time. Assistant messages use natural language; `/help`, `/status` and `/stop` stay within the assistant. IM approval restrictions remain in force; handle assistant approvals on the desktop assistant and dispatched-work approvals in the relevant desktop project. Unbinding preserves conversation history and plans. Escape dismisses the connection dialog before the assistant or its drawers.

The top toolbar provides the project and calendar visibility toggles. Sidebar headings show only their titles, without duplicate collapse buttons.

## 实现说明

- 对话存放在隐藏项目 `assistant:research` 的固定会话 `research-assistant` 中，不出现在项目列表、最近会话、搜索或用量统计里，也不会被清理。
- 该会话的回合使用独立的系统提示词和 5 个工具（`research_projects`、`research_activity`、`research_plan`、`dispatch_to_project`、`project_session_result`），不加载文件/Shell 工具、Python/R 运行时、MCP、Skill 或 ACP 外部 Agent。
- 计划保存在全局表 `assistant_tasks`（迁移 `0063_assistant_tasks`，幂等），不属于任何项目，不随项目导入导出。
- 命令：`open_research_assistant`（绑定当前窗口，记住要恢复的项目）、`close_research_assistant`（恢复）；`get_research_assistant_projects` 和 `get_research_assistant_plan` 在服务端核验隐私设置后提供侧栏数据。为该项目新建会话、分支或开启探索的请求会被后端拒绝。
- 微信状态命令：`assistant_weixin_status`；绑定、启用和解除绑定复用微信命令并传 `destination: "assistant"`，缺省仍为原项目入口。助理配置使用 `assistant_weixin_*` 设置键，token 单独存在系统 keyring；助理消息不写入原 IM 共享路由，也不改变桌面当前项目。

## 限制

- 不会主动推送：晨报需要你开口问。
- 派出的会话完成后不会自动回报，需要问助理或打开该项目查看。
- 已保存计划显示在助理页的日历侧栏；首页独立研究日历仍展示研究活动。
