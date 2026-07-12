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
  let activeProjectId = "default";
  let labBenchReady = false;
  let labBenchFailure = false;
  (window as any).__failLabBench = () => {
    labBenchFailure = true;
  };
  const nextProjectOpenDelayMs: Record<string, number> = {};
  let failNextProjectOpenId: string | null = null;
  (window as any).__delayNextProjectOpen = (projectId: string, milliseconds: number) => {
    nextProjectOpenDelayMs[String(projectId)] = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__failNextProjectOpen = (projectId: string) => {
    failNextProjectOpenId = String(projectId);
  };
  let skills = [
    { name: "remote-compute-modal", description: "Run jobs on Modal", tags: ["compute"], enabled: true, builtin: true, dir: "/skills/remote-compute-modal" },
    { name: "alphafold2", description: "Predict protein structures", tags: ["protein", "structure"], enabled: true, builtin: true, dir: "/skills/alphafold2" },
    { name: "paper-narrative", description: "Shape a paper story", tags: [], enabled: true, builtin: false, dir: "/home/me/.wisp/skills/paper-narrative" },
  ];
  let memoryEnabled = true;
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
      capabilities_json: "{\"gpu_summary\":\"NVIDIA A100\",\"scheduler\":\"slurm\"}",
      last_probe_at: 1783482300,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482300,
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

  (window as any).__TAURI__ = {
    core: {
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
          case "list_demos":
            return demos;
          case "load_demo":
            return demo;
          case "load_session":
            return [];
          case "list_sessions":
            ((window as any).__projectSessionRefreshes ??= []).push(activeProjectId);
            return [];
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
              { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
              { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 1, updated_at: 1, running_count: 0, needs_you_count: 0 },
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
            };
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
            return [];
          case "list_execution_contexts":
            return executionContexts;
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
            ];
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
          case "get_lab_bench":
            if (labBenchFailure) {
              throw new Error("Bench ledger unavailable");
            }
            if (!labBenchReady) {
              return { conversation: null, provenance: null, today: [] };
            }
            return {
              conversation: {
                run: { id: "run-wet-1", title: "RNA extraction", status: "running" },
                wet_lab_run: { display_id: "RUN-000231", protocol_revision_id: "PRT-000017@3" },
              },
              provenance: {
                participants: [
                  { id: "part-in", material_unit_id: "MAT-000312", direction: "input", role: "reagent" },
                  { id: "part-out", material_unit_id: "SMP-001842", direction: "output", role: "product" },
                ],
                subject_participants: [{ id: "subject-part-1", subject_id: "MOU-000031", role: "subject", effect: "observed" }],
                deviations: [{ id: "dev-1", description: "Extra lysis buffer", impact: "minor" }],
                raw_evidence: [{ id: "data-1", display_id: "DAT-000882", uri: "s3://instrument/run-1/manifest.json" }],
                observations: [{ id: "obs-1", role: "Qubit measurement" }],
                assessments: [{ id: "assessment-1", verdict: "pass" }],
                closeout: { issues: ["outputs_without_location"], data_evidence_count: 1 },
              },
              today: [{ id: "event-today-1", role: "Run planned" }],
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
            if (String(msg).includes("RNA extraction")) labBenchReady = true;
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
                  modes: { currentModeId: "code", availableModes: [{ id: "code", name: "Code" }] },
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

  const project = { id: "default", name: "wisp-science", root: "/mock/root", skill_count: 12, mcp_server_count: 8, memory_file_count: 2, has_api_key: true };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        ((window as any).__sendInvokeLog ??= []).push({ cmd, args });
        switch (cmd) {
          case "list_demos": return [];
          case "load_demo": return { id: "x", title: "x", request: "x", response: "x" };
          case "load_session": return [];
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
          case "get_settings": return {
            provider: "openai",
            api_url: "https://api.deepseek.com",
            model: "deepseek-v4-pro",
            label: "deepseek-v4-pro",
            has_api_key: true,
            locale: "en",
            supports_vision: true,
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
          case "stop_agent":
          case "rewind_session":
          case "revoke_approval_grant":
          case "revoke_all_approval_grants":
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
              if (msg === "alpha") {
                await new Promise((resolve) => setTimeout(resolve, 1200));
                emit("agent", { kind: "Text", frame_id: fid, delta: ":tail" });
                await new Promise((resolve) => setTimeout(resolve, 3800));
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
