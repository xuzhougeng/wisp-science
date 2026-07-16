// Dev-only Tauri bridge mock. Load with http://localhost:1421/?mock=1
(function () {
  const listeners = {};
  const emit = (event, payload) => {
    try {
      listeners[event]?.({ payload });
    } catch (_) {}
  };

  const sessions = [
    { id: "s1", title: "查找文献, FX-cell", ts: 1719900000, folder_id: "d1" },
    { id: "s2", title: "我确认下你你有什么skill", ts: 1719890000 },
    { id: "s3", title: "你能做啥", ts: 1719880000 },
  ];
  const folders = [{ id: "d1", name: "Research" }];

  const project = {
    id: "default",
    name: "wisp-science",
    root: "C:\\mock\\wisp-science",
    skill_count: 58,
    mcp_server_count: 24,
    memory_file_count: 0,
    has_api_key: true,
  };
  let mockUpdateCheck = {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases",
  };
  const memoryFiles = [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }];
  let memoryEnabled = true;
  const mockModels = [
    {
      id: "default",
      label: "deepseek-v4-pro",
      provider: "openai",
      api_url: "https://api.deepseek.com",
      model: "deepseek-v4-pro",
      has_api_key: true,
      active: true,
      max_tokens: 4096,
      reasoning_effort: "",
      supports_vision: true,
      use_for_vision: true,
    },
  ];
  const mockCredentials = {
    openalex_api_key: false,
    infinisynapse_api_key: false,
    scimaster_api_key: false,
    ncbi_api_key: false,
    ncbi_email: false,
  };
  const mockChannels = {
    feishu_enabled: false,
    feishu_app_id: "",
    feishu_has_secret: false,
    feishu_state: "stopped",
    feishu_detail: "",
    weixin_enabled: false,
    weixin_bound: false,
    weixin_state: "stopped",
    weixin_detail: "",
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        switch (cmd) {
          case "list_sessions":
            return sessions;
          case "list_sessions_page": {
            const cursor = args?.cursor;
            const start = cursor ? sessions.findIndex((item) => item.id === cursor.id) + 1 : 0;
            const items = sessions.slice(start, start + 100);
            const hasMore = start + items.length < sessions.length;
            const last = items.at(-1);
            return {
              items,
              next_cursor: hasMore && last ? { id: last.id, ts: last.ts } : null,
              running_ids: sessions.filter((item) => item.running).map((item) => item.id),
            };
          }
          case "list_folders":
            return folders;
          case "create_folder": {
            const id = "d" + (folders.length + 1);
            const row = { id, name: args?.name ?? "Folder" };
            folders.push(row);
            return row;
          }
          case "rename_folder": {
            const f = folders.find((x) => x.id === args?.id);
            if (f) f.name = args?.name ?? f.name;
            return null;
          }
          case "delete_folder": {
            const idx = folders.findIndex((x) => x.id === args?.id);
            if (idx >= 0) folders.splice(idx, 1);
            sessions.forEach((s) => { if (s.folder_id === args?.id) s.folder_id = null; });
            return null;
          }
          case "move_session": {
            const s = sessions.find((x) => x.id === args?.id);
            if (s) s.folder_id = args?.folderId ?? args?.folder_id ?? null;
            return null;
          }
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: sessions.length, updated_at: 1 }];
          case "list_recent_sessions":
            return sessions.map((s) => ({ id: s.id, project_id: "default", title: s.title, ts: s.ts }));
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: sessions.length, updated_at: 1 };
          case "delete_project":
            return null;
          case "pick_directory":
            return "/Users/mock/Desktop/demo-project";
          case "load_session":
            return [
              { role: "user", text: "查找文献 FX-cell", tool_name: null, ok: null },
              { role: "reasoning", text: "Search PubMed and preprints for FX-cell literature.", tool_name: null, ok: null },
              { role: "tool", text: "12 hits written to report.csv", tool_name: "python", ok: true },
              {
                role: "assistant",
                text: "## FX-cell literature\n\n| gene | score |\n| --- | --- |\n| FX-cell | 0.91 |\n\nThe score follows $s = \\frac{1}{1 + e^{-x}}$ and GPT-style \\(a_i^2 + b_i^2\\) too.\n\n$$\\int_0^1 x^2 \\, dx = \\frac{1}{3}$$\n\nSee `report.csv` or {{artifact:00000001}}.\n\n```python\nimport pandas as pd\ndf = pd.read_csv('report.csv')\nprint(df.head())\n```",
                tool_name: null,
                ok: null,
              },
            ];
          case "list_demos":
            return [{ id: "manifest_crispr_screen", title: "Design a genome-wide CRISPR knockout screen targeting all kinases" }];
          case "load_demo":
            return {
              id: "manifest_crispr_screen",
              title: "CRISPR screen",
              request: "Design a genome-wide CRISPR knockout screen targeting all kinases.",
              response: "## Human Kinome CRISPR-KO Screen\n\n| kinase | guides |\n| --- | --- |\n| AKT1 | 4 |",
              thinking: "Planning kinome coverage.",
            };
          case "get_settings":
            return { provider: "openai", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", label: "deepseek-v4-pro", has_api_key: true, locale: "en", max_iter: 100, max_tokens: 4096, reasoning_effort: "", supports_vision: true };
          case "list_models":
            return mockModels;
          case "credential_status":
            return Object.entries(mockCredentials);
          case "channels_status":
            return { ...mockChannels };
          case "set_feishu_channel":
            mockChannels.feishu_enabled = !!args?.enabled;
            mockChannels.feishu_app_id = args?.appId ?? "";
            if (args?.appSecret) mockChannels.feishu_has_secret = true;
            mockChannels.feishu_state = mockChannels.feishu_enabled ? "running" : "stopped";
            return null;
          case "set_weixin_channel":
            mockChannels.weixin_enabled = !!args?.enabled;
            mockChannels.weixin_state = mockChannels.weixin_enabled ? "running" : "stopped";
            return null;
          case "weixin_bind_start":
            return {
              qrcode: "mock-qr",
              qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="220" height="220"><rect width="220" height="220" fill="#8a8a8a"/></svg>'),
            };
          case "weixin_bind_poll":
            mockChannels.weixin_bound = true;
            return "confirmed";
          case "weixin_unbind":
            mockChannels.weixin_bound = false;
            mockChannels.weixin_enabled = false;
            mockChannels.weixin_state = "stopped";
            return null;
          case "save_model":
          case "remove_model":
          case "set_active_model":
            return mockModels;
          case "get_project_info":
            return project;
          case "get_onboarding_state":
            return { show: false, has_api_key: true };
          case "get_bootstrap_status":
            return { skills_loaded: 66, python_ok: true, mcp_catalog: 24, uv_ok: true, node_ok: true, npm_ok: true, sci_ok: true, pixi_ok: true, app_version: "0.4.0-mock", workspace: project.root, errors: [] };
          case "get_capabilities":
            return {
              skills: [{ name: "bear-support", description: "Find papers supporting a claim." }],
              mcp_servers: ["mcp_pubmed"],
              memory_files: [],
              project,
            };
          case "list_dir":
            return [
              { name: "data", is_dir: true, size: 0 },
              { name: "report.csv", is_dir: false, size: 4096 },
            ];
          case "read_file":
            return { path: args?.path ?? "report.csv", mime: "text/csv", text: "gene,score\nFX-cell,0.91", base64: null };
          case "set_settings":
          case "set_api_key":
          case "new_session":
            return `s-${Math.random().toString(36).slice(2)}`;
          case "set_credential":
            mockCredentials[String(args?.id ?? "")] = String(args?.value ?? "").trim().length > 0;
            return null;
          case "delete_session": {
            const id = args?.id;
            const i = sessions.findIndex((s) => s.id === id);
            if (i >= 0) sessions.splice(i, 1);
            return null;
          }
          case "rename_session": {
            const id = args?.id;
            const title = (args?.title ?? "").trim();
            const s = sessions.find((x) => x.id === id);
            if (s && title) s.title = title;
            return null;
          }
          case "rewind_session":
          case "confirm_response":
          case "dismiss_onboarding":
          case "stop_agent":
            return null;
          case "check_for_updates":
            return mockUpdateCheck;
          case "validate_settings":
            return "Validated openai with deepseek-v4-pro";
          case "get_memory_view":
            return { enabled: memoryEnabled, today_file: "2026-07-04.md", files: memoryFiles };
          case "set_memory_enabled":
            memoryEnabled = !!args?.enabled;
            return { enabled: memoryEnabled, today_file: "2026-07-04.md", files: memoryFiles };
          case "list_memory":
          case "write_memory_file":
          case "delete_memory_file":
          case "clear_memory":
            return memoryFiles;
          case "read_memory_file":
            return "User prefers DeepSeek.\n";
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "mock-frame";
            const msg = (args && args.message) || "";
            if (String(msg).includes("MDLIST")) {
              const md = [
                "FX细胞（FX cell）是一种常用于病毒学研究的人源细胞系，具有以下特点：",
                "",
                "- **来源**：从人胚肾细胞（HEK293）衍生",
                "- **应用**：广泛用于慢病毒载体包装和生产",
                "- **优势**：转染效率高，适合大规模病毒生产",
                "",
                "有什么我可以帮你的吗？",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 80);
              return fid;
            }
            if (String(msg).includes("MDCODE")) {
              const md = [
                "缺少的是：",
                "",
                "```text",
                "CAF状态 → 免疫变化",
                "CAF状态 → 上皮变化",
                "```",
                "",
                "```python",
                "def immune_change(caf_status):",
                "    # 暗色代码注释",
                "    return \"免疫变化\" if caf_status else None",
                "```",
                "",
                "```diff",
                "-CAF状态 → 未知",
                "+CAF状态 → 免疫变化",
                "```",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 80);
              return fid;
            }
            setTimeout(() => {
              emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Searching literature…" });
              emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "scimaster-cli search FX-cell" });
              emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: "12 hits" });
              emit("agent", { kind: "Text", frame_id: fid, delta: "Mock reply for: " + (args?.message ?? "") });
              emit("agent", { kind: "Usage", frame_id: fid, round: 1, input: 19800, output: 300, ctx_tokens: 12000, max_context: 1000000 });
              emit("agent", { kind: "Done", frame_id: fid });
            }, 80);
            return fid;
          }
          case "open_external_url":
            if (args?.url) window.open(args.url, "_blank", "noopener,noreferrer");
            return null;
          default:
            return null;
        }
      },
    },
    event: {
      listen: async (event, cb) => {
        listeners[event] = cb;
        return () => {
          delete listeners[event];
        };
      },
    },
  };
})();
