# 科研助理 / Research assistant

首页右上角「科研助理」打开一条**不属于任何项目、永不新建的对话**。它像一位科研秘书：汇报你做过什么、记住你打算做什么、把具体工作交给项目里的对话去完成。它自己不写代码、不改文件、不运行命令或分析，也不检索文献。

## 能做什么

- **汇报**：「我昨天做了什么？」「这周 RNA-seq 项目有什么进展？」——读取各项目已登记的研究活动（Run、产出、研究记录、会话中的请求）和每日回顾，与[研究日历](research-journey.md#首页研究日历--home-research-calendar)同源。一次最多读 7 天；只说记录里有的内容。
- **记计划**：「记住我今天要写完 Methods、催合作者的数据」——计划逐条保存，带日期和状态（待办 / 完成 / 放弃）。之前没完成的条目会自动出现在今天的列表里。计划存在对话之外，长对话压缩后也不会丢。
- **派活**：「在 RNA-seq 项目里开个对话，用 batch 作协变量重跑差异分析」——助理在该项目中新建一条带标题的会话，把完整指令发过去，由项目自己的 Agent 在后台完成；需要审批的操作仍在那个项目里等你确认。派出的工作会记入今天的计划。
- **跟进**：「刚才那个任务跑完没？」——查看派出会话是否仍在运行，以及它的最终回答。

## 使用

1. 首页点击「科研助理」。打开后显示之前的全部对话（按页加载），并聚焦输入框。
2. 按 Escape 或右上角关闭按钮回到首页；助理正在进行的回复会在后台继续。
3. 每次发送时，助理会获知当前的本地日期和时间（不写入对话记录），据此理解「今天」「昨天」。
4. 模型使用该对话在输入框中选择的模型。

只有一条对话：没有会话列表，不能新建、分支或开启探索。对话很长时沿用常规的归档式压缩，旧消息不会被静默丢弃。隐私模式隐藏的项目对助理不可见，也不能接收派活。

## English

**Research assistant** on the home screen opens one conversation that belongs to no project and never splits into new ones. It reports recorded activity across projects (the same records and daily recaps as the research calendar, up to 7 days per read), keeps a dated plan (open / done / dropped; unfinished items carry forward to today), dispatches work by starting a titled conversation in a project and sending it a self-contained instruction, and checks a dispatched conversation's status and final answer. It cannot read or write files, run code or commands, or search literature — it organizes, project conversations do the work. Escape closes it; a running reply continues in the background. Projects hidden by privacy mode are invisible to it.

## 实现说明

- 对话存放在隐藏项目 `assistant:research` 的固定会话 `research-assistant` 中，与随手一聊一样不出现在项目列表、最近会话、搜索或用量统计里，但不会被清理。
- 该会话的回合使用独立的系统提示词和 5 个工具（`research_projects`、`research_activity`、`research_plan`、`dispatch_to_project`、`project_session_result`），不加载文件/Shell 工具、Python/R 运行时、MCP、Skill 或 ACP 外部 Agent。
- 计划保存在全局表 `assistant_tasks`（迁移 `0063_assistant_tasks`，幂等），不属于任何项目，不随项目导入导出。
- 命令：`open_research_assistant`（绑定当前窗口，记住要恢复的项目）、`close_research_assistant`（恢复）。为该项目新建会话、分支或开启探索的请求会被后端拒绝。

## 限制

- 不会主动推送：晨报需要你开口问。
- 派出的会话完成后不会自动回报，需要问助理或打开该项目查看。
- 计划暂不显示在研究日历上。
