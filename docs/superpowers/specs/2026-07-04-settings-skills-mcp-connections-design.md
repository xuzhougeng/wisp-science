# Settings 页面:Skill 管理 + MCP Connections 设计

日期:2026-07-04
状态:已批准设计,待写实现计划

## 背景与目标

当前 wisp-science 的 Skill 和 MCP 都是固定的:

- **Skills** 从 `skill_paths()`(内置资源目录 + `<workspace>/.wisp/skills` + `~/.wisp/skills` + `WISP_SKILLS_PATH`)全量发现,全部注入 agent,**无 UI 开关、无添加入口**。
- **MCP** 只有内置 bio-tools 聚合包,通过 `wire_python_and_mcp` 在 agent 创建时挂载;仅能靠 `WISP_MCP_PKG` / `WISP_MCP_COMMAND` 环境变量控制;**无 UI、无远程支持**。
- `Capabilities` 弹窗是只读展示。

目标:提供一个**多分区 Settings 页面**,支持:

1. **Skill 管理** —— 列出所有 skill、逐个启用/禁用、通过文件选择器添加本地 skill、删除用户添加的 skill。
2. **Connections** —— 配置本地(stdio)或远程(HTTP)MCP 服务:增删改、启用/禁用、测试连接;内置 bio-tools 作为一个总开关。

## 关键架构约束(已核实)

- 前端:Leptos 0.6(Rust→WASM),单文件 `ui/src/main.rs`,通过 Tauri `invoke`/`listen` 与后端通信。
- 持久化:SQLite key-value(`wisp-store`),API key 走系统钥匙串。
- **Agent 生命周期**:每会话惰性创建一次(`ensure_active_frame`/`send_message`,lib.rs:571-588),缓存在 `SessionRuntime.agent`。Skill 描述在 `seed_system_prompt` 时写入 system prompt(仅当 `ctx.is_empty()`);MCP 工具在 `wire_python_and_mcp` 时挂载。→ **开关改动对新会话生效,不影响已运行会话**。
- **MCP 传输当前只有 stdio**(`crates/wisp-mcp/src/client.rs`)。远程支持需新增 HTTP 传输。
- 依赖已就绪:`reqwest` 0.12(workspace,含 `stream`/`json`/`rustls-tls`)、`tauri-plugin-dialog` v2。**无 `zip` 依赖**(故 .zip 上传延后)。
- 用户可写 skill 目录:`~/.wisp/skills/`;内置 skill 来自只读资源目录 `wisp_skills::bundled_dir()`。

## 数据模型与持久化

复用现有 settings key-value,**不建新表**。新增三个键:

| 键 | 类型 | 含义 |
|---|---|---|
| `disabled_skills` | JSON `Vec<String>` | 被禁用的 skill 名字集合;缺省 = 全部启用 |
| `mcp_connections` | JSON `Vec<McpConnection>` | 用户自定义 MCP 连接 |
| `bio_tools_enabled` | JSON `bool` | 内置 bio-tools 总开关,默认 `true` |

连接配置(定义在 `wisp-mcp` 或 src-tauri,序列化用 serde):

```rust
struct McpConnection {
    id: String,          // uuid v4
    name: String,        // 展示名 / 工具命名前缀
    enabled: bool,
    transport: McpTransport,
}

#[serde(tag = "kind", rename_all = "lowercase")]
enum McpTransport {
    Stdio { command: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String> },
    Http  { url: String, headers: Vec<(String, String)> },   // 远程 web MCP,headers 放鉴权
}
```

## 后端:Tauri commands

### Skills

- `list_skills()` —— 扩展现有命令,每项返回 `{ name, description, enabled, builtin, dir }`。`enabled = !disabled_skills.contains(name)`;`builtin = dir 在内置资源目录下`。
- `set_skill_enabled(name: String, enabled: bool)` —— 更新 `disabled_skills` 集合并持久化。
- `install_skill(src_path: String)` —— 前端用 `tauri-plugin-dialog` 选文件/文件夹后调用:
  - 若 `src_path` 是目录且含 `SKILL.md` → 递归拷贝到 `~/.wisp/skills/<name>/`。
  - 若 `src_path` 是 `SKILL.md` 文件 → 拷到 `~/.wisp/skills/<name>/SKILL.md`。
  - 解析 frontmatter 取 `name`;校验有 `name` 和 `description`,否则报错。
  - 目标目录已存在则报错(不覆盖)。
  - 成功后重载 `ActiveProject.skills`(见"运行时重挂载")。
- `remove_skill(name: String)` —— 仅允许删除 `~/.wisp/skills/<name>/`;内置 skill 拒绝(只能禁用)。删后重载 SkillIndex。

Agent 创建路径(lib.rs:573/581):seed 前用 `disabled_skills` 过滤 `ap.skills`。新增 `SkillIndex::filtered(&HashSet<String>) -> SkillIndex`(或在传给 `Agent::new` / `seed_system_prompt` 前构造过滤后的 `Arc<SkillIndex>`)。`use_skill` 工具也用过滤后的索引,禁用的 skill 不可调用。

### Connections

- `list_mcp_connections()` → `{ bio_tools_enabled, connections: Vec<McpConnection> }`。
- `add_mcp_connection(conn)` / `update_mcp_connection(conn)` / `delete_mcp_connection(id)`。
- `set_mcp_connection_enabled(id, enabled)` / `set_bio_tools_enabled(enabled)`。
- `test_mcp_connection(conn)` → 按 transport 建客户端 + `tools_list()`,返回工具数或错误串(不落库,纯探活)。

`wire_python_and_mcp`(lib.rs:457)扩展:保留现有 Python REPL 挂载;bio-tools 改为受 `bio_tools_enabled` 控制;之后遍历 `mcp_connections` 中 `enabled` 的项,按 transport 分派 `McpClient::launch`(stdio)或新的 HTTP 客户端,逐个 `register_mcp`。单个连接失败只记 error 不中断其余。

> 注:`wire_python_and_mcp` 目前只拿 `app_data`,需能读到 settings(传入 `&Store` 或预先取好 config)。

### wisp-mcp:新增 HTTP 传输

- Cargo.toml 加 `reqwest = { workspace = true }`。
- `McpClient` 内部改为 `enum Transport { Stdio(现有 child+pipes), Http(HttpTransport) }`,`request(method, params)` 内 match 分派。对外 `tools_list()` / `call()` 签名不变 → **`McpTool` 零改动**。
- 构造:保留 `launch(cmd, args)` / `launch_bio_tools(...)`(stdio);新增 `connect_http(url, headers)`。
- **HTTP 传输 = MCP Streamable HTTP**:
  - `POST <url>`,`Accept: application/json, text/event-stream`,body 是 JSON-RPC 请求。
  - 响应 `Content-Type: application/json` → 直接解析单个 JSON-RPC 响应。
  - 响应 `text/event-stream` → 解析 SSE 帧(`data:` 行拼 JSON),取匹配 `id` 的 JSON-RPC 响应。
  - `initialize` 响应若带 `Mcp-Session-Id` 头,后续请求带上该头。
  - 自定义 `headers`(鉴权)注入每次请求。

## 前端:多分区 Settings 页面(Leptos)

现有 Settings 弹窗(单页表单)→ 改为**左侧导航 + 右侧内容**,三个分区:

- **General** —— 迁移现有 provider / API URL / model / 语言 / 工作区 / Validate / 更新检查。
- **Skills** —— skill 列表(名 + 描述 + 开关);顶部"Add skill"按钮走 `tauri-plugin-dialog` 选文件/夹 → `install_skill`;用户 skill 带删除按钮,内置 skill 只有开关。
- **Connections** —— 顶部"Bio-tools"内置行(总开关);下面用户连接列表(名 / 类型徽标 / 开关 / 编辑 / 删除);"Add connection"打开表单(选 Local stdio 或 Remote URL,填对应字段),含"Test"按钮调 `test_mcp_connection`。

现有 **Capabilities 弹窗保持不动**(只读运行时状态)。

生效提示:Skills / Connections 分区顶部一行小字"改动对新会话生效",旁边复用现有"New session"入口。

## 运行时重挂载

- `install_skill` / `remove_skill` 后:重建 `ActiveProject.skills = Arc::new(SkillIndex::load(&skill_paths(&root)))` 写回 `state.active`(参考 lib.rs:809 已有的重建写法)。
- Skill/MCP 开关:仅更新持久化;下个新会话创建 agent 时自然读取新配置。不主动杀正在跑的会话 agent。

## 有意的简化(ponytail)

- **.zip 上传延后** —— 无 zip 依赖;先支持文件夹 / SKILL.md。加 zip 依赖再补。
- **bio-tools 保持聚合内置** —— 不拆成 87 个独立连接,一个总开关;用户连接是"额外加的"。
- **不做 GitHub 导入** —— 只做文件选择器上传。
- **开关对新会话生效** —— 不做会话中途热切换(与"消息已持久化、system prompt 不重 seed"的架构冲突,不值得)。
- **disabled_skills 全局** —— 不做 per-project;简单且够用。

## 测试(留可运行自检,无框架)

- `SkillIndex::filtered` —— 造含 A/B/C 的索引,禁用 B,断言剩 A/C 且 `use_skill` 找不到 B。
- `McpConnection` / `McpTransport` serde 往返 —— stdio 与 http 各一条,序列化再反序列化相等。
- **HTTP 传输 SSE 解析** —— 喂一段固定 `text/event-stream` body(含 `data:` 帧),断言解出正确 `id` 的 JSON-RPC 结果。这是本次唯一有分支的解析逻辑,必须有自检。

## 涉及文件

- `crates/wisp-mcp/src/client.rs`、`Cargo.toml` —— Transport 枚举 + HTTP 客户端。
- `crates/wisp-skills/src/index.rs` —— `SkillIndex::filtered`。
- `src-tauri/src/lib.rs` —— 新命令、`wire_python_and_mcp` 扩展、skill 安装/删除/重载、命令注册。
- `ui/src/main.rs`、`ui/src/i18n.rs`、`ui/styles.css` —— 多分区 Settings UI + 文案 + 样式。
