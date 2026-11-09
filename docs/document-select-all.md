# Select all in an open document

**Ctrl+A** on Windows/Linux or **Command+A** on macOS selects the text of the
document you are reading instead of the whole window. With a file open in the
center (for example `METHODS.md`), the shortcut selects only the rendered
document: not the sidebar, tab strip, path line or toolbar buttons. Copy then
gives just the document.

What gets selected depends on the preview:

- Markdown, text, PDF pages and Word documents: the rendered document body.
- Code and JSON previews: the source only, without line numbers.
- Notebooks: every cell. CSV and other tables: the table.
- R and Python files in the editor: the whole source, ready to type over.
- Images: nothing; the shortcut no longer tints the whole window.

The shortcut follows where you last clicked or moved focus. The same applies
to file previews in the right pane and in the artifact viewer. When the
document is split beside the conversation, click the document first; clicking
the conversation keeps the browser's normal behavior. Text fields (the
composer, search boxes, the editor once focused) keep their own select-all. A
dialog over the document gets the shortcut instead of the document behind it.

中文：在中间打开文件（例如 `METHODS.md`）后，按 **Ctrl+A**（macOS 为 **⌘A**）只选中
文档正文，不再连同侧栏、标签、路径和工具栏一起选中，复制得到的就是文档内容。
代码预览只选源码，不带行号；R/Python 编辑器选中全部源码；图片不选任何内容。
快捷键作用于最近点击或聚焦的区域；文档与对话并排时，先点击文档。输入框保留各自的全选。

Manual checks: open a Markdown file in the center and press Ctrl/⌘+A without
clicking it; copy and paste, and confirm only the document text arrives. Open a
shell script and confirm the selection has no line numbers. Open an R script
and confirm the editor is focused with all source selected. Split the document
beside the conversation, click the conversation, press Ctrl/⌘+A and confirm
the document is not singled out; click the document and press it again. Open
Settings over a document and confirm the shortcut does not select the
document behind it.
