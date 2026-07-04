// Dev-only Tauri bridge mock. Load with http://localhost:1420/?mock=1
(function () {
  const listeners = {};
  const emit = (event, payload) => {
    try {
      listeners[event]?.({ payload });
    } catch (_) {}
  };

  const sessions = [
    { id: "s1", title: "查找文献, FX-cell", ts: 1719900000 },
    { id: "s2", title: "我确认下你你有什么skill", ts: 1719890000 },
    { id: "s3", title: "你能做啥", ts: 1719880000 },
  ];

  const project = {
    name: "wisp-science",
    root: "C:\\mock\\wisp-science",
    skill_count: 58,
    mcp_server_count: 24,
    memory_file_count: 0,
    has_api_key: true,
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
    },
  ];

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        switch (cmd) {
          case "list_sessions":
            return sessions;
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
                text: "## FX-cell literature\n\n| gene | score |\n| --- | --- |\n| FX-cell | 0.91 |\n\nSee `report.csv` or {{artifact:00000001}}.\n\n```python\nimport pandas as pd\ndf = pd.read_csv('report.csv')\nprint(df.head())\n```",
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
            return { provider: "openai", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", label: "deepseek-v4-pro", has_api_key: true, locale: "en", max_tokens: 4096, reasoning_effort: "" };
          case "list_models":
            return mockModels;
          case "save_model":
          case "remove_model":
          case "set_active_model":
            return mockModels;
          case "get_project_info":
            return project;
          case "get_onboarding_state":
            return { show: false, has_api_key: true };
          case "get_bootstrap_status":
            return { skills_loaded: 58, python_ok: true, mcp_catalog: 24, uv_ok: true, app_version: "0.3.0-mock", workspace: project.root, errors: [] };
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
          case "check_for_updates":
            return null;
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
            const fid = (args && args.session_id) || "mock-frame";
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
