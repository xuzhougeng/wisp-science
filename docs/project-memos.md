# 项目备忘录 / Project memos

项目备忘录是研究者在一个项目里只给自己看的笔记：实验里踩过的坑、常用的开场 prompt、某台服务器的习惯写法。它和[全局记忆](basic-configuration.md)、研究历程的人工记录都不同：**Agent 永远不会读到备忘录**，除非你自己把它粘进输入框并发送。

Project memos are researcher-only notes inside a project. Unlike global memory
or research journal entries, the agent never sees a memo: it is not injected
into any turn, no tool lists or reads it, it does not enter the research
journey, daily recaps or the home calendar, and conversation exports do not
include it. A memo reaches the model only when you paste it into the composer
and send that message yourself.

## 隔离保证 / Isolation

- 只存在应用数据库的 `project_memos` 表里，不写入工作目录，所以 `read`、`grep`、`shell` 等文件工具看不到。
- 组装一轮对话时只注入项目指令和全局记忆，不读取备忘录。
- 没有任何内置工具或 MCP 工具暴露备忘录。
- 研究历程、每日回顾、首页研究日历的投影不读取这张表。
- 会话导出只包含对话本身。
- 插入输入框并发送后，这段文字就成为普通用户消息，之后和任何输入一样进入对话历史。

## 范围与持久化 / Scope and persistence

- 每个项目一份列表，主线和所有探索分支共享同一份。备忘录不是研究状态，新增或修改不会推进项目状态修订，也不参与检查点、采用或丢弃。
- 标题可留空（界面用正文首行代替），最长 200 字符；正文必填，最长 20,000 字符。可置顶；列表按置顶优先、最近更新在前排序。
- 删除是硬删除，已删除的 ID 不会复用。
- 随项目包导出和导入；旧版本导出的项目包没有这张表，导入时得到空列表。只有表里有记录时才计入项目包指纹，升级本身不改变已发布项目的指纹。
- 迁移 `0067_project_memos` 幂等。跨边界类型为 `wisp-dto::ProjectMemo`、`ProjectMemoInput`、`ProjectMemoPatch`。

## 命令 / Commands

`list_project_memos`、`add_project_memo(input)`、`update_project_memo(patch)`、`delete_project_memo(memoId)`。四个命令都针对当前窗口的工作项目，前端不传项目 ID，因此隐私模式下隐藏的项目无法被按 ID 读取。`patch` 只需带要修改的字段，未提供的字段保持不变。

## 使用 / Using the pane

1. 打开项目，点击左侧「备忘录」。备忘录作为右侧面板的一个标签打开（也可以从右侧面板的「添加面板」菜单打开），聊天和输入框保持可见；窄窗口下右侧面板变成抽屉。面板顶部常驻「仅本机 · AI 不可见」。
2. 「新建备忘录」在列表顶部就地打开编辑器：标题可选，正文必填；`⌘/Ctrl+Enter` 保存，Escape 取消。保存失败时编辑器和已输入内容保留，并在卡片内显示错误。未保存的草稿属于当前项目：关闭标签再打开会原样恢复，切换到其他项目时丢弃。
3. 卡片显示标题（空标题用正文首行，灰色斜体表示未命名）、两行预览和相对时间；置顶的卡片排最前并带图钉。点击标题或正文展开全文，双击或点「编辑」进入编辑；「更多」里有置顶/取消置顶、复制、删除。删除立即生效，不可恢复。
4. 「插入输入框」只把正文放进输入框，不会发送：输入框为空时直接填入，否则追加在现有草稿后面并空一行；随后输入框获得焦点，面板保持打开，按钮短暂显示「已插入」。备忘录标题不会随正文插入。
5. 列表非空时顶部出现搜索框，按标题和正文过滤当前项目的备忘录。
6. 在输入框里输入 `/memo`（从 `/` 菜单选中，或直接键入 `/memo ` 再加空格），`/` 菜单变成备忘录选择器：列出标题（空标题用正文首行）和下一行预览，空格后继续输入即按标题和正文过滤，Enter 把所选正文放进输入框（规则同「插入输入框」：空则填入，否则追加在 `/memo` 之前的草稿后面并空一行），不会发送。Escape 只关闭选择器并删掉 `/memo …` 这一段，之前的草稿保留。项目还没有备忘录时，选择器只有一行「这个项目还没有备忘录 · 打开备忘录」，Enter 打开右侧备忘录面板。直接把 `/memo` 作为消息发送也只会打开选择器，不会到模型。

Escape 每次只关闭最上层：卡片的「更多」菜单 → 「放弃修改？」确认 → 正在编辑的卡片（有改动时先确认）。面板内没有打开的层时，Escape 交给应用：在抽屉宽度（≤960px）下关闭右侧面板，宽窗口下右侧面板是停靠面板，用标签上的关闭按钮关闭。

Open a project and choose Memos in the sidebar: the pane opens as a right-pane
tab next to the conversation. New memos are edited in place (title optional,
body required, ⌘/Ctrl+Enter saves); a failed save keeps the editor and an
unsaved draft survives closing the tab until you discard it or switch project.
Insert drops the body into the composer without sending, appending after a
blank line when a draft is already there. Escape closes one layer at a time:
card menu, discard prompt, editor, then (at drawer widths) the pane. Typing
`/memo ` in the composer turns the `/` menu into a memo picker: the text after
the space filters, Enter pastes the body, Escape removes the token and keeps
the draft; with no memos the only row opens the Memos pane.

## 当前状态 / Status

存储层、命令、右侧面板 UI 和 `/memo` 选择器均已实现，见设计稿 [docs/superpowers/specs/2026-10-15-project-memos-design.md](superpowers/specs/2026-10-15-project-memos-design.md)。面板采用现有右侧面板标签而不是独立抽屉，宽度随右侧面板设置。
