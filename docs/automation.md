# 自动化 / Automation

自动化不属于某一个项目，入口在科研助理页面：顶栏「远程接入」左侧的「自动化」按钮。按计划运行任务，或在需要时随时执行。点击「返回科研助理」或按 Escape 关闭；打开着「创建定时任务」表单时，Escape 先关闭表单。从这里打开的设置等上层界面会先消耗 Escape。

## 内置：每日研究回顾

默认开启。每天到设定时间（默认 09:00，可在卡片上修改）后运行一次：为每个项目最近 3 个完整日中有记录、尚无回顾的日子起草 [每日研究回顾](research-journey.md#每日研究回顾--daily-recap)。错过的时间（例如电脑休眠）会在之后打开 Wisp 时补跑；已有或已忽略的回顾不会被覆盖，没有记录的日子不调用模型。一次运行累计 3 个错误后停止，卡片显示首个错误，第二天重试。

- 开关和时间立即保存。「立即运行」不受时间限制，不改变第二天的计划。
- 模型在 **设置 → 专家 → Recap** 中更换；卡片上的链接直接打开该页。
- 草稿只出现在研究历程和研究日历中，确认之前不算正式记录。

## 定时任务

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
- **Scheduled tasks**: send a prompt into a new session of a chosen project daily,
  weekly or every few hours. Pause, run now or delete each task. Templates prefill
  a literature watch, a run check and a weekly report. Tasks run only while Wisp is
  open; missed slots collapse into one run after the next launch.

Commands: `get_daily_recap_automation`, `set_daily_recap_automation(enabled, time)`,
`run_daily_recap_now`, `list_all_schedules`, and `create_schedule` with an explicit
`projectId`, alongside the existing `set_schedule_enabled`, `run_schedule_now`
and `delete_schedule`. Settings live in the `automation_daily_recap` row.

Manual check: create a task from the weekly-report template for a second project,
confirm the cadence and next run, run it now and open the new session in that
project; pause and delete it. Turn the daily recap off, refresh, and confirm it
stays off; set its time a minute ahead with a project that has activity
yesterday and confirm a draft appears in that day's research journey.
