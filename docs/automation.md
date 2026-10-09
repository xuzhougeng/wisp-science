# 自动化 / Automation

自动化不属于某一个项目，入口在科研助理页面：顶栏「远程接入」左侧的「自动化」按钮。按计划运行任务，或在需要时随时执行。点击「返回科研助理」或按 Escape 关闭；打开着「创建定时任务」表单时，Escape 先关闭表单。从这里打开的设置等上层界面会先消耗 Escape。

助理完成一轮对话后刷新项目列表时，任务表单保留上次已加载的可见项目、未提交内容和输入焦点；即使这时才首次打开自动化，也不会短暂出现空项目列表。隐私模式隐藏的项目仍会从候选项移除。

## 内置：每日研究回顾

默认开启。每天到设定时间（默认 09:00，可在卡片上修改）后运行一次：为每个项目最近 3 个完整日中有记录、尚无回顾的日子起草 [每日研究回顾](research-journey.md#每日研究回顾--daily-recap)。错过的时间（例如电脑休眠）会在之后打开 Wisp 时补跑；已有或已忽略的回顾不会被覆盖，没有记录的日子不调用模型。一次运行累计 3 个错误后停止，卡片显示首个错误，第二天重试。

- 开关和时间立即保存。「立即运行」不受时间限制，不改变第二天的计划。
- 模型在 **设置 → 专家 → Recap** 中更换；卡片上的链接直接打开该页。
- 草稿只出现在研究历程和研究日历中，确认之前不算正式记录。

## 研究职责

把一份长期职责（目标、KPI、工作约束）交给项目里的 Agent，由它一轮轮推进。入口同在本页的「研究职责」区块，详见 [研究职责](research-mandates.md)。

## 定时任务

### 当前会话的 timer

在普通 Wisp 对话中输入 `/timer 1h 当前进展如何`，会在一小时后开始，每小时在该会话执行一次提示词。支持正整数 `m`（分钟）、`h`（小时）、`d`（天），范围为 1 分钟至 365 天。每个会话只有一个 timer，重复输入命令会修改间隔和提示词；暂停状态会保留。单独输入 `/timer` 打开管理面板。

输入框旁的时钟显示频率、下次执行时间或暂停状态，点击可修改、暂停/恢复或取消。取消只停止后续触发，当前执行和最新结果保留；停止当前执行仍使用聊天的停止按钮。会话忙时等待空闲，错过的多个时间点合并为一次检查。仅在 Wisp 运行时触发，配置随会话持久保存。

下一次检查开始前，只移除 timer 上一轮的提问、工具调用、结果和回答，普通消息保留。模型上下文也移除旧轮次；若其后发生过压缩，会恢复检查前的上下文并保留后续普通轮次，让下次正常压缩重新处理。此操作不撤销文件修改或计算任务。删除会话会同时删除 timer。ACP 外部代理维护自己的远程历史，暂不支持这种轮次替换；只读的子代理会话也不能设置 timer。

### 项目级任务

定时任务在指定项目中新建一个会话，发送提示词，像普通对话一样运行，结果在该项目的会话列表中查看。列表显示每个项目的任务、频率、下次运行时间；可以暂停/恢复、「立即运行」（不改变计划）和删除（再次点击确认）。隐私模式隐藏的项目，其任务也不显示。

- 「创建定时任务」选择项目、名称、频率（每天 / 每周某天 / 每隔 N 小时）、时间和提示词。名称留空时使用提示词首行。
- 模板预填常见任务：文献追踪（每周一 09:00）、运行巡检（每天 18:00）、研究周报（每周五 17:00）。提交前可以修改任何字段。
- 任务只在 Wisp 运行时触发，没有系统级后台服务；关闭期间错过的多个时间点合并为下次启动后的一次补跑。
- 每天/每周任务按首次运行时间固定间隔重复；跨夏令时切换后，墙钟时间可能偏移一小时。

## Automation (English)

Automation spans every project, so it opens from the research assistant: the
**Automation** button in its header, next to Remote access.

- **Daily research recap** (built-in, on by default): once a day after its time
  (09:00 by default), drafts a recap for each project's last three complete days
  that have recorded activity and no recap yet. Missed mornings catch up on the
  next launch; existing or dismissed recaps are never overwritten; quiet days make
  no model call. Change the model in Settings → Specialists → Recap.
- **Research mandates**: hand a project agent a long-running responsibility — a
  goal, KPIs and constraints — that it carries round by round. See
  [Research mandates](research-mandates.md).
- **Scheduled tasks**: send a prompt into a new session of a chosen project daily,
  weekly or every few hours. Pause, run now or delete each task. Templates prefill
  a literature watch, a run check and a weekly report. Tasks run only while Wisp is
  open; missed slots collapse into one run after the next launch.
- **Conversation timers**: `/timer 1h Check progress` repeats in the current native
  Wisp conversation. Use positive integer minutes (`m`), hours (`h`) or days (`d`),
  from one minute to 365 days. Each conversation has one timer; repeat the command
  to edit it, or use the clock to edit, pause/resume or cancel. A bare `/timer`
  opens its panel. Busy conversations defer and coalesce missed checks. Each fire
  removes the preceding timer turn from both the transcript and model context,
  preserving human turns and files. Compacted context containing the old result
  is rebuilt from the pre-timer epoch plus subsequent ordinary turns. Settings
  survive restart; deleting the conversation deletes its timer. ACP conversations and
  watch-only subagent conversations are unsupported.

Commands: `get_daily_recap_automation`, `set_daily_recap_automation(enabled, time)`,
`run_daily_recap_now`, `list_all_schedules`, and `create_schedule` with an explicit
`projectId`, alongside the existing `set_schedule_enabled`, `run_schedule_now`
and `delete_schedule`. Settings live in the `automation_daily_recap` row.

Manual check: create a task from the weekly-report template for a second project,
confirm the cadence and next run, run it now and open the new session in that
project; pause and delete it. Turn the daily recap off, refresh, and confirm it
stays off; set its time a minute ahead with a project that has activity
yesterday and confirm a draft appears in that day's research journey.
