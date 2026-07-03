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
          case "pick_directory":
            return "/mock/root/new-project";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1 };
          case "delete_project":
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
            return `s-${Math.random().toString(36).slice(2)}`;
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "send_message": {
            const fid = (args && args.session_id) || "t1";
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

// Variant for parallel-session tests: each `send_message` streams an `echo:<msg>`
// reply immediately but delays `Done` so the session stays "running" while the
// test starts a second conversation. `list_sessions` reports every session that
// received a user turn so the sidebar can list them.
export function parallelMock(): void {
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try { listeners[event]?.({ payload }); } catch { /* not registered yet */ }
  };
  const sessions: { id: string; title: string; ts: number }[] = [];

  const project = { name: "wisp-science", root: "/mock/root", skill_count: 12, mcp_server_count: 8, memory_file_count: 2, has_api_key: true };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        switch (cmd) {
          case "list_demos": return [];
          case "load_demo": return { id: "x", title: "x", request: "x", response: "x" };
          case "list_sessions": return sessions.slice();
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1 }];
          case "list_recent_sessions": return sessions.map((s) => ({ id: s.id, project_id: "default", title: s.title, ts: s.ts }));
          case "pick_directory": return "/mock/root/new-project";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1 };
          case "delete_project": return null;
          case "get_settings": return { provider: "openai", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", has_api_key: true, locale: "en" };
          case "get_project_info": return project;
          case "get_onboarding_state": return { show: false, has_api_key: true };
          case "get_capabilities": return { skills: [], mcp_servers: [], memory_files: [], project };
          case "list_dir": return [];
          case "read_file": return { path: "x", mime: "text/plain", text: "", base64: null };
          case "upload_file": return { id: "a", name: "x", kind: "text/csv", path: "x", ts: 1 };
          case "new_session": return `s-${Math.random().toString(36).slice(2)}`;
          case "stop_agent":
          case "rewind_session":
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "validate_settings": return "ok";
          case "send_message": {
            const fid = (args && args.session_id) || "t1";
            const msg = (args && args.message) || "";
            sessions.push({ id: fid, title: msg, ts: Date.now() });
            // Stream the reply at once, but — like the real backend — keep the
            // turn "running": send_message stays PENDING until Done, so the
            // command's own resolution is the completion signal (the frontend
            // relies on this and no longer trusts the Done broadcast alone).
            // While A is pending here, a second conversation can start and A
            // still shows as running.
            emit("agent", { kind: "Text", frame_id: fid, delta: `echo:${msg}` });
            await new Promise((resolve) => setTimeout(resolve, 5000));
            emit("agent", { kind: "Done", frame_id: fid });
            return fid;
          }
          default: return null;
        }
      },
    },
    event: {
      listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
        listeners[event] = cb;
        return () => { listeners[event] = undefined; };
      },
    },
  };
}
