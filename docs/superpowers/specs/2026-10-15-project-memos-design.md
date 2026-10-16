# 项目备忘录 / Project memos — 设计稿

状态：第一步（迁移 `0067_project_memos`、store、DTO、Tauri 命令、测试）和第二步（右侧面板 UI、插入输入框、Escape 栈、Playwright 测试）已实现，见 [docs/project-memos.md](../../project-memos.md)。实现偏差：备忘录作为现有右侧面板的 `RightTab::Memos` 标签，而不是 §6.2 的独立 360px 抽屉，宽度与窄窗口行为沿用右侧面板；未保存草稿由应用持有、关闭标签后保留，取代 §6.4 的关闭确认。第三步 `/memo`（§6.6）已实现：选中命令后输入框填入 `/memo ` 并以该 token 保持选择器打开，空格后的文字即过滤词，Escape 删除 token 并保留之前的草稿；没有备忘录时唯一一行打开备忘录面板。日期：2026-10-15。

## 1. 问题

研究者在一个项目里需要随手记一些"只给自己看"的东西：实验里踩过的坑、给 AI 的常用开场白、某个服务器的习惯写法、下次想问模型的一组问题。这些内容有两个要求：

1. **AI 默认看不见**。它不进 system prompt，不进 `<global_memory>`，不进研究历程，也不能被 `read` / `grep` / `shell` 工具扫到。
2. **随时能变成 prompt**。研究者可以一键把某条备忘填进 composer，再编辑后发送；发出去的那一刻才是 AI 第一次见到它。

现有的三个"记东西"的功能都不满足这两点：

| 现有功能 | 为什么不合适 |
| --- | --- |
| 全局记忆 `global_memories` | 每轮注入 `<global_memory>`，AI 一定看见；而且是全局的，不按项目。 |
| 研究历程人工记录 `research_journal_entries` | 是研究事实的流水（进展/发现/决策），进入每日回顾的输入，AI 会读；语义上也不是"备忘"。 |
| 工作目录里的 markdown 文件 | 在 `workspace_dir` 下，`read`/`grep`/`shell` 工具都能碰到，等于公开给 AI。 |

所以需要一个新的、与 AI 完全隔离的项目级私有存储，加一个"发送到 composer"的桥。

## 2. 目标与非目标

目标（第一版）：

- 每个项目一份备忘录列表，纯文本（Markdown 语法只做轻量渲染，不做富文本）。
- 增、改、删、置顶、搜索；按更新时间排序。
- 一键 **插入 composer**（替换或追加到当前草稿），不自动发送。
- 可选：输入 `/memo` 在 `/` 菜单里直接挑一条插入。
- 随项目包导出/导入，纳入同步指纹。
- 隐私模式下隐藏的项目，其备忘录同样不可见。
- 探索分支（exploration）与主线共用同一份备忘录：备忘录是研究者的私有工具，不是研究状态，不参与检查点/采用/丢弃。

非目标：

- 不做富文本、附件、图片。
- 不做 AI 自动整理、不做"让 AI 读备忘录"的工具（这正是它和记忆的分界线）。
- 不做跨项目备忘录（全局的用全局记忆或 Library）。
- 不做多人协作。

## 3. 隔离边界：怎样保证 AI 看不见

这是本功能唯一需要刻意设计的点，列成可测试的约束：

1. **只存 SQLite**，表 `project_memos`，不落 `workspace_dir`，不写任何文件。文件工具天然看不到。
2. **不进上下文装配**。`agent_turn.rs` 组装一轮时只注入 `global_memory_runtime_injection` 与项目指令；不新增任何对 `project_memos` 的读取。测试：构造一轮 send_message，断言 prompt 里不含备忘录内容。
3. **不暴露工具**。`wisp-tools` / MCP 不注册 `list_memos`、`read_memo` 一类工具；`get_project_memos` 只是 Tauri 命令，供 UI 调用。
4. **不进研究历程、每日回顾、首页日历**。`research_journey()` 的投影不 JOIN 这张表。
5. **不进会话导出**。`export_session` 只导出 frames/messages，备忘录不是会话的一部分。
6. **发送即公开**。插入 composer 后文本成为普通用户消息，之后它就和任何用户输入一样被 AI 看到并留在 messages 里。UI 在插入时不需要额外提醒，但文档里说明这一点。

## 4. 数据模型

迁移 `crates/wisp-store/migrations/00NN_project_memos.sql`（幂等，沿用 `IF NOT EXISTS` 风格）：

```sql
CREATE TABLE IF NOT EXISTS project_memos (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,                  -- 可空串；空时 UI 用正文首行代替
    body        TEXT NOT NULL CHECK(length(trim(body)) > 0),
    pinned      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_project_memos_project_updated
    ON project_memos(project_id, pinned DESC, updated_at DESC, id DESC);
```

说明：

- 不加 `exploration_id`。备忘录对主线和所有探索分支可见、共享，见 §2。
- 不加 `source_frame_id`。第一版不记"从哪个会话记下来的"；要加时按 `global_memories.source_frame_id` 的样式加 `REFERENCES frames(id) ON DELETE SET NULL`，向后兼容。
- 上限：标题 ≤ 200 字符，正文 ≤ 20,000 字符（比研究记录的 10,000 宽，因为备忘录会放整段 prompt）。
- 删除即硬删除。备忘录没有审计价值，不做软删除；但 ID 用 `uuid v4`，和 Specialist/模型 profile 一样**不重用已删 ID**。

### DTO（`crates/wisp-dto`）

```rust
pub struct ProjectMemo {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct ProjectMemoInput {        // 新建
    pub title: String,
    pub body: String,
}

pub struct ProjectMemoPatch {        // 修改；None 表示不改
    pub id: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub pinned: Option<bool>,
}
```

`ui/src/dto.rs` 直接 re-export；`src-tauri/src/dto_contract_tests.rs` 加一条反序列化样例。

## 5. 后端

### 存储层 `crates/wisp-store/src/project_memos.rs`

```rust
impl Store {
    pub async fn list_project_memos(&self, project_id: &str) -> Result<Vec<ProjectMemo>>;
    pub async fn add_project_memo(&self, project_id: &str, input: &ProjectMemoInput) -> Result<ProjectMemo>;
    pub async fn update_project_memo(&self, project_id: &str, patch: &ProjectMemoPatch) -> Result<ProjectMemo>;
    pub async fn delete_project_memo(&self, project_id: &str, memo_id: &str) -> Result<()>;
}
```

- 都走 `route_project(project_id)`，兼容项目自带数据库（project-owned storage）。
- `update` / `delete` 用 `WHERE id=? AND project_id=?`，跨项目 ID 不会误改。
- 写操作走 `begin_write()`。**不调用** `bump_state_generation_in_tx`：备忘录不是研究状态，不应触发项目状态修订和分支快照。
- `project_transfer.rs`：导出/导入 `project_memos`（按 `project_id` 过滤）；清理函数加 `DELETE FROM project_memos WHERE project_id=?`；指纹表加 `("project_memos", "*", "id")`。旧项目包无此表时导入照常，沿用 `attached_table_exists` 守卫。

### Tauri 命令 `src-tauri/src/project_commands.rs`

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `list_project_memos` | 当前 ActiveProject | `Vec<ProjectMemo>` |
| `add_project_memo` | `ProjectMemoInput` | `ProjectMemo` |
| `update_project_memo` | `ProjectMemoPatch` | `ProjectMemo` |
| `delete_project_memo` | `memo_id` | `()` |

全部用 `AppState` 中的当前项目 ID，不让前端传 `project_id`，避免隐私模式下读到隐藏项目。在 `lib.rs` 的 `generate_handler!` 注册。

## 6. UI 设计

### 6.1 入口

侧栏项目导航 `nav_entries`（`ui/src/sidebar.rs`）在「研究历程」下方加一项：

```
("notebook", "sidebar.memos", "nav-tier-1", memos_open.into(), open_memos)
```

图标用现成的 `notebook`（已在 `compose_icon` 集合里，侧栏里尚未用过）。i18n 文案：`备忘录` / `Memos`。

### 6.2 面板形态：右侧抽屉，不是全屏视图

研究历程是全屏工作区，因为它要替代聊天。备忘录不同：它的典型用法是 **一边看聊天，一边翻备忘录，把一条塞进 composer**。所以做成贴着主区右侧的抽屉（宽 360 px，可拖到 280–520 px），聊天和 composer 保持可见、可操作。

```
┌─────────┬──────────────────────────────┬──────────────────┐
│ sidebar │  chat                        │ 备忘录      搜索 ✕│
│         │                              ├──────────────────┤
│ 研究历程 │                              │ 📌 服务器连接习惯 │
│ ▸备忘录  │                              │   ssh gpu02 ...  │
│ 文献    │                              │   2 天前    [插入]│
│         │                              ├──────────────────┤
│         │                              │ 常用分析开场白    │
│         │                              │   你是……请先……   │
│         │                              │   上周     [插入]│
│         │  ┌────────────────────────┐  ├──────────────────┤
│         │  │ composer               │  │   + 新建备忘录    │
│         │  └────────────────────────┘  └──────────────────┘
```

窄窗口（主区 < 900 px）时抽屉改为覆盖在聊天上方的浮层，仍不全屏。

### 6.3 列表项

每条卡片：

- 标题（空标题时取正文首行，灰色斜体表示"未命名"）。
- 正文前两行预览，`white-space: pre-wrap`，不渲染 Markdown。
- 相对时间（复用会话列表的时间格式）。
- 置顶的卡片排最前，带 `pin` 图标。
- 悬停/聚焦出现操作：**插入**（`arrow-left`，主操作）、**编辑**（`square-pen`）、**更多**（`more` → 置顶/取消置顶、复制、删除）。

点击卡片正文 = 展开到完整正文（卡片内就地展开），不进编辑；双击或点「编辑」才进编辑。

### 6.4 新建 / 编辑

卡片就地变成编辑器，不开弹窗：标题单行输入 + 正文 `textarea`（自动高度，最低 6 行），底部「保存」「取消」。

- `⌘/Ctrl+Enter` 保存，Escape 取消（见 §6.7）。
- 保存后乐观更新列表，失败回滚并在卡片内显示错误。
- 关闭抽屉时若有未保存编辑，先问「放弃修改？」（复用现有 confirm modal）。

### 6.5 插入 composer

「插入」按钮行为：

| composer 状态 | 行为 |
| --- | --- |
| 空 | 直接填入正文，光标到末尾 |
| 有草稿 | 追加到草稿末尾，中间空一行 |
| 正在生成回复 | 仍可追加，排队语义与手动输入一致 |

- 只插 `body`，不插标题。标题是研究者给自己的索引。
- 经 `sanitize_composer_text` 过滤后写入 `input` 信号；不触发发送。
- 插入后 composer 获得焦点，抽屉**不关闭**（用户常连续插两条）。
- 卡片短暂显示「已插入」反馈（1.5 s）。

### 6.6 `/memo` 斜杠命令

`SLASH_COMMANDS` 加 `"memo"`，图标 `notebook`，`slash_command_fills_text` 返回 `false`（选中即动作）。选中后 `/` 菜单切换为备忘录选择器：列出标题 + 首行，输入过滤，Enter 插入所选正文，Escape 回到普通输入。没有备忘录时显示「这个项目还没有备忘录」+「打开备忘录」。

这是可选项；第一版可以只做侧栏抽屉，`/memo` 作为后续 PR。

### 6.7 Escape 栈

按 AGENTS.md 规则，所有层参与窗口级 Escape 栈，顺序从顶到底：

1. 卡片「更多」菜单（组件内 `window_capture_escape`，只关菜单）。
2. 正在编辑的卡片（取消编辑，有改动时先弹确认；确认框本身在更高层）。
3. 备忘录抽屉本身（根状态，进 app 栈；关闭抽屉）。
4. 其他已有层。

测试：打开抽屉后不移动焦点立即按 Escape，抽屉关、聊天不变；进入编辑后按一次 Escape 只退出编辑、抽屉仍开。

### 6.8 空状态与隐私

- 空列表：一段说明「备忘录只给你自己看，AI 不会读到。需要时点「插入」把它放进输入框。」+「新建备忘录」。
- 面板顶部常驻一行小字：「仅本机 · AI 不可见」。这是功能的核心承诺，值得一直显示。
- 隐私模式隐藏的项目：不会成为当前项目，命令层自然读不到。

## 7. 与现有功能的关系

| | 全局记忆 | 研究历程人工记录 | 备忘录 |
| --- | --- | --- | --- |
| 范围 | 全局 | 项目（分主线/分支） | 项目（主线与分支共享） |
| AI 是否看见 | 每轮注入 | 进回顾输入 | 永不，除非手动插入发送 |
| 用途 | 稳定偏好/事实 | 研究事实流水 | 私人速记、prompt 草稿 |
| 随项目导出 | 否 | 是 | 是 |

文档 `docs/basic-configuration.md` 或新建 `docs/project-memos.md` 加一节，并在全局记忆文档旁加一句"想记只给自己看的东西，用项目备忘录"。

## 8. 实施切分（每个 PR 一件事）

1. **存储 + DTO + 命令**：迁移、`project_memos.rs`、四个 Tauri 命令、项目包传输、DTO 契约测试、store 单测（CRUD、跨项目隔离、导入导出、旧包兼容）。
2. **侧栏抽屉 UI**：入口、列表、就地编辑、插入 composer、Escape 栈、i18n、Playwright 测试（mock bridge：新建→插入→composer 内容正确；Escape 顺序；空状态）。
3. **`/memo` 选择器**：composer picker 新模式 + 单测。
4. **隔离回归测试**：`agent_turn` 组装一轮后 prompt 中不含备忘录正文；`research_journey` 投影不含备忘录。这条可以并进 PR 1。

## 9. 待定问题

- 要不要在插入时附带标题行（如 `## 标题`）？倾向不加：标题是索引，正文才是 prompt。
- 要不要"从选中的聊天文本新建备忘录"（右键菜单）？有用但属于 PR 2 之后的增量。
- 导出到项目包是否应可选关闭（"私人内容不随包走"）？第一版随包导出，项目包本来就是研究者自己的东西；若后续项目包用于分享，再加开关。
