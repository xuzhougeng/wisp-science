# Find in a conversation

## Search all conversations in a project

Open **Search** in the sidebar (or **Ctrl+K / Command+K**) and choose
**Search this project's chat history**. **Ctrl+Shift+F** on Windows/Linux or
**Command+Shift+F** on macOS opens it directly while a project is open.

Enter a keyword or phrase to search saved questions, assistant replies and tool
output across the current project's conversations, including shelved chats,
branches, unloaded older messages and original messages retained after context
compaction. Other projects, exploration workspaces, internal execution frames,
system prompts, reasoning, attachment bytes and tool-call arguments are excluded.
This searches saved message text; it does not search files or use a model/API.

Matching is literal: `%`, `_`, quotes and backslashes have no special meaning.
English letters ignore case; Chinese and other characters match their original
text. Leading/trailing whitespace is ignored. Blank queries show no history;
keywords are limited to 256 characters. Results show one excerpt per matching
message, highlighting its first match, with the conversation title, author and
time. The newest 50 matches appear first; **Load more matches** continues through
the remaining history. Select a result (or use arrows and Enter) to open its
conversation and jump to the corresponding question/response turn. Search
failures offer **Retry search**. Escape closes the topmost open surface first.
If a legacy record has an ambiguous turn location, the conversation opens with
a notice so its outline can be used. While that conversation is generating a
reply, jumping to an unloaded older turn waits until you reopen the result after
the reply finishes.

中文：点击侧栏“搜索”后选择“搜索当前项目的聊天记录”，或直接按
**Ctrl/⌘+Shift+F**。输入关键词即可检索当前项目各会话中已保存的提问、回复和
工具输出，包括已收起会话、未加载的旧消息及压缩前保留的原始历史。搜索结果
显示会话名、时间和关键词高亮片段；点击结果跳转到对应的对话轮次。“加载更多匹配”
可继续查看。支持中文和短语，英文不区分大小写，`%`、`_` 按普通字符查找。

Manual checks: search a keyword found only in an old reply in another conversation
of the same project, then a shelved conversation; open both results. Search `%_`
and a Chinese phrase. With more than 50 hits, load another page. Open chat search
above Shelved conversations, then open the action palette and press Escape
immediately: only the palette should close. Repeat Escape to close chat search
while keeping the shelved collection open.

## Find in the displayed conversation

Press **Ctrl+F** on Windows/Linux or **Command+F** on macOS while viewing the
conversation to search its displayed message bodies. The count and orange
active highlight use the same ordered list of matches; yellow highlights show
the other matches. Sidebar labels, session titles, composer text and hidden
content are excluded. Matching is literal and case-insensitive, including text
split by inline Markdown formatting.

**Enter** / **Shift+Enter** and the next/previous arrows move through matches,
wrapping at either end. Navigation pauses automatic following of new output,
so a result reached from the bottom remains visible. **Back to latest** resumes
following. **Escape** closes the find bar after any overlaid dialog or menu;
closing find preserves the reading position. Changing sessions closes find.

Search covers the currently displayed transcript window. Load older messages
to search that window; the count refreshes when displayed content changes.
Collapsed content is searchable after expanding it. Editors and terminals keep
their own find shortcuts when focused.
Older WebViews without the CSS Highlight API retain their native find UI.

## Finding older questions after compaction

The conversation outline lists the saved visual history, including questions
that are no longer part of the model's compacted context. Its count does not
depend on which message page is currently displayed. Context summary
checkpoints are not counted as questions.

Choose a question in the outline to load and scroll to it. When an old message
cursor cannot be used safely after compaction, Wisp walks saved history pages
until it finds that question. Loading failures are shown and the selection can
be retried. Scrolling upward also loads earlier pages. This restores access to
persisted history; it cannot reconstruct events that were never saved.
