// Self-contained mock of the Tauri v2 webview globals. Passed to
// Playwright's `page.addInitScript`, so it runs in the page before the Leptos
// wasm boots and installs `window.__TAURI__` with canned invoke/listen data.
//
// Keep it dependency-free and closure-free: Playwright serializes the function
// source and runs it verbatim in the browser.
export function tauriMock(): void {
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try {
      listeners[event]?.({ payload });
    } catch {
      /* listener may not be registered yet */
    }
  };

  const demos = [
    { id: "manifest_crispr_screen", title: "Design a genome-wide CRISPR knockout screen targeting all kinases" },
    { id: "manifest_enzyme_engineering", title: "Engineer an enzyme for higher thermostability" },
  ];
  const demo = {
    id: "manifest_crispr_screen",
    title: "CRISPR screen",
    request: "Design a genome-wide CRISPR knockout screen targeting all kinases.",
    response: "## Human Kinome CRISPR-KO Screen\n\nDemo report: 2,072 targeting sgRNAs across 522 kinases.\n\n[Off-target analysis (figure)]",
    thinking: "Let me plan the kinome list and guide selection.",
  };

  const project = {
    name: "wisp-science",
    root: "/mock/root",
    skill_count: 12,
    mcp_server_count: 8,
    memory_file_count: 2,
    has_api_key: true,
  };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        switch (cmd) {
          case "list_demos":
            return demos;
          case "load_demo":
            return demo;
          case "list_sessions":
            return [];
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1 }];
          case "list_recent_sessions":
            return [];
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1 };
          case "delete_project":
          case "pick_directory":
            return null;
          case "get_settings":
            return { provider: "", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", has_api_key: true, locale: "en" };
          case "get_project_info":
            return project;
          case "get_onboarding_state":
            return { show: false, has_api_key: true };
          case "get_capabilities":
            return {
              skills: [{ name: "remote-compute-modal", description: "Run jobs on Modal" }],
              mcp_servers: ["mcp_bio", "mcp_chem"],
              memory_files: [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }],
              project,
            };
          case "list_dir":
            return [
              { name: "data", is_dir: true, size: 0 },
              { name: "report.csv", is_dir: false, size: 4096 },
            ];
          case "read_file":
            return { path: args?.path ?? "report.csv", mime: "text/csv", text: "a,b\n1,2", base64: null };
          case "upload_file":
            return {
              id: "art-upload-1",
              name: args?.filename ?? "upload.csv",
              kind: "text/csv",
              path: `uploads/${args?.filename ?? "upload.csv"}`,
              ts: 1,
            };
          case "set_settings":
          case "set_api_key":
            return null;
          case "validate_settings":
            return "Validated openai with deepseek-v4-pro";
          case "new_session":
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "send_message": {
            const fid = "t1";
            setTimeout(() => {
              emit("agent", { kind: "Text", frame_id: fid, delta: "Hello " });
              emit("agent", { kind: "Text", frame_id: fid, delta: "from mock wisp-science." });
              emit("agent", { kind: "ToolResult", frame_id: fid, name: "read", ok: true, content: "ok" });
              emit("agent", { kind: "Done", frame_id: fid });
            }, 50);
            return fid;
          }
          default:
            return null;
        }
      },
    },
    event: {
      listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
        listeners[event] = cb;
        return () => {
          listeners[event] = undefined;
        };
      },
    },
  };
}
