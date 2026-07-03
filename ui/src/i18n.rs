use leptos::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    #[default]
    En,
    Zh,
}

impl Locale {
    pub fn from_code(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("zh") || s.starts_with("zh-") || s.starts_with("zh_") {
            Self::Zh
        } else {
            Self::En
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }

    pub fn detect_browser() -> Self {
        web_sys::window()
            .and_then(|w| w.navigator().language())
            .map(|l| Self::from_code(&l))
            .unwrap_or_default()
    }
}

pub fn set_document_lang(locale: Locale) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(html) = doc.document_element() {
            let _ = html.set_attribute("lang", locale.code());
        }
    }
}

pub fn t(locale: Locale, key: &str) -> String {
    lookup(locale, key).unwrap_or(key).into()
}

fn lookup(locale: Locale, key: &str) -> Option<&'static str> {
    match (locale, key) {
        (Locale::En, "sidebar.new_session") => Some("New session"),
        (Locale::En, "sidebar.open_demo") => Some("Open demo"),
        (Locale::En, "sidebar.files") => Some("Files"),
        (Locale::En, "sidebar.sessions") => Some("Sessions"),
        (Locale::En, "sidebar.today") => Some("Today"),
        (Locale::En, "sidebar.earlier") => Some("Earlier"),
        (Locale::En, "sidebar.no_sessions") => Some("No saved sessions yet."),
        (Locale::En, "sidebar.untitled") => Some("Untitled session"),
        (Locale::En, "sidebar.capabilities") => Some("Capabilities"),
        (Locale::En, "sidebar.settings") => Some("Settings"),
        (Locale::En, "sidebar.collapse") => Some("Collapse"),
        (Locale::En, "sidebar.show") => Some("Show sidebar"),
        (Locale::En, "sidebar.skills_meta") => Some("{skills} skills · {mcp} MCP · {mem} mem"),
        (Locale::En, "center.new_session") => Some("New session"),
        (Locale::En, "center.toggle_panel") => Some("Toggle panel"),
        (Locale::En, "empty.title") => Some("How can I help with your science today?"),
        (Locale::En, "empty.subtitle") => Some("Design experiments, analyze data, or explore ~80 biological databases — all running locally."),
        (Locale::En, "composer.placeholder") => Some("Ask wisp-science to design, analyze, or build something…"),
        (Locale::En, "composer.hint") => Some("Enter to send · Shift+Enter for newline · Drop files to attach"),
        (Locale::En, "composer.attach") => Some("Attach"),
        (Locale::En, "composer.uploading") => Some("Uploading…"),
        (Locale::En, "composer.remove_attachment") => Some("Remove attachment"),
        (Locale::En, "composer.stop") => Some("Stop"),
        (Locale::En, "composer.send") => Some("Send"),
        (Locale::En, "right.artifacts") => Some("Artifacts"),
        (Locale::En, "right.artifacts_n") => Some("Artifacts ({n})"),
        (Locale::En, "right.file") => Some("File"),
        (Locale::En, "right.provenance") => Some("Provenance"),
        (Locale::En, "right.provenance_n") => Some("Provenance ({n})"),
        (Locale::En, "right.close") => Some("Close panel"),
        (Locale::En, "right.close_file") => Some("Close file"),
        (Locale::En, "right.no_artifacts.title") => Some("No artifacts yet"),
        (Locale::En, "right.no_artifacts.body") => Some("Markdown tables, CSV blocks, equations, and file previews from wisp-science appear here."),
        (Locale::En, "right.no_file.title") => Some("No file open"),
        (Locale::En, "right.no_file.body") => Some("Browse project files from the sidebar Files button, or click a path in chat."),
        (Locale::En, "right.browse_files") => Some("Browse files"),
        (Locale::En, "right.no_tools.title") => Some("No tool calls yet"),
        (Locale::En, "right.no_tools.body") => Some("Shell, Python, and MCP tool invocations appear here with inputs and outputs."),
        (Locale::En, "right.input") => Some("Input"),
        (Locale::En, "right.output") => Some("Output"),
        (Locale::En, "confirm.title") => Some("Confirm action"),
        (Locale::En, "confirm.deny") => Some("Deny"),
        (Locale::En, "confirm.approve") => Some("Approve"),
        (Locale::En, "demos.title") => Some("Open a demo session"),
        (Locale::En, "demos.hint") => Some("Pre-baked example runs from the bundled seed catalog (read-only)."),
        (Locale::En, "demos.close") => Some("Close"),
        (Locale::En, "settings.title") => Some("Settings"),
        (Locale::En, "settings.language") => Some("Language"),
        (Locale::En, "settings.language.en") => Some("English"),
        (Locale::En, "settings.language.zh") => Some("中文"),
        (Locale::En, "settings.provider") => Some("Provider"),
        (Locale::En, "settings.provider.openai") => Some("OpenAI-compatible"),
        (Locale::En, "settings.provider.openai_responses") => Some("OpenAI (Responses API)"),
        (Locale::En, "settings.provider.anthropic") => Some("Anthropic"),
        (Locale::En, "settings.api_url") => Some("API URL"),
        (Locale::En, "settings.model") => Some("Model"),
        (Locale::En, "settings.api_key") => Some("API key (stored in OS keyring)"),
        (Locale::En, "settings.workspace_dir") => Some("Workspace folder (blank = default; applies after restart)"),
        (Locale::En, "settings.stored_key") => Some("(stored — leave blank to keep)"),
        (Locale::En, "settings.tip") => Some("Tip: DeepSeek/OpenAI-compatible uses /chat/completions; Anthropic uses /v1/messages."),
        (Locale::En, "settings.check_updates") => Some("Check for updates"),
        (Locale::En, "settings.validate") => Some("Valid"),
        (Locale::En, "settings.cancel") => Some("Cancel"),
        (Locale::En, "settings.save") => Some("Save"),
        (Locale::En, "status.checking_updates") => Some("Checking for updates..."),
        (Locale::En, "status.update_check_complete") => Some("Update check complete."),
        (Locale::En, "status.failed_load_settings") => Some("Failed to load settings"),
        (Locale::En, "status.saving_settings") => Some("Saving settings..."),
        (Locale::En, "status.settings_saved") => Some("Settings saved"),
        (Locale::En, "status.validating") => Some("Validating current settings..."),
        (Locale::En, "status.validation_succeeded") => Some("Validation succeeded"),
        (Locale::En, "status.validated") => Some("Validated {provider} with {model}"),
        (Locale::En, "status.save_failed") => Some("Save failed: {msg}"),
        (Locale::En, "status.api_key_save_failed") => Some("API key save failed: {msg}"),
        (Locale::En, "status.validation_failed") => Some("Validation failed: {msg}"),
        (Locale::En, "status.send_failed") => Some("Send failed: {msg}"),
        (Locale::En, "status.usage") => Some("{in}k in / {out}k out | ctx {pct}%"),
        (Locale::En, "status.compact") => Some("compact {before} → {after}"),
        (Locale::En, "status.demo") => Some("demo: {title}"),
        (Locale::En, "err.api_url_required") => Some("API URL is required."),
        (Locale::En, "err.model_required") => Some("Model is required."),
        (Locale::En, "err.api_key_required") => Some("API key is required."),
        (Locale::En, "err.unknown") => Some("Unknown error"),
        (Locale::En, "err.validation_timeout") => Some("Validation timed out after 30s"),
        (Locale::En, "err.file_not_found") => Some("File not found: {path}"),
        (Locale::En, "chat.you") => Some("You"),
        (Locale::En, "chat.assistant") => Some("wisp-science"),
        (Locale::En, "chat.thinking") => Some("thinking"),
        (Locale::En, "chat.error") => Some("Error"),
        (Locale::En, "msg.copy") => Some("Copy"),
        (Locale::En, "msg.edit") => Some("Edit"),
        (Locale::En, "tool.running") => Some("Running"),
        (Locale::En, "tool.copy_code") => Some("Copy code"),
        (Locale::En, "tool.copy_input") => Some("Copy input"),
        (Locale::En, "tool.copy_output") => Some("Copy output"),
        (Locale::En, "ctx.copy") => Some("Copy"),
        (Locale::En, "ctx.cut") => Some("Cut"),
        (Locale::En, "ctx.paste") => Some("Paste"),
        (Locale::En, "ctx.select_all") => Some("Select all"),
        (Locale::En, "ctx.copy_code") => Some("Copy code"),
        (Locale::En, "ctx.copy_title") => Some("Copy title"),
        (Locale::En, "ctx.copy_name") => Some("Copy name"),
        (Locale::En, "ctx.copy_message") => Some("Copy message"),
        (Locale::En, "ctx.open_session") => Some("Open session"),
        (Locale::En, "artifact.latex") => Some("LaTeX"),
        (Locale::En, "artifact.table") => Some("Table {n}"),
        (Locale::En, "artifact.code") => Some("Code {n}"),
        (Locale::En, "artifact.equation") => Some("Equation {n}"),
        (Locale::En, "artifact.meta.table") => Some("{rows} rows × {cols} cols"),
        (Locale::En, "artifact.meta.code") => Some("{lang} · {lines} lines"),
        (Locale::En, "artifact.meta.fasta") => Some("{seqs} sequences"),
        (Locale::En, "artifact.kind.fasta") => Some("FASTA"),
        (Locale::En, "artifact.kind.msa") => Some("MSA"),
        (Locale::En, "artifact.meta.text") => Some("{chars} chars"),
        (Locale::En, "artifact.meta.file") => Some("{kind}"),
        (Locale::En, "table.rows_note") => Some("Showing first 500 of {total} rows"),
        (Locale::En, "loading") => Some("Loading…"),
        (Locale::En, "files.title") => Some("Project files"),
        (Locale::En, "files.root") => Some("Root: {path}"),
        (Locale::En, "caps.title") => Some("Capabilities"),
        (Locale::En, "caps.runtime") => Some("Runtime v{version}"),
        (Locale::En, "caps.workspace") => Some("Workspace: {path}"),
        (Locale::En, "caps.runtime_status") => Some("Python: {py} · uv: {uv} · skills: {skills} · bundled MCP packages: {mcp}"),
        (Locale::En, "caps.skills") => Some("Skills"),
        (Locale::En, "caps.mcp_servers") => Some("MCP servers"),
        (Locale::En, "caps.memory_files") => Some("Memory files"),
        (Locale::En, "caps.mcp_bio") => Some("MCP bio-tools"),
        (Locale::En, "caps.skills_section") => Some("Skills"),
        (Locale::En, "caps.permissions") => Some("Permissions"),
        (Locale::En, "caps.permissions_hint") => Some("Shell and destructive file operations require your approval in a confirm dialog before running."),
        (Locale::En, "caps.close") => Some("Close"),
        (Locale::En, "caps.setup_env") => Some("Set up environment"),
        (Locale::En, "caps.env_setup_prompt") => Some(
            "Capabilities shows Python and uv are missing on this machine. Call use_skill for local-env-setup first, then follow it to install uv and Python for my OS, verify the managed venv, and tell me when to restart wisp-science.",
        ),
        (Locale::En, "caps.ready") => Some("ready"),
        (Locale::En, "caps.missing") => Some("missing"),
        (Locale::En, "onboard.welcome.title") => Some("Welcome to wisp-science"),
        (Locale::En, "onboard.welcome.body") => Some("Your local science assistant — design experiments, analyze data, and query ~80 biological databases without leaving your machine."),
        (Locale::En, "onboard.connect.title") => Some("Connect your model"),
        (Locale::En, "onboard.connect.body") => Some("Add an API key in Settings (OpenAI-compatible or Anthropic). Keys are stored in your OS keyring, not in the project folder."),
        (Locale::En, "onboard.features.title") => Some("What wisp-science can do"),
        (Locale::En, "onboard.features.body") => Some("Run Python in a sandboxed REPL, call bio-tools MCP servers, preview PDFs/molecules/structures in the right panel, and browse project files from the sidebar."),
        (Locale::En, "onboard.back") => Some("Back"),
        (Locale::En, "onboard.next") => Some("Next"),
        (Locale::En, "onboard.start") => Some("Get started"),

        (Locale::En, "projects.title") => Some("Projects"),
        (Locale::En, "projects.new") => Some("New project"),
        (Locale::En, "projects.recent") => Some("Recent sessions"),
        (Locale::En, "projects.name_ph") => Some("Project name"),
        (Locale::En, "projects.choose_dir") => Some("Choose folder"),
        (Locale::En, "projects.create") => Some("Create"),
        (Locale::En, "projects.cancel") => Some("Cancel"),
        (Locale::En, "projects.sessions_n") => Some("{n} sessions"),
        (Locale::En, "projects.empty") => Some("No projects yet — create one to start."),
        (Locale::En, "projects.delete") => Some("Delete"),
        (Locale::En, "projects.delete_confirm") => Some("Remove this project from Wisp? Your files on disk are kept."),
        (Locale::En, "projects.back") => Some("Projects"),

        (Locale::Zh, "sidebar.new_session") => Some("新建会话"),
        (Locale::Zh, "sidebar.open_demo") => Some("打开示例"),
        (Locale::Zh, "sidebar.files") => Some("文件"),
        (Locale::Zh, "sidebar.sessions") => Some("会话"),
        (Locale::Zh, "sidebar.today") => Some("今天"),
        (Locale::Zh, "sidebar.earlier") => Some("更早"),
        (Locale::Zh, "sidebar.no_sessions") => Some("暂无已保存的会话。"),
        (Locale::Zh, "sidebar.untitled") => Some("未命名会话"),
        (Locale::Zh, "sidebar.capabilities") => Some("能力"),
        (Locale::Zh, "sidebar.settings") => Some("设置"),
        (Locale::Zh, "sidebar.collapse") => Some("收起"),
        (Locale::Zh, "sidebar.show") => Some("显示侧栏"),
        (Locale::Zh, "sidebar.skills_meta") => Some("{skills} 技能 · {mcp} MCP · {mem} 记忆"),
        (Locale::Zh, "center.new_session") => Some("新建会话"),
        (Locale::Zh, "center.toggle_panel") => Some("切换面板"),
        (Locale::Zh, "empty.title") => Some("今天想做什么科研？"),
        (Locale::Zh, "empty.subtitle") => Some("设计实验、分析数据，或探索约 80 个生物数据库——全部在本地运行。"),
        (Locale::Zh, "composer.placeholder") => Some("请 wisp-science 设计、分析或构建…"),
        (Locale::Zh, "composer.hint") => Some("Enter 发送 · Shift+Enter 换行 · 拖放文件可附加"),
        (Locale::Zh, "composer.attach") => Some("附加"),
        (Locale::Zh, "composer.uploading") => Some("上传中…"),
        (Locale::Zh, "composer.remove_attachment") => Some("移除附件"),
        (Locale::Zh, "composer.stop") => Some("停止"),
        (Locale::Zh, "composer.send") => Some("发送"),
        (Locale::Zh, "right.artifacts") => Some("产物"),
        (Locale::Zh, "right.artifacts_n") => Some("产物 ({n})"),
        (Locale::Zh, "right.file") => Some("文件"),
        (Locale::Zh, "right.provenance") => Some("溯源"),
        (Locale::Zh, "right.provenance_n") => Some("溯源 ({n})"),
        (Locale::Zh, "right.close") => Some("关闭面板"),
        (Locale::Zh, "right.close_file") => Some("关闭文件"),
        (Locale::Zh, "right.no_artifacts.title") => Some("暂无产物"),
        (Locale::Zh, "right.no_artifacts.body") => Some("Markdown 表格、CSV 块、公式和文件预览会显示在这里。"),
        (Locale::Zh, "right.no_file.title") => Some("未打开文件"),
        (Locale::Zh, "right.no_file.body") => Some("从侧栏「文件」浏览项目文件，或在对话中点击路径。"),
        (Locale::Zh, "right.browse_files") => Some("浏览文件"),
        (Locale::Zh, "right.no_tools.title") => Some("暂无工具调用"),
        (Locale::Zh, "right.no_tools.body") => Some("Shell、Python 和 MCP 工具调用及其输入输出会显示在这里。"),
        (Locale::Zh, "right.input") => Some("输入"),
        (Locale::Zh, "right.output") => Some("输出"),
        (Locale::Zh, "confirm.title") => Some("确认操作"),
        (Locale::Zh, "confirm.deny") => Some("拒绝"),
        (Locale::Zh, "confirm.approve") => Some("批准"),
        (Locale::Zh, "demos.title") => Some("打开示例会话"),
        (Locale::Zh, "demos.hint") => Some("来自内置种子目录的预置示例（只读）。"),
        (Locale::Zh, "demos.close") => Some("关闭"),
        (Locale::Zh, "settings.title") => Some("设置"),
        (Locale::Zh, "settings.language") => Some("语言"),
        (Locale::Zh, "settings.language.en") => Some("English"),
        (Locale::Zh, "settings.language.zh") => Some("中文"),
        (Locale::Zh, "settings.provider") => Some("提供商"),
        (Locale::Zh, "settings.provider.openai") => Some("OpenAI 兼容"),
        (Locale::Zh, "settings.provider.openai_responses") => Some("OpenAI (Responses API)"),
        (Locale::Zh, "settings.provider.anthropic") => Some("Anthropic"),
        (Locale::Zh, "settings.api_url") => Some("API 地址"),
        (Locale::Zh, "settings.model") => Some("模型"),
        (Locale::Zh, "settings.api_key") => Some("API 密钥（保存在系统密钥环）"),
        (Locale::Zh, "settings.workspace_dir") => Some("工作区目录（留空=默认；重启后生效）"),
        (Locale::Zh, "settings.stored_key") => Some("（已保存 — 留空则保持不变）"),
        (Locale::Zh, "settings.tip") => Some("提示：DeepSeek/OpenAI 兼容接口使用 /chat/completions；Anthropic 使用 /v1/messages。"),
        (Locale::Zh, "settings.check_updates") => Some("检查更新"),
        (Locale::Zh, "settings.validate") => Some("验证"),
        (Locale::Zh, "settings.cancel") => Some("取消"),
        (Locale::Zh, "settings.save") => Some("保存"),
        (Locale::Zh, "status.checking_updates") => Some("正在检查更新…"),
        (Locale::Zh, "status.update_check_complete") => Some("更新检查完成。"),
        (Locale::Zh, "status.failed_load_settings") => Some("加载设置失败"),
        (Locale::Zh, "status.saving_settings") => Some("正在保存设置…"),
        (Locale::Zh, "status.settings_saved") => Some("设置已保存"),
        (Locale::Zh, "status.validating") => Some("正在验证当前设置…"),
        (Locale::Zh, "status.validation_succeeded") => Some("验证成功"),
        (Locale::Zh, "status.validated") => Some("已验证 {provider} / {model}"),
        (Locale::Zh, "status.save_failed") => Some("保存失败：{msg}"),
        (Locale::Zh, "status.api_key_save_failed") => Some("API 密钥保存失败：{msg}"),
        (Locale::Zh, "status.validation_failed") => Some("验证失败：{msg}"),
        (Locale::Zh, "status.send_failed") => Some("发送失败：{msg}"),
        (Locale::Zh, "status.usage") => Some("{in}k 输入 / {out}k 输出 | 上下文 {pct}%"),
        (Locale::Zh, "status.compact") => Some("压缩 {before} → {after}"),
        (Locale::Zh, "status.demo") => Some("示例：{title}"),
        (Locale::Zh, "err.api_url_required") => Some("API 地址不能为空。"),
        (Locale::Zh, "err.model_required") => Some("模型不能为空。"),
        (Locale::Zh, "err.api_key_required") => Some("API 密钥不能为空。"),
        (Locale::Zh, "err.unknown") => Some("未知错误"),
        (Locale::Zh, "err.validation_timeout") => Some("验证超时（30 秒）"),
        (Locale::Zh, "err.file_not_found") => Some("文件未找到：{path}"),
        (Locale::Zh, "chat.you") => Some("你"),
        (Locale::Zh, "chat.assistant") => Some("wisp-science"),
        (Locale::Zh, "chat.thinking") => Some("思考中"),
        (Locale::Zh, "chat.error") => Some("错误"),
        (Locale::Zh, "msg.copy") => Some("复制"),
        (Locale::Zh, "msg.edit") => Some("编辑"),
        (Locale::Zh, "tool.running") => Some("运行中"),
        (Locale::Zh, "tool.copy_code") => Some("复制代码"),
        (Locale::Zh, "tool.copy_input") => Some("复制输入"),
        (Locale::Zh, "tool.copy_output") => Some("复制输出"),
        (Locale::Zh, "ctx.copy") => Some("复制"),
        (Locale::Zh, "ctx.cut") => Some("剪切"),
        (Locale::Zh, "ctx.paste") => Some("粘贴"),
        (Locale::Zh, "ctx.select_all") => Some("全选"),
        (Locale::Zh, "ctx.copy_code") => Some("复制代码"),
        (Locale::Zh, "ctx.copy_title") => Some("复制标题"),
        (Locale::Zh, "ctx.copy_name") => Some("复制名称"),
        (Locale::Zh, "ctx.copy_message") => Some("复制消息"),
        (Locale::Zh, "ctx.open_session") => Some("打开会话"),
        (Locale::Zh, "artifact.latex") => Some("LaTeX"),
        (Locale::Zh, "artifact.table") => Some("表格 {n}"),
        (Locale::Zh, "artifact.code") => Some("代码 {n}"),
        (Locale::Zh, "artifact.equation") => Some("公式 {n}"),
        (Locale::Zh, "artifact.meta.table") => Some("{rows} 行 × {cols} 列"),
        (Locale::Zh, "artifact.meta.code") => Some("{lang} · {lines} 行"),
        (Locale::Zh, "artifact.meta.fasta") => Some("{seqs} 条序列"),
        (Locale::Zh, "artifact.kind.fasta") => Some("FASTA"),
        (Locale::Zh, "artifact.kind.msa") => Some("MSA"),
        (Locale::Zh, "artifact.meta.text") => Some("{chars} 字符"),
        (Locale::Zh, "artifact.meta.file") => Some("{kind}"),
        (Locale::Zh, "table.rows_note") => Some("显示前 500 行，共 {total} 行"),
        (Locale::Zh, "loading") => Some("加载中…"),
        (Locale::Zh, "files.title") => Some("项目文件"),
        (Locale::Zh, "files.root") => Some("根目录：{path}"),
        (Locale::Zh, "caps.title") => Some("能力"),
        (Locale::Zh, "caps.runtime") => Some("运行时 v{version}"),
        (Locale::Zh, "caps.workspace") => Some("工作区：{path}"),
        (Locale::Zh, "caps.runtime_status") => Some("Python：{py} · uv：{uv} · 技能：{skills} · 内置 MCP：{mcp}"),
        (Locale::Zh, "caps.skills") => Some("技能"),
        (Locale::Zh, "caps.mcp_servers") => Some("MCP 服务"),
        (Locale::Zh, "caps.memory_files") => Some("记忆文件"),
        (Locale::Zh, "caps.mcp_bio") => Some("MCP 生物工具"),
        (Locale::Zh, "caps.skills_section") => Some("技能"),
        (Locale::Zh, "caps.permissions") => Some("权限"),
        (Locale::Zh, "caps.permissions_hint") => Some("Shell 和破坏性文件操作运行前会弹出确认对话框。"),
        (Locale::Zh, "caps.close") => Some("关闭"),
        (Locale::Zh, "caps.setup_env") => Some("配置环境"),
        (Locale::Zh, "caps.env_setup_prompt") => Some(
            "能力面板显示本机缺少 Python 和 uv。请先 use_skill 加载 local-env-setup，按该 skill 为我的系统安装 uv 和 Python，验证托管 venv，并告知何时重启 wisp-science。",
        ),
        (Locale::Zh, "caps.ready") => Some("就绪"),
        (Locale::Zh, "caps.missing") => Some("缺失"),
        (Locale::Zh, "onboard.welcome.title") => Some("欢迎使用 wisp-science"),
        (Locale::Zh, "onboard.welcome.body") => Some("本地科研助手——设计实验、分析数据，查询约 80 个生物数据库，无需离开本机。"),
        (Locale::Zh, "onboard.connect.title") => Some("连接模型"),
        (Locale::Zh, "onboard.connect.body") => Some("在设置中添加 API 密钥（OpenAI 兼容或 Anthropic）。密钥保存在系统密钥环，不会写入项目文件夹。"),
        (Locale::Zh, "onboard.features.title") => Some("wisp-science 能做什么"),
        (Locale::Zh, "onboard.features.body") => Some("在沙箱 REPL 中运行 Python、调用 bio-tools MCP 服务、在右侧面板预览 PDF/分子/结构，并从侧栏浏览项目文件。"),
        (Locale::Zh, "onboard.back") => Some("上一步"),
        (Locale::Zh, "onboard.next") => Some("下一步"),
        (Locale::Zh, "onboard.start") => Some("开始使用"),

        (Locale::Zh, "projects.title") => Some("项目"),
        (Locale::Zh, "projects.new") => Some("新建项目"),
        (Locale::Zh, "projects.recent") => Some("最近会话"),
        (Locale::Zh, "projects.name_ph") => Some("项目名称"),
        (Locale::Zh, "projects.choose_dir") => Some("选择文件夹"),
        (Locale::Zh, "projects.create") => Some("创建"),
        (Locale::Zh, "projects.cancel") => Some("取消"),
        (Locale::Zh, "projects.sessions_n") => Some("{n} 个会话"),
        (Locale::Zh, "projects.empty") => Some("还没有项目 —— 新建一个开始。"),
        (Locale::Zh, "projects.delete") => Some("删除"),
        (Locale::Zh, "projects.delete_confirm") => Some("从 Wisp 移除该项目？磁盘上的文件会保留。"),
        (Locale::Zh, "projects.back") => Some("项目"),

        _ => None,
    }
}

pub fn tf(locale: Locale, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = t(locale, key);
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

pub fn tab_count(locale: Locale, base: &str, n: usize) -> String {
    if n == 0 {
        t(locale, base)
    } else {
        tf(locale, &format!("{base}_n"), &[("n", &n.to_string())])
    }
}

pub fn localize_backend(locale: Locale, msg: &str) -> String {
    match msg {
        "API URL is required." => t(locale, "err.api_url_required"),
        "Model is required." => t(locale, "err.model_required"),
        "API key is required." => t(locale, "err.api_key_required"),
        "Validation timed out after 30s" => t(locale, "err.validation_timeout"),
        "Validation succeeded" => t(locale, "status.validation_succeeded"),
        m if m.starts_with("Validated ") => {
            if let Some(rest) = m.strip_prefix("Validated ") {
                if let Some((provider, model)) = rest.split_once(" with ") {
                    return tf(locale, "status.validated", &[("provider", provider), ("model", model)]);
                }
            }
            msg.to_string()
        }
        _ => msg.to_string(),
    }
}

pub fn use_locale() -> ReadSignal<Locale> {
    use_context::<ReadSignal<Locale>>().expect("locale context")
}
