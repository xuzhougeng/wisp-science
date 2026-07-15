// Self-contained mock of the Tauri v2 webview globals. Passed to
// Playwright's `page.addInitScript`, so it runs in the page before the Leptos
// wasm boots and installs `window.__TAURI__` with canned invoke/listen data.
//
// Keep it dependency-free and closure-free: Playwright serializes the function
// source and runs it verbatim in the browser.
export function tauriMock(): void {
  class Channel {
    onmessage: ((message: any) => void) | null = null;
  }
  const pdfBase64 = "JVBERi0xLjQKJVdpc3AKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUiA0IDAgUl0gL0NvdW50IDIgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNSAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNiAwIFIgPj4KZW5kb2JqCjUgMCBvYmoKPDwgL0xlbmd0aCA0OCA+PgpzdHJlYW0KQlQgL0YxIDI0IFRmIDcyIDcyMCBUZCAoUERGIHByZXZpZXcgd29ya3MpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNiAwIG9iago8PCAvTGVuZ3RoIDQ2ID4+CnN0cmVhbQpCVCAvRjEgMjQgVGYgNzIgNzIwIFRkIChTZWNvbmQgUERGIHBhZ2UpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNyAwIG9iago8PCAvVHlwZSAvRm9udCAvU3VidHlwZSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA4CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAxNSAwMDAwIG4gCjAwMDAwMDAwNjQgMDAwMDAgbiAKMDAwMDAwMDEyNyAwMDAwMCBuIAowMDAwMDAwMjUzIDAwMDAwIG4gCjAwMDAwMDAzNzkgMDAwMDAgbiAKMDAwMDAwMDQ3NyAwMDAwMCBuIAowMDAwMDAwNTczIDAwMDAwIG4gCnRyYWlsZXIKPDwgL1NpemUgOCAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKNjQyCiUlRU9GCg==";
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try {
      listeners[event]?.({ payload });
    } catch {
      /* listener may not be registered yet */
    }
  };
  (window as any).__tauriEmit = emit;

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
    id: "default",
    name: "wisp-science",
    root: "/mock/root",
    skill_count: 12,
    mcp_server_count: 8,
    memory_file_count: 2,
    has_api_key: true,
  };
  const query = new URLSearchParams(window.location.search);
  const mockLongPages = Number(query.get("mockLongPages") ?? 0);
  const mockLongSession = query.get("mockLongSession") === "1" || mockLongPages > 0;
  const mockResourceSession = query.get("mockResourceSession") === "1";
  const mockSessions = query.get("mockManySessions") === "1"
    ? Array.from({ length: 101 }, (_, index) => ({
        id: `session-${String(index + 1).padStart(3, "0")}`,
        title: `Paged session ${index + 1}`,
        ts: 2000 - index,
        running: false,
      }))
    : mockLongSession
      ? [{ id: "long-session", title: "Long transcript", ts: 2000, running: false }]
      : [];
  let activeProjectId = "default";
  let terminalCounter = 0;
  let mockUpdateCheck = {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases",
  };
  let mockUpdateCheckPending = false;
  let mockPetEnabled = new URLSearchParams(window.location.search).get("mockPet") === "1";
  let mockPetDirectory = mockPetEnabled ? "C:\\Users\\tester\\.codex\\pets\\wispy" : "";
  (window as any).__petWindowVisible = false;
  let resolveMockUpdateCheck: (() => void) | null = null;
  const syncedProjects = new Set<string>();
  const nextProjectOpenDelayMs: Record<string, number> = {};
  let failNextProjectOpenId: string | null = null;
  (window as any).__delayNextProjectOpen = (projectId: string, milliseconds: number) => {
    nextProjectOpenDelayMs[String(projectId)] = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__failNextProjectOpen = (projectId: string) => {
    failNextProjectOpenId = String(projectId);
  };
  (window as any).__setMockUpdateCheck = (value: Record<string, unknown>) => {
    mockUpdateCheck = { ...mockUpdateCheck, ...(value ?? {}) };
  };
  (window as any).__setMockUpdateCheckPending = (pending: boolean) => {
    mockUpdateCheckPending = Boolean(pending);
  };
  (window as any).__resolveMockUpdateCheck = () => {
    resolveMockUpdateCheck?.();
    resolveMockUpdateCheck = null;
  };
  let skills = [
    { name: "remote-compute-modal", description: "Run jobs on Modal", tags: ["compute"], enabled: true, builtin: true, dir: "/skills/remote-compute-modal" },
    { name: "alphafold2", description: "Predict protein structures", tags: ["protein", "structure"], enabled: true, builtin: true, dir: "/skills/alphafold2" },
    { name: "paper-narrative", description: "Shape a paper story", tags: [], enabled: true, builtin: false, dir: "/home/me/.wisp/skills/paper-narrative" },
  ];
  let memoryEnabled = true;
  let autoReviewEnabled = true;
  let memoryFiles = [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }];
  let mockSpecialists: any[] = [
    { id: "reviewer", name: "Reviewer", icon: "review", color: "clay", description: "", instructions: "rubric", model_id: "", skills: [], connectors: [], builtin: true },
  ];
  let sessionSpecialists: Record<string, string> = {};
  let mockModels = [
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
    {
      id: "opus",
      label: "opus-4.8",
      provider: "anthropic",
      api_url: "https://api.anthropic.com",
      model: "opus-4.8",
      has_api_key: true,
      active: false,
      max_tokens: 4096,
      reasoning_effort: "",
      supports_vision: true,
      use_for_vision: false,
    },
  ];
  let mockAcpAgents = [
    { id: "acp-test", label: "Test ACP Agent", command: "fake-acp", args: ["--stdio"] },
  ];
  const acpBindings: Record<string, string> = {};
  const acpPermissionFrames: Record<string, string> = {};
  const acpLongResolvers: Record<string, (value: string) => void> = {};
  let mockCredentials: Record<string, boolean> = {
    openalex_api_key: false,
    infinisynapse_api_key: false,
    scimaster_api_key: false,
    ncbi_api_key: false,
    ncbi_email: false,
  };
  let mockApprovalGrants = [
    {
      scope: "global",
      kind: "command",
      target: "shell",
      label: "Shell commands",
    },
  ];
  let mockMcpConnections = [
    {
      id: "conn-wolai",
      name: "wolai_cmp",
      enabled: true,
      transport: {
        kind: "http",
        url: "https://api.wolai.com/v1/mcp/",
        headers: [],
      },
    },
  ];
  const mockMcpTools = [
    { name: "wolai_search", description: "Search Wolai pages", inputSchema: { type: "object", properties: {} } },
    { name: "wolai_create_page", description: "Create a Wolai page", inputSchema: { type: "object", properties: {} } },
  ];
  const executionContexts = [
    {
      id: "local",
      kind: "local",
      label: "Local machine",
      config_json: "{}",
      capabilities_json: "{\"os\":\"linux\",\"arch\":\"x86_64\",\"python\":\"3.12.1\"}",
      last_probe_at: 1783482000,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482000,
    },
    {
      id: "ssh:gpu-server",
      kind: "ssh",
      label: "gpu-server",
      config_json: "{}",
      capabilities_json: "{\"gpu_summary\":\"NVIDIA A100\",\"scheduler\":\"slurm\",\"python_executable\":\"/opt/python/bin/python\",\"rscript_executable\":\"/opt/R/bin/Rscript\",\"r_jsonlite\":true}",
      last_probe_at: 1783482300,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482300,
    },
  ];
  let runtimeInfos: any[] = [
    {
      runtimeId: "runtime-python-local",
      generation: 1,
      key: { projectId: "default", contextId: "local", language: "python" },
      status: "ready",
      interpreter: "/mock/python",
      version: "3.12.1",
      processId: 1201,
      startedAtMs: Date.now() - 60_000,
      lastActivityAtMs: Date.now() - 5_000,
      residentMemoryBytes: 512 * 1024 * 1024,
      lastError: null,
    },
    {
      runtimeId: "runtime-r-local",
      generation: 2,
      key: { projectId: "default", contextId: "local", language: "r" },
      status: "dead",
      interpreter: "/usr/bin/Rscript",
      version: "4.4.1",
      processId: null,
      startedAtMs: Date.now() - 120_000,
      lastActivityAtMs: Date.now() - 30_000,
      residentMemoryBytes: null,
      lastError: "runtime process exited unexpectedly",
    },
    {
      runtimeId: "runtime-python-ssh",
      generation: 1,
      key: { projectId: "default", contextId: "ssh:gpu-server", language: "python" },
      status: "busy",
      interpreter: "/opt/python/bin/python",
      version: "3.11.9",
      processId: 2201,
      startedAtMs: Date.now() - 180_000,
      lastActivityAtMs: Date.now(),
      residentMemoryBytes: 10 * 1024 * 1024 * 1024,
      lastError: null,
    },
  ];
  const runs = [
    {
      id: "run-kinase-001",
      project_id: "default",
      frame_id: "s-complete",
      context_id: "ssh:gpu-server",
      title: "Kinase screen QC",
      kind: "ssh_direct",
      status: "succeeded",
      command: "python qc.py",
      script_path: null,
      input_refs_json: "[]",
      output_specs_json: "[]",
      created_at: 1783482600,
      started_at: 1783482605,
      ended_at: 1783482609,
      exit_code: 0,
      stdout_tail: "wrote qc table",
      stderr_tail: "",
      remote_workdir: "~/.wisp-science/runs/run-kinase-001",
      remote_handle_json: "{\"kind\":\"ssh_direct\"}",
      timeout_secs: 14400,
      last_polled_at: 1783482609,
      last_poll_error: null,
      env_snapshot_json: "{}",
    },
    {
      id: "run-local-002",
      project_id: "default",
      frame_id: "s-complete",
      context_id: "local",
      title: "Local normalization",
      kind: "command",
      status: "running",
      command: "python normalize.py",
      script_path: null,
      input_refs_json: "[]",
      output_specs_json: "[]",
      created_at: 1783482700,
      started_at: 1783482701,
      ended_at: null,
      exit_code: null,
      stdout_tail: "",
      stderr_tail: "",
      remote_workdir: null,
      remote_handle_json: null,
      timeout_secs: 300,
      last_polled_at: null,
      last_poll_error: null,
      env_snapshot_json: "{}",
    },
  ];
  const artifacts = [
    { id: "art-tree", name: "nif3.treefile", kind: "text/treefile", path: "nif3.treefile", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
    { id: "art-profile", name: "plddt_profile.png", kind: "image/png", path: "plddt_profile.png", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-old", session_title: "Older structure run", origin: "output" },
    { id: "art-counts", name: "counts.csv", kind: "text/csv", path: "counts.csv", ts: Math.floor(Date.now() / 1000), project_id: "other", project_name: "Other project", session_id: "s-other", session_title: "Cross-project counts", origin: "upload" },
    { id: "art-html", name: "dashboard.html", kind: "text/html", path: "dashboard.html", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
    { id: "art-markdown", name: "analysis-report.md", kind: "text/markdown", path: "analysis-report.md", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
  ];
  let libraryItems: any[] = [];

  (window as any).__TAURI__ = {
    core: {
      Channel,
      invoke: async (cmd: string, args: any) => {
        ((window as any).__skillInvokeLog ??= []).push({ cmd, args });
        const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
        const plain = (value: any): any => {
          if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
          if (Array.isArray(value)) return value.map(plain);
          if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
          return value;
        };
        switch (cmd) {
          case "list_library_items":
            return libraryItems.map(({ base64: _base64, ...item }) => item);
          case "star_library_code": {
            const sessionId = String(arg("sessionId") ?? "");
            const language = String(arg("language") ?? "");
            const code = String(arg("code") ?? "");
            const existing = libraryItems.find((item) => item.kind === "code"
              && item.source_session_id === sessionId && item.language === language && item.code === code);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "code",
              title: code.split("\n").find((line) => line.trim())?.trim() ?? "Code",
              language,
              code,
              content_type: null,
              source_project_id: activeProjectId,
              source_project_name: activeProjectId === "other" ? "Other project" : project.name,
              source_session_id: sessionId,
              source_session_title: "Current analysis",
              source_path: null,
              created_at: Math.floor(Date.now() / 1000),
              base64: null,
            };
            libraryItems.unshift(item);
            return item;
          }
          case "star_library_figure": {
            const sessionId = String(arg("sessionId") ?? "");
            const path = String(arg("path") ?? "").replaceAll("\\", "/").replace(/^\.\//, "");
            const existing = libraryItems.find((item) => item.kind === "figure"
              && item.source_session_id === sessionId && item.source_path === path);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "figure",
              title: String(arg("name") ?? "Figure"),
              language: "python",
              code: "import matplotlib\nplt.savefig('volcano.png')",
              content_type: "image/png",
              source_project_id: activeProjectId,
              source_project_name: activeProjectId === "other" ? "Other project" : project.name,
              source_session_id: sessionId,
              source_session_title: "Current analysis",
              source_path: path,
              created_at: Math.floor(Date.now() / 1000),
              base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=",
            };
            libraryItems.unshift(item);
            return item;
          }
          case "get_library_item": {
            const item = libraryItems.find((entry) => entry.id === arg("id"));
            if (!item) throw new Error("Library item not found");
            return item;
          }
          case "delete_library_item": {
            const before = libraryItems.length;
            libraryItems = libraryItems.filter((entry) => entry.id !== arg("id"));
            return libraryItems.length !== before;
          }
          case "list_demos":
            return demos;
          case "load_demo":
            return demo;
          case "load_session":
            if (mockResourceSession) {
              return {
                items: [{
                  role: "assistant",
                  text: "[Open bound report](D:/ZZM/03.%20figures/report.md')",
                  tool_name: null,
                  ok: null,
                  resources: [{
                    id: "resource-link-markdown",
                    ordinal: 0,
                    originalReference: "D:/ZZM/03.%20figures/report.md'",
                    artifactId: "resource-artifact-markdown",
                    artifactVersionId: "resource-version-markdown",
                    displayName: "report.md",
                    kind: "markdown",
                    mimeType: "text/markdown",
                    status: "ready",
                    error: null,
                  }],
                }],
                next_before_seq: null,
                user_offset: 0,
              };
            }
            if (mockLongSession) {
              const before = arg("beforeSeq");
              ((window as any).__transcriptPageCalls ??= []).push(before ?? null);
              if (mockLongPages > 0) {
                const pageIndex = before == null ? 0 : Number(before);
                return {
                  items: Array.from({ length: 20 }, (_, index) => ({
                    role: index % 2 === 0 ? "user" : "assistant",
                    text: `Window page ${pageIndex} row ${index} ${"x".repeat(256)}`,
                    tool_name: null,
                    ok: null,
                  })),
                  next_before_seq: pageIndex + 1 < mockLongPages ? pageIndex + 1 : null,
                  user_offset: Math.max(0, (mockLongPages - pageIndex - 1) * 10),
                };
              }
              if (before != null) {
                return {
                  items: Array.from({ length: 20 }, (_, index) => ({
                    role: index % 2 === 0 ? "user" : "assistant",
                    text: index === 0 ? "Oldest loaded question" : `Earlier transcript row ${index}`,
                    tool_name: null,
                    ok: null,
                  })),
                  next_before_seq: null,
                  user_offset: 0,
                };
              }
              return {
                items: Array.from({ length: 20 }, (_, index) => ({
                  role: index % 2 === 0 ? "user" : "assistant",
                  text: index === 0 ? "Newest page first question" : `Newest transcript row ${index}`,
                  tool_name: null,
                  ok: null,
                })),
                next_before_seq: 41,
                user_offset: 10,
              };
            }
            return { items: [], next_before_seq: null, user_offset: 0 };
          case "list_sessions":
            ((window as any).__projectSessionRefreshes ??= []).push(activeProjectId);
            return mockSessions;
          case "list_sessions_page": {
            ((window as any).__projectSessionRefreshes ??= []).push(activeProjectId);
            const cursor = plain(arg("cursor"));
            const start = cursor ? mockSessions.findIndex((item) => item.id === cursor.id) + 1 : 0;
            const items = mockSessions.slice(start, start + 100);
            const hasMore = start + items.length < mockSessions.length;
            const last = items.at(-1);
            return {
              items,
              next_cursor: hasMore && last ? { id: last.id, ts: last.ts } : null,
              running_ids: mockSessions.filter((item) => item.running).map((item) => item.id),
            };
          }
          case "list_folders":
            ((window as any).__projectFolderRefreshes ??= []).push(activeProjectId);
            return [];
          case "create_folder":
          case "rename_folder":
          case "delete_folder":
          case "move_session":
            return null;
          case "list_projects":
            return [
              { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0, sync_configured: syncedProjects.has("default"), last_synced_at: syncedProjects.has("default") ? Math.floor(Date.now() / 1000) : null },
              { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 1, updated_at: 1, running_count: 0, needs_you_count: 0, sync_configured: syncedProjects.has("other"), last_synced_at: syncedProjects.has("other") ? Math.floor(Date.now() / 1000) : null },
            ];
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
          case "open_project": {
            const openingProjectId = String(arg("id") ?? "default");
            const delay = nextProjectOpenDelayMs[openingProjectId] ?? 0;
            delete nextProjectOpenDelayMs[openingProjectId];
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            if (failNextProjectOpenId === openingProjectId) {
              failNextProjectOpenId = null;
              throw new Error(`mock failed to open ${openingProjectId}`);
            }
            activeProjectId = openingProjectId;
            ((window as any).__projectOpenCompletions ??= []).push(activeProjectId);
            return { id: activeProjectId, name: activeProjectId === "other" ? "Other project" : project.name, workspace_dir: activeProjectId === "other" ? "/mock/other" : project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          }
          case "create_project":
            activeProjectId = "default";
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "import_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "join_synced_project":
            return { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 1, updated_at: 2, running_count: 0, needs_you_count: 0 };
          case "export_project":
            return "/mock/wisp-project.zip";
          case "sync_project":
            if ((window as any).__failSyncConflict) {
              (window as any).__failSyncConflict = false;
              throw new Error("Sync conflict: this device and another device both changed the project. No data was overwritten.");
            }
            syncedProjects.add(String(arg("id") ?? "default"));
            return { status: "synced", direction: "push", revision: "revision-1", uploadedFiles: 1, downloadedFiles: 0, skippedPaths: [] };
          case "resolve_project_sync":
            return { status: "synced", direction: arg("strategy") === "remote" ? "pull" : "push", revision: "revision-2", uploadedFiles: 1, downloadedFiles: 1, skippedPaths: [] };
          case "project_sync_code":
            return "wisp-sync:mock-secret-code";
          case "get_project_sync_status":
            return { configured: true, transportKind: "folder", lastSyncedAt: 1, lastDirection: "push", revision: "revision-1" };
          case "delete_project":
            return null;
          case "open_project_window":
            return `proj-${arg("id")}`;
          case "get_settings":
            return {
              provider: "",
              api_url: "https://api.deepseek.com",
              model: "deepseek-v4-pro",
              has_api_key: true,
              locale: "en",
              max_tokens: 4096,
              reasoning_effort: "",
              supports_vision: true,
              sync_backend: "relay",
              sync_relay_url: "https://relay.example.test",
              sync_folder: "",
              sync_relay_token: "",
              has_sync_relay_token: true,
              pet_enabled: mockPetEnabled,
              pet_directory: mockPetDirectory,
            };
          case "get_pet":
            return {
              enabled: mockPetEnabled,
              directory: mockPetDirectory,
              error: null,
              asset: mockPetEnabled ? {
                id: "wispy",
                displayName: "Wispy",
                description: "A cheerful neon terminal spirit.",
                spriteVersionNumber: 2,
                spritesheetDataUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=",
                frameCounts: { idle: 7, "running-right": 8, "running-left": 8, waving: 4, jumping: 5, failed: 8, waiting: 6, running: 6, review: 6 },
              } : null,
            };
          case "get_pet_runtime_status":
            return { running: [], waiting: [], reviewing: [] };
          case "set_pet_window_visible":
            (window as any).__petWindowVisible = Boolean(arg("visible"));
            return null;
          case "list_models":
            return mockModels;
          case "list_acp_agents":
            return mockAcpAgents;
          case "get_acp_session_agent":
            return acpBindings[String(arg("frameId") ?? "")] ?? null;
          case "save_acp_agent": {
            const profile = { ...(plain(arg("profile")) ?? {}) };
            if (!profile.id) profile.id = `acp-${mockAcpAgents.length + 1}`;
            const index = mockAcpAgents.findIndex((agent) => agent.id === profile.id);
            if (index >= 0) mockAcpAgents[index] = profile;
            else mockAcpAgents.push(profile);
            return mockAcpAgents;
          }
          case "remove_acp_agent":
            mockAcpAgents = mockAcpAgents.filter((agent) => agent.id !== arg("id"));
            return mockAcpAgents;
          case "test_acp_agent":
            return {
              protocolVersion: 1,
              implementation: { name: "fake-acp", title: "Fake ACP", version: "1.0" },
              capabilities: { loadSession: true, sessionCapabilities: { configOptions: true } },
              authMethods: [{ id: "browser", name: "Sign in", description: "Authenticate in browser" }],
            };
          case "authenticate_acp_agent":
            return null;
          case "set_acp_session_config":
            return [{ id: "model", name: "Model", type: "select", currentValue: arg("value")?.value ?? "fast", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] }];
          case "set_acp_session_mode":
            return String(arg("modeId") ?? "");
          case "respond_acp_permission":
            setTimeout(() => {
              const requestId = String(arg("requestId"));
              const frameId = acpPermissionFrames[requestId] ?? "";
              emit("permission-resolved", { frameId, requestId });
              emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
              delete acpPermissionFrames[requestId];
            }, 0);
            return null;
          case "credential_status":
            return Object.entries(mockCredentials);
          case "list_ssh_hosts":
            return [{
              alias: "gpu-server",
              user: "researcher",
              port: 22,
              identity_file: null,
              notes: "Mock GPU host",
            }];
          case "list_execution_contexts":
            return executionContexts;
          case "probe_execution_context":
            return executionContexts.find((context) =>
              context.id === String(arg("contextId") ?? arg("context_id"))
            ) ?? null;
          case "update_execution_context_interpreters": {
            const context = executionContexts.find((item) =>
              item.id === String(arg("contextId") ?? arg("context_id"))
            );
            if (!context) throw new Error("Execution context not found");
            const config = JSON.parse(context.config_json || "{}");
            delete config.python_path;
            delete config.rscript_path;
            const python = String(arg("pythonExecutable") ?? arg("python_executable") ?? "").trim();
            const rscript = String(arg("rscriptExecutable") ?? arg("rscript_executable") ?? "").trim();
            if (python) config.python_executable = python;
            else delete config.python_executable;
            if (rscript) config.rscript_executable = rscript;
            else delete config.rscript_executable;
            context.config_json = JSON.stringify(config);
            return context;
          }
          case "list_runtimes":
            return runtimeInfos;
          case "inspect_runtime":
            return {
              objects: [
                {
                  name: "counts",
                  typeName: "DataFrame",
                  summary: "12000000 × 48",
                  sizeBytes: 4 * 1024 * 1024 * 1024,
                },
                {
                  name: "model",
                  typeName: "RandomForestClassifier",
                  summary: "",
                  sizeBytes: null,
                },
              ],
              totalCount: 2,
            };
          case "start_runtime": {
            const contextId = String(arg("contextId") ?? arg("context_id"));
            const language = String(arg("language"));
            const info = {
              runtimeId: `runtime-${language}-${Date.now()}`,
              generation: 1,
              key: { projectId: activeProjectId, contextId, language },
              status: "ready",
              interpreter: language === "r" ? "/opt/R/bin/Rscript" : "/opt/python/bin/python",
              version: language === "r" ? "4.4.1" : "3.11.9",
              processId: 3301,
              startedAtMs: Date.now(),
              lastActivityAtMs: Date.now(),
              residentMemoryBytes: null,
              lastError: null,
            };
            runtimeInfos = runtimeInfos.filter((item) => !(
              item.key.projectId === activeProjectId
              && item.key.contextId === contextId
              && item.key.language === language
            ));
            runtimeInfos.push(info);
            return info;
          }
          case "stop_runtime": {
            const info = runtimeInfos.find((item) =>
              item.key.projectId === String(arg("projectId") ?? arg("project_id"))
              && item.key.contextId === String(arg("contextId") ?? arg("context_id"))
              && item.key.language === String(arg("language"))
            );
            if (info) {
              info.status = "dead";
              info.lastActivityAtMs = Date.now();
              info.processId = null;
            }
            return info ?? null;
          }
          case "restart_runtime": {
            const info = runtimeInfos.find((item) =>
              item.key.projectId === String(arg("projectId") ?? arg("project_id"))
              && item.key.contextId === String(arg("contextId") ?? arg("context_id"))
              && item.key.language === String(arg("language"))
            );
            if (info) {
              info.runtimeId = `runtime-restarted-${Date.now()}`;
              info.generation += 1;
              info.status = "ready";
              info.processId = 4401;
              info.lastActivityAtMs = Date.now();
              info.lastError = null;
            }
            return info ?? null;
          }
          case "import_wsl_contexts":
            return [
              ...executionContexts,
              {
                id: "wsl:Ubuntu-24.04",
                kind: "wsl",
                label: "Ubuntu-24.04",
                config_json: "{\"distro\":\"Ubuntu-24.04\"}",
                capabilities_json: "{}",
                last_probe_at: null,
                last_probe_status: null,
                last_probe_error: null,
                created_at: 1783478400,
                updated_at: 1783478400,
              },
            ];
          case "open_terminal": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "local");
            return {
              id: `terminal-mock-${++terminalCounter}`,
              projectId: activeProjectId,
              contextId,
              title: `${contextId} — Terminal`,
              kind: contextId.startsWith("ssh:") ? "ssh" : "local",
              displayCwd: "/mock/root",
              processId: 1234,
              running: true,
            };
          }
          case "attach_terminal": {
            setTimeout(() => arg("onEvent")?.onmessage?.({
              event: "output",
              data: { base64: btoa("terminal ready\r\n") },
            }), 0);
            return {
              id: String(arg("sessionId") ?? "terminal-mock"),
              projectId: activeProjectId,
              contextId: "ssh:gpu-server",
              title: "ssh:gpu-server — Terminal",
              kind: "ssh",
              displayCwd: "/mock/root",
              processId: 1234,
              running: true,
            };
          }
          case "write_terminal":
          case "resize_terminal":
          case "terminate_terminal":
            return null;
          case "list_runs":
            return runs;
          case "cancel_run": {
            const run = runs.find((r) => r.id === (arg("runId") ?? arg("run_id")));
            if (run) {
              run.status = "cancelled";
              run.ended_at = Math.floor(Date.now() / 1000);
            }
            return run ?? null;
          }
          case "save_model": {
            const profile = plain(arg("profile") ?? {});
            const useForVision = Boolean(arg("useForVision") ?? profile.use_for_vision);
            mockModels = mockModels.map((m) => m.id === profile.id ? {
              ...m,
              ...profile,
              use_for_vision: useForVision,
            } : {
              ...m,
              use_for_vision: useForVision ? false : m.use_for_vision,
            });
            return mockModels;
          }
          case "remove_model":
            return mockModels;
          case "set_active_model": {
            const id = arg("id") ?? "";
            mockModels = mockModels.map((m) => ({ ...m, active: m.id === id }));
            return mockModels;
          }
          case "get_project_info":
            ((window as any).__projectInfoReads ??= []).push(activeProjectId);
            return activeProjectId === "other"
              ? { ...project, id: "other", name: "Other project", root: "/mock/other" }
              : project;
          case "get_project_settings":
            return { name: project.name, description: "", agent_context: "" };
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
          case "list_mcp_connections":
            return { connections: mockMcpConnections };
          case "list_connectors":
            return {
              scope: "ask",
              connectors: [
                {
                  key: "biomart",
                  name: "BioMart",
                  kind: "bundled",
                  enabled: true,
                  skip_approvals: false,
                  transport: "",
                  subtitle: "",
                  tools: [{ name: "biomart_query", mode: "allow", description: "" }],
                },
                {
                  key: "conn-wolai",
                  name: "wolai_cmp",
                  kind: "custom",
                  enabled: true,
                  skip_approvals: false,
                  transport: "http",
                  subtitle: "https://api.wolai.com/v1/mcp/",
                  tools: [],
                },
              ],
            };
          case "list_approval_grants":
            return mockApprovalGrants;
          case "revoke_approval_grant": {
            const scope = String(arg("scope") ?? "");
            const kind = String(arg("kind") ?? "");
            const target = String(arg("target") ?? "");
            mockApprovalGrants = mockApprovalGrants.filter(
              (row) => row.scope !== scope || row.kind !== kind || row.target !== target,
            );
            return null;
          }
          case "revoke_all_approval_grants":
            mockApprovalGrants = [];
            return null;
          case "test_mcp_connection":
            return mockMcpTools;
          case "set_mcp_connection_enabled": {
            const id = arg("id") ?? "";
            const enabled = Boolean(arg("enabled"));
            mockMcpConnections = mockMcpConnections.map((c) => c.id === id ? { ...c, enabled } : c);
            return null;
          }
          case "delete_mcp_connection": {
            const id = arg("id") ?? "";
            mockMcpConnections = mockMcpConnections.filter((c) => c.id !== id);
            return null;
          }
          case "add_mcp_connection":
          case "update_mcp_connection":
          case "set_connector_enabled":
          case "set_tool_approval":
          case "set_approval_scope":
          case "set_connector_skip_approvals":
            return null;
          case "set_credential": {
            const id = String(arg("id") ?? "");
            mockCredentials[id] = String(arg("value") ?? "").trim().length > 0;
            return null;
          }
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
              { name: "config.json", is_dir: false, size: 64 },
              { name: "model.pdb", is_dir: false, size: 256 },
              { name: "sequences.fasta", is_dir: false, size: 256 },
            ];
          case "list_remote_dir": {
            const path = String(arg("path") ?? "~");
            if (path === "/home/research/projects") {
              return {
                path,
                entries: [
                  { name: "rna-seq", is_dir: true, size: 0 },
                  { name: "README.md", is_dir: false, size: 512 },
                ],
              };
            }
            return {
              path: "/home/research",
              entries: [
                { name: "projects", is_dir: true, size: 0 },
                { name: "notes.txt", is_dir: false, size: 128 },
              ],
            };
          }
          case "search_files": {
            const q = String(arg("query") ?? "").toLowerCase();
            const all = [
              { path: "data/report.csv", name: "report.csv", is_dir: false, size: 4096 },
              { path: "counts.csv", name: "counts.csv", is_dir: false, size: 128 },
            ];
            return all.filter((h) => h.name.toLowerCase().includes(q));
          }
          case "search_artifacts": {
            const q = String(arg("query") ?? "").toLowerCase();
            return q ? artifacts.filter((a) => a.name.toLowerCase().includes(q)) : artifacts;
          }
          case "search_sessions": {
            const q = String(arg("query") ?? "").toLowerCase();
            const rows = [
              { id: "s-current", project_id: "default", project_name: "wisp-science", title: "Current analysis", ts: 1, activity_at: 3, status: "complete" },
              { id: "s-old", project_id: "default", project_name: "wisp-science", title: "Older structure run", ts: 1, activity_at: 2, status: "complete" },
              { id: "s-other", project_id: "other", project_name: "Other project", title: "Cross-project counts", ts: 1, activity_at: 1, status: "complete" },
              { id: "s-complete", project_id: "default", project_name: "wisp-science", title: "Enumerate MCP bio-tools databases", ts: 1, activity_at: 1, status: "complete" },
            ];
            return q ? rows.filter((s) => s.title.toLowerCase().includes(q)) : rows;
          }
          case "read_file": {
            const path = String(arg("path") ?? "report.csv");
            if (path.toLowerCase().endsWith(".pdb")) {
              return { path, mime: "chemical/x-pdb", text: "ATOM      1  CA  ALA A   1      11.104  13.207   9.132  1.00 20.00           C\nEND\n", base64: null };
            }
            if (path.toLowerCase().endsWith(".fasta")) {
              return { path, mime: "text/plain", text: ">seq1\nMKTIIALSYIFCLVFADYKDDDDK\n>seq2\nMKTIIALSYIFCLVFADYKDDDDK\n", base64: null };
            }
            if (path.toLowerCase().includes(".pdf")) {
              return { path, mime: "application/pdf", text: null, base64: pdfBase64 };
            }
            if (path.toLowerCase().includes(".png")) {
              return { path, mime: "image/png", text: null, base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=" };
            }
            if (path.toLowerCase().includes(".json")) {
              return { path, mime: "application/json", text: '{"model":{"name":"wisp","enabled":true}}', base64: null };
            }
            if (path.toLowerCase().includes(".html")) {
              return { path, mime: "text/html", text: '<style>#mode::after{content:"Desktop"}@media(max-width:900px){#mode::after{content:"Mobile"}}</style><div id="mode"></div>', base64: null };
            }
            return { path, mime: "text/csv", text: "a,b\n1,2", base64: null };
          }
          case "read_artifact":
            if (arg("id") === "art-html") {
              return { path: "artifact:art-html", mime: "text/html", text: '<style>#mode::after{content:"Desktop"}@media(max-width:900px){#mode::after{content:"Mobile"}}</style><div id="mode"></div>', base64: null };
            }
            if (arg("id") === "art-markdown") {
              return { path: "artifact:art-markdown", mime: "text/markdown", text: "# Differential expression report\n\nRendered Markdown body.", base64: null };
            }
            return { path: `artifact:${arg("id")}`, mime: "text/csv", text: "a,b\n1,2", base64: null };
          case "read_artifact_version":
            if (arg("versionId") === "resource-version-markdown") {
              return {
                path: "artifact-version:resource-version-markdown",
                mime: "text/markdown",
                text: `# Bound report\n\n${Array.from({ length: 120 }, (_, index) => `Scrollable row ${index + 1}`).join("\n\n")}`,
                base64: null,
              };
            }
            throw new Error("Artifact version not found");
          case "missing_files": {
            const paths = Array.isArray(arg("paths")) ? arg("paths") : [];
            return paths.filter((p) => String(p).includes("/.pdf") || String(p).includes("\\.pdf"));
          }
          case "export_session":
            return "/mock/export.zip";
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
          case "set_settings": {
            const next = plain(arg("settings") ?? {});
            mockPetEnabled = Boolean(next.pet_enabled);
            mockPetDirectory = String(next.pet_directory ?? "");
            return null;
          }
          case "set_api_key":
            return null;
          case "check_for_updates":
            if (mockUpdateCheckPending) {
              await new Promise<void>((resolve) => {
                resolveMockUpdateCheck = resolve;
              });
              mockUpdateCheckPending = false;
            }
            return mockUpdateCheck;
          case "validate_settings":
            return "Validated openai with deepseek-v4-pro";
          case "get_memory_view":
            return { enabled: memoryEnabled, today_file: "2026-07-04.md", files: memoryFiles };
          case "set_memory_enabled":
            memoryEnabled = !!args?.enabled;
            return { enabled: memoryEnabled, today_file: "2026-07-04.md", files: memoryFiles };
          case "get_auto_review_enabled":
            return autoReviewEnabled;
          case "set_auto_review_enabled":
            autoReviewEnabled = !!args?.enabled;
            return autoReviewEnabled;
          case "list_memory":
          case "write_memory_file":
          case "delete_memory_file":
          case "clear_memory":
            return memoryFiles;
          case "read_memory_file":
            return "User prefers DeepSeek.\n";
          case "new_session":
            return `s-${Math.random().toString(36).slice(2)}`;
          case "branch_session":
            return `branch-${Math.random().toString(36).slice(2)}`;
          case "side_chat":
            return `Side answer: ${arg("question") ?? ""}`;
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "stop_session":
          case "stop_agent":
            setTimeout(() => {
              const frameId = String(arg("id") ?? arg("sessionId") ?? "");
              emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "cancelled" });
              acpLongResolvers[frameId]?.(frameId);
              delete acpLongResolvers[frameId];
            }, 0);
            return null;
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            const msg = (args && args.message) || "";
            const acpAgentId = args?.acpAgentId ?? acpBindings[fid];
            if (acpAgentId && String(msg).includes("ACPTHINK")) {
              // Codex-style ordering: a short reply streams first, THEN thinking,
              // THEN tool calls. Thinking must fold into the steps panel with the
              // tools, not dangle under the reply.
              acpBindings[fid] = acpAgentId;
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Let me search the literature first." });
                emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Planning which databases to query." });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "s1", title: "web_search", kind: "search", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCallUpdate", payload: { toolCallId: "s1", status: "completed", content: [{ type: "content", content: { type: "text", text: "hit" } }] } });
                emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              return fid;
            }
            if (acpAgentId) {
              acpBindings[fid] = acpAgentId;
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("acp-session-state", {
                  frameId: fid,
                  modes: { currentModeId: "agent", availableModes: [{ id: "read-only", name: "Read Only" }, { id: "agent", name: "Agent" }, { id: "full-access", name: "Full Access" }] },
                  configOptions: [{ id: "model", name: "Model", type: "select", currentValue: "fast", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] }],
                });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "tool-a", title: "Read files", kind: "read", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "tool-b", title: "Run checks", kind: "execute", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCallUpdate", payload: { toolCallId: "tool-a", status: "completed", content: [{ type: "content", content: { type: "text", text: "read complete" } }] } });
                emit("acp-session-update", { frameId: fid, kind: "Plan", payload: { entries: [{ content: "Inspect", priority: "high", status: "completed" }, { content: "Implement", priority: "medium", status: "in_progress" }] } });
                emit("acp-session-update", { frameId: fid, kind: "ConfigOptions", payload: { configOptions: [{ id: "model", name: "Model", type: "select", currentValue: "smart", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] }] } });
                emit("acp-session-update", { frameId: fid, kind: "Usage", payload: { used: 1200, size: 8000 } });
                if (String(msg).includes("PERMISSION")) {
                  acpPermissionFrames["permission-1"] = fid;
                  emit("permission-request", { requestId: "permission-1", frameId: fid, toolCall: { toolCallId: "tool-b", title: "Run checks" }, options: [{ id: "allow", name: "Allow once", kind: "allowonce" }, { id: "reject", name: "Reject", kind: "rejectonce" }] });
                }
                emit("agent", { kind: "Text", frame_id: fid, delta: "Hello from ACP." });
                if (!String(msg).includes("LONG") && !String(msg).includes("PERMISSION")) emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              if (String(msg).includes("LONG")) return await new Promise<string>((resolve) => { acpLongResolvers[fid] = resolve; });
              return fid;
            }
            if (String(msg).includes("PRESTARTFAIL")) {
              throw new Error("No model profile is available");
            }
            if (String(msg).includes("POSTSTARTFAIL")) {
              throw new Error("[turn-started] execution failed after turn/start");
            }
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
            if (String(arg("message") ?? "").includes("NEEDRCONFIRM")) {
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: "R execution requires approval",
                    tool: "r",
                    preview: "[r @ local] summary(dataset)",
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
            if (String(arg("message") ?? "").includes("DELAYUSER")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "delayed reply" });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 1200);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWUNREVIEWABLE")) {
              const incompleteReport = {
                id: "review-auto-unreviewable",
                summary: "Review could not establish full traceability because tool output evidence was incomplete.",
                reviewer_model: "Test ACP Agent",
                reviewer_effort: "",
                reviewer_backend: "acp_agent",
                review_status: "unreviewable",
                evidence_coverage: 0,
                coverage_gaps: ["python analysis.py did not persist inspectable output (only status, location, or terminal handle)."],
                findings: [],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The ACP analysis completed." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: incompleteReport });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWFAIL")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The primary answer still completed." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "ReviewFailed", frame_id: fid, message: "ACP reviewer returned invalid JSON" });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWCLEAN")) {
              const cleanReport = {
                id: "review-auto-clean",
                summary: "No issues found in the response.",
                reviewer_model: "claude-sonnet-5",
                reviewer_effort: "high",
                findings: [],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The analysis is consistent with the tool result." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: cleanReport });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEW")) {
              const openReport = {
                id: "review-auto-1",
                summary: "Checked the reported value against the tool result.",
                reviewer_model: "claude-sonnet-5",
                reviewer_effort: "high",
                findings: [
                  {
                    message_index: 1,
                    claim: "The analysis reports 5 significant genes.",
                    evidence: "The tool result reports 3 significant genes.",
                    fix: "Change the count from 5 to 3.",
                    verdict: "warn",
                    severity: "low",
                    status: "open",
                  },
                ],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The analysis found 5 significant genes." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: openReport });
                emit("agent", { kind: "CorrectionStarted", frame_id: fid, model: "deepseek-v4-pro" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Correction: the analysis found 3 significant genes." });
                emit("agent", {
                  kind: "Review",
                  frame_id: fid,
                  report: {
                    ...openReport,
                    summary: "The corrected value matches the tool result.",
                    findings: openReport.findings.map((finding) => ({ ...finding, status: "resolved" })),
                  },
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("STEPSLIVE")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Inspect the live output." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "long-running-command" });
                }, 30);
                setTimeout(() => {
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: "shell output line" });
                }, 2_500);
                setTimeout(() => {
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "print('next')" });
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: "next output" });
                }, 2_800);
                setTimeout(() => {
                  emit("agent", { kind: "Text", frame_id: fid, delta: "Live steps finished." });
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 3_100);
              });
            }
            if (String(arg("message") ?? "").includes("RNOTEBOOK")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "r", preview: "[r @ ssh:gpu-server] summary(dataset)" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "r", ok: true, content: "Length Class Mode" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "R summary complete." });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            // Multi-tool path (#82): a thinking + tool-call run that must fold
            // into one collapsible "steps" panel instead of a wall of cards.
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
            if (String(arg("message") ?? "").includes("MDLIST")) {
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
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDTABLE")) {
              const md = [
                "| Tissue | TPM |",
                "|---|---:|",
                "| Veg 0DAF | 2.62 |",
                "| Notch 0DAF | 1.81 |",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDCODE")) {
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
          case "list_specialists":
            return mockSpecialists;
          case "save_specialist_cmd": {
            const spec = plain(arg("spec") ?? {});
            if (!spec.id) { spec.id = `sp${mockSpecialists.length}`; spec.builtin = false; }
            mockSpecialists = mockSpecialists.some((s) => s.id === spec.id)
              ? mockSpecialists.map((s) => (s.id === spec.id ? { ...s, ...spec, builtin: s.builtin, instructions: s.builtin ? s.instructions : spec.instructions } : s))
              : [...mockSpecialists, spec];
            return mockSpecialists;
          }
          case "remove_specialist": {
            const id = arg("id");
            if (mockSpecialists.find((s) => s.id === id)?.builtin) throw new Error("Built-in specialists cannot be removed.");
            mockSpecialists = mockSpecialists.filter((s) => s.id !== id);
            return mockSpecialists;
          }
          case "set_session_specialist":
            sessionSpecialists[arg("frameId")] = arg("id");
            return null;
          case "get_session_specialist":
            return mockSpecialists.find((s) => s.id === sessionSpecialists[arg("frameId")]) ?? null;
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
    window: {
      getCurrentWindow: () => ({
        startDragging: async () => {
          (window as any).__petDragStarted = true;
        },
      }),
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
  const folders: { id: string; name: string }[] = [];
  const queues: Record<string, Promise<void>> = {};

  const project = { id: "default", name: "wisp-science", root: "/mock/root", skill_count: 12, mcp_server_count: 8, memory_file_count: 2, has_api_key: true };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        ((window as any).__sendInvokeLog ??= []).push({ cmd, args });
        const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
        switch (cmd) {
          case "list_demos": return [];
          case "load_demo": return { id: "x", title: "x", request: "x", response: "x" };
          case "load_session": return { items: [], next_before_seq: null, user_offset: 0 };
          case "list_sessions": return sessions.slice();
          case "list_sessions_page": return {
            items: sessions.slice(),
            next_cursor: null,
            running_ids: sessions.filter((item: any) => item.running).map((item) => item.id),
          };
          case "list_folders": return folders.slice();
          case "create_folder": {
            const folder = { id: `folder-${folders.length + 1}`, name: String(arg("name") ?? "") };
            folders.push(folder);
            return folder;
          }
          case "rename_folder": {
            const folder = folders.find((entry) => entry.id === arg("id"));
            if (folder) folder.name = String(arg("name") ?? folder.name);
            return null;
          }
          case "delete_folder": {
            const index = folders.findIndex((entry) => entry.id === arg("id"));
            if (index >= 0) folders.splice(index, 1);
            return null;
          }
          case "list_projects":
            return [
              { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
              { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
            ];
          case "list_recent_sessions": return sessions.map((s) => ({
            id: s.id, project_id: "default", title: s.title, ts: s.ts,
            status: "complete",
          }));
          case "pick_directory": return "/mock/root/new-project";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "delete_project": return null;
          case "get_settings": return {
            provider: "openai",
            api_url: "https://api.deepseek.com",
            model: "deepseek-v4-pro",
            label: "deepseek-v4-pro",
            has_api_key: true,
            locale: "en",
            supports_vision: true,
            sync_backend: "relay",
            sync_relay_url: "https://relay.example.test",
            sync_folder: "",
            sync_relay_token: "",
            has_sync_relay_token: true,
          };
          case "get_project_info": return project;
          case "get_onboarding_state": return { show: false, has_api_key: true };
          case "get_capabilities": return { skills: [], mcp_servers: [], memory_files: [], project };
          case "list_approval_grants": return [];
          case "list_dir": return [];
          case "search_files": return [];
          case "search_artifacts": return [];
          case "read_file": return { path: "x", mime: "text/plain", text: "", base64: null };
          case "missing_files": return [];
          case "export_session": return "/mock/export.zip";
          case "upload_file": return { id: "a", name: "x", kind: "text/csv", path: "x", ts: 1 };
          case "new_session": return `s-${Math.random().toString(36).slice(2)}`;
          case "rename_session": {
            const session = sessions.find((entry) => entry.id === arg("id"));
            if (session) session.title = String(arg("title") ?? session.title);
            return null;
          }
          case "delete_session": {
            const index = sessions.findIndex((entry) => entry.id === arg("id"));
            if (index >= 0) sessions.splice(index, 1);
            return null;
          }
          case "move_session": return null;
          case "transfer_session_to_project": {
            if (arg("mode") === "move") {
              const index = sessions.findIndex((entry) => entry.id === arg("id"));
              if (index >= 0) sessions.splice(index, 1);
            }
            return `transferred-${String(arg("id"))}`;
          }
          case "stop_agent":
          case "rewind_session":
          case "revoke_approval_grant":
          case "revoke_all_approval_grants":
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "validate_settings": return "ok";
          case "check_for_updates":
            return {
              current_version: "0.9.0",
              latest_version: "0.9.0",
              update_available: false,
              release_url: "https://github.com/xuzhougeng/wisp-science/releases",
            };
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            const msg = (args && args.message) || "";
            const run = async () => {
              if (!sessions.some((s) => s.id === fid)) {
                sessions.push({ id: fid, title: msg, ts: Date.now() });
              }
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "Text", frame_id: fid, delta: `echo:${msg}` });
              if (msg === "alpha") {
                await new Promise((resolve) => setTimeout(resolve, 1200));
                emit("agent", { kind: "Text", frame_id: fid, delta: ":tail" });
                await new Promise((resolve) => setTimeout(resolve, 3800));
              } else if (msg.startsWith("actions-")) {
                await new Promise((resolve) => setTimeout(resolve, 50));
              } else {
                await new Promise((resolve) => setTimeout(resolve, 5000));
              }
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
