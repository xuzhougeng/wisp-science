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
  let skills = [
    { name: "remote-compute-modal", description: "Run jobs on Modal", tags: ["compute"], enabled: true, builtin: true, dir: "/skills/remote-compute-modal" },
    { name: "alphafold2", description: "Predict protein structures", tags: ["protein", "structure"], enabled: true, builtin: true, dir: "/skills/alphafold2" },
    { name: "paper-narrative", description: "Shape a paper story", tags: [], enabled: true, builtin: false, dir: "/home/me/.wisp/skills/paper-narrative" },
  ];
  let memoryEnabled = true;
  let memoryFiles = [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }];
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

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        ((window as any).__skillInvokeLog ??= []).push({ cmd, args });
        const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
        switch (cmd) {
          case "list_demos":
            return demos;
          case "load_demo":
            return demo;
          case "list_sessions":
            return [];
          case "list_folders":
            return [];
          case "create_folder":
          case "rename_folder":
          case "delete_folder":
          case "move_session":
            return null;
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 }];
          case "list_recent_sessions":
            return [
              {
                id: "s-needs-you",
                project_id: "default",
                title: "帮我找一篇单细胞的文章",
                ts: 1,
                status: "needs_you",
              },
              {
                id: "s-complete",
                project_id: "default",
                title: "Enumerate MCP bio-tools databases",
                ts: 2,
                status: "complete",
              },
            ];
          case "pick_directory":
            return "/mock/root/new-project";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "delete_project":
            return null;
          case "open_project_window":
            return `proj-${arg("id")}`;
          case "get_settings":
            return { provider: "", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", has_api_key: true, locale: "en", max_tokens: 4096, reasoning_effort: "" };
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
          case "get_capabilities":
            return {
              skills,
              mcp_servers: ["mcp_bio", "mcp_chem"],
              memory_files: [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }],
              project,
            };
          case "list_skills":
            return skills;
          case "set_skill_tags": {
            const name = arg("name") ?? "";
            const tags = Array.isArray(arg("tags")) ? arg("tags") : [];
            skills = skills.map((s) => s.name === name ? { ...s, tags } : s);
            return null;
          }
          case "set_skill_enabled": {
            const name = arg("name") ?? "";
            const enabled = Boolean(arg("enabled"));
            skills = skills.map((s) => s.name === name ? { ...s, enabled } : s);
            return null;
          }
          case "set_skills_enabled": {
            const names = new Set(Array.isArray(arg("names")) ? arg("names") : []);
            const enabled = Boolean(arg("enabled"));
            skills = skills.map((s) => names.has(s.name) ? { ...s, enabled } : s);
            return null;
          }
          case "list_dir":
            return [
              { name: "data", is_dir: true, size: 0 },
              { name: "report.csv", is_dir: false, size: 4096 },
            ];
          case "read_file":
            return { path: arg("path") ?? "report.csv", mime: "text/csv", text: "a,b\n1,2", base64: null };
          case "get_artifact_provenance":
            return {
              code: "import matplotlib\nplt.savefig('volcano.png')",
              language: "python",
              output: "saved volcano.png",
              exit_status: "ok",
              inputs: [{ path: "DE_results.csv", produced_here: false }],
              env: { name: "kernel", packages: [{ name: "matplotlib", version: "3.8.0" }] },
            };
          case "upload_file":
            return {
              id: "art-upload-1",
              name: arg("filename") ?? "upload.csv",
              kind: "text/csv",
              path: `uploads/${arg("filename") ?? "upload.csv"}`,
              ts: 1,
            };
          case "set_settings":
          case "set_api_key":
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
          case "new_session":
            return `s-${Math.random().toString(36).slice(2)}`;
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            // Long-approval path (#63 regression test): emit a confirm-request
            // whose body is far taller than the viewport.
            if (String(arg("message") ?? "").includes("NEEDCONFIRM")) {
              const longBody = Array.from({ length: 120 }, (_, i) => `rm -rf /mock/path/line-${i}`).join("\n");
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: `Dangerous command detected:\n${longBody}`,
                    tool: "shell",
                    preview: longBody,
                  }),
                50,
              );
              return fid;
            }
            // Long-stream path (#61 regression test): drip many text deltas so the
            // thread re-renders repeatedly and grows well past the viewport.
            if (String(arg("message") ?? "").includes("SCROLLTEST")) {
              let n = 0;
              const tick = () => {
                if (n < 80) {
                  emit("agent", { kind: "Text", frame_id: fid, delta: `line ${n}\n` });
                  n++;
                  setTimeout(tick, 6);
                } else {
                  emit("agent", { kind: "Done", frame_id: fid });
                }
              };
              setTimeout(tick, 20);
              return fid;
            }
            // Multi-tool path (#82): a thinking + tool-call run that must fold
            // into one collapsible "steps" panel instead of a wall of cards.
            const msg = (args && args.message) || "";
            if (String(arg("message") ?? "").includes("STEPSDEMO")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Let me inspect the count matrix header first." });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "zcat counts.txt.gz | head" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: Array.from({ length: 8 }, (_, i) => `gene_${i}\t12\t8\t15`).join("\n") });
                emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Now load the full matrix and summarize." });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "import pandas as pd\ndf = pd.read_csv('counts.txt.gz', sep='\\t')" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: Array.from({ length: 18 }, (_, i) => `col_${i}: ok`).join("\n") });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "write", preview: "/mock/root/deseq2.R" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "write", ok: true, content: "" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The data is clean: 60,675 genes × 15 samples in a 2×2 factorial design." });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            setTimeout(() => {
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "Text", frame_id: fid, delta: "Hello " });
              emit("agent", { kind: "Text", frame_id: fid, delta: "from mock wisp-science." });
              emit("agent", { kind: "ToolResult", frame_id: fid, name: "read", ok: true, content: "ok" });
              emit("agent", { kind: "Done", frame_id: fid });
            }, 50);
            return fid;
          }
          case "open_external_url":
            if (arg("url")) window.open(String(arg("url")), "_blank", "noopener,noreferrer");
            return null;
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
  const queues: Record<string, Promise<void>> = {};

  const project = { name: "wisp-science", root: "/mock/root", skill_count: 12, mcp_server_count: 8, memory_file_count: 2, has_api_key: true };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        ((window as any).__sendInvokeLog ??= []).push({ cmd, args });
        switch (cmd) {
          case "list_demos": return [];
          case "load_demo": return { id: "x", title: "x", request: "x", response: "x" };
          case "list_sessions": return sessions.slice();
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 }];
          case "list_recent_sessions": return sessions.map((s) => ({
            id: s.id, project_id: "default", title: s.title, ts: s.ts,
            status: "complete",
          }));
          case "pick_directory": return "/mock/root/new-project";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "delete_project": return null;
          case "get_settings": return { provider: "openai", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", label: "deepseek-v4-pro", has_api_key: true, locale: "en" };
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
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            const msg = (args && args.message) || "";
            const run = async () => {
              if (!sessions.some((s) => s.id === fid)) {
                sessions.push({ id: fid, title: msg, ts: Date.now() });
              }
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "Text", frame_id: fid, delta: `echo:${msg}` });
              await new Promise((resolve) => setTimeout(resolve, 5000));
              emit("agent", { kind: "Done", frame_id: fid });
            };
            const previous = queues[fid] ?? Promise.resolve();
            const current = previous.then(run, run);
            queues[fid] = current.catch(() => undefined);
            await current;
            return fid;
          }
          case "open_external_url":
            if (arg("url")) window.open(String(arg("url")), "_blank", "noopener,noreferrer");
            return null;
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
