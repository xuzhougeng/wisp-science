// Self-contained mock of the Tauri v2 webview globals. Passed to
// Playwright's `page.addInitScript`, so it runs in the page before the Leptos
// wasm boots and installs `window.__TAURI__` with canned invoke/listen data.
//
// Keep it dependency-free and closure-free: Playwright serializes the function
// source and runs it verbatim in the browser.
export function tauriMock(fixtures?: { xlsxBase64?: string; pptxBase64?: string }): void {
  class Channel {
    onmessage: ((message: any) => void) | null = null;
  }
  const pdfBase64 = "JVBERi0xLjQKJVdpc3AKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUiA0IDAgUl0gL0NvdW50IDIgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNSAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNiAwIFIgPj4KZW5kb2JqCjUgMCBvYmoKPDwgL0xlbmd0aCA0OCA+PgpzdHJlYW0KQlQgL0YxIDI0IFRmIDcyIDcyMCBUZCAoUERGIHByZXZpZXcgd29ya3MpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNiAwIG9iago8PCAvTGVuZ3RoIDQ2ID4+CnN0cmVhbQpCVCAvRjEgMjQgVGYgNzIgNzIwIFRkIChTZWNvbmQgUERGIHBhZ2UpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNyAwIG9iago8PCAvVHlwZSAvRm9udCAvU3VidHlwZSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA4CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAxNSAwMDAwIG4gCjAwMDAwMDAwNjQgMDAwMDAgbiAKMDAwMDAwMDEyNyAwMDAwMCBuIAowMDAwMDAwMjUzIDAwMDAwIG4gCjAwMDAwMDAzNzkgMDAwMDAgbiAKMDAwMDAwMDQ3NyAwMDAwMCBuIAowMDAwMDAwNTczIDAwMDAwIG4gCnRyYWlsZXIKPDwgL1NpemUgOCAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKNjQyCiUlRU9GCg==";
  // Real .docx (pandoc-built) with headings, a table, and OMML equations —
  // exercises the offline docx-preview render path (P3 / #274).
  const docxBase64 = "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";
  const base64Bytes = (value: string) => Uint8Array.from(atob(value), (char) => char.charCodeAt(0));
  const xlsxBase64 = fixtures?.xlsxBase64 ?? "";
  const pptxBase64 = fixtures?.pptxBase64 ?? "";
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try {
      listeners[event]?.({ payload });
    } catch {
      /* listener may not be registered yet */
    }
  };
  (window as any).__tauriEmit = emit;
  // Tests that exercise startup-time native events must wait until the WASM
  // side has completed its async `listen()` registration. Exposing readiness
  // avoids arbitrary sleeps and preserves the real event bus semantics: an
  // event emitted before registration is not queued.
  (window as any).__tauriListenerReady = (event: string) =>
    typeof listeners[String(event)] === "function";

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
  const mockOAuthPending = query.get("mockOAuthPending") === "1";
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
  let resolveMockOAuth: (() => void) | null = null;
  let mockPetEnabled = new URLSearchParams(window.location.search).get("mockPet") === "1";
  let mockPetDirectory = mockPetEnabled ? "C:\\Users\\tester\\.codex\\pets\\wispy" : "";
  (window as any).__petWindowVisible = false;
  let resolveMockUpdateCheck: (() => void) | null = null;
  const syncedProjects = new Set<string>();
  const nextProjectOpenDelayMs: Record<string, number> = {};
  let nextProbeDelayMs = 0;
  let failNextProjectOpenId: string | null = null;
  (window as any).__delayNextProjectOpen = (projectId: string, milliseconds: number) => {
    nextProjectOpenDelayMs[String(projectId)] = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__delayNextProbe = (milliseconds: number) => {
    nextProbeDelayMs = Math.max(0, Number(milliseconds) || 0);
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
  (window as any).__resolveMockOAuth = () => {
    resolveMockOAuth?.();
    resolveMockOAuth = null;
  };
  let skills = [
    { name: "remote-compute-modal", description: "Run jobs on Modal", tags: ["compute"], enabled: true, builtin: true, dir: "/skills/remote-compute-modal" },
    { name: "alphafold2", description: "Predict protein structures", tags: ["protein", "structure"], enabled: true, builtin: true, dir: "/skills/alphafold2" },
    { name: "paper-narrative", description: "Shape a paper story", tags: [], enabled: true, builtin: false, dir: "/home/me/.wisp/skills/paper-narrative" },
  ];
  let memoryEnabled = true;
  let autoReviewEnabled = false;
  const sessionDelegationEnabled: Record<string, boolean> = {};
  let lastDelegationSessionId = "s-current";
  // Mutable workspace fixtures let live FileChanged events prove that open
  // previews re-read content written by an agent tool.
  const workspaceMd: Record<string, string> = {};
  let workspaceR = 'library(Seurat)\nin_dir <- "data"\nplot(1:3)\n';
  (window as any).__setMockWorkspaceR = (value: string) => { workspaceR = String(value); };
  let workspaceEntries = [
    { path: "data", is_dir: true, size: 0 },
    { path: "report.csv", is_dir: false, size: 4096 },
    { path: "config.json", is_dir: false, size: 64 },
    { path: "model.pdb", is_dir: false, size: 256 },
    { path: "sequences.fasta", is_dir: false, size: 256 },
    { path: "analysis.R", is_dir: false, size: 128 },
    { path: "qc.py", is_dir: false, size: 96 },
    { path: "pixi.toml", is_dir: false, size: 64 },
    { path: "analysis.ipynb", is_dir: false, size: 4096 },
    { path: "analysis.unknown", is_dir: false, size: 128 },
    { path: "manuscript.docx", is_dir: false, size: 11351 },
    { path: "office-preview.xlsx", is_dir: false, size: 3600 },
    { path: "office-preview.pptx", is_dir: false, size: 8600 },
  ];
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
  let mockAgentWorkflowCounter = 0;
  let mockAgentWorkflows: any[] = [];
  const mockAgentTemplates = [
    { id: "biology_interpreter", display_name: "Biology interpretation Agent", description: "Interpret biological meaning", role: "analyst", backend: "local", automatic_requires_confirmation: false },
    { id: "code_execution", display_name: "Code execution Agent", description: "Execute project code", role: "coder", backend: "acp", automatic_requires_confirmation: true },
    { id: "reviewer", display_name: "Reviewer Agent", description: "Independently review results", role: "reviewer", backend: "local", automatic_requires_confirmation: false },
    { id: "visualization", display_name: "Visualization Agent", description: "Create scientific figures", role: "coder", backend: "acp", automatic_requires_confirmation: true },
  ];
  const agentWorkflowSnapshot = (goal: string, mode: string, requestedTemplates: string[] = []) => {
    const id = `workflow-${++mockAgentWorkflowCounter}`;
    let templateIds = requestedTemplates;
    if (mode !== "manual") {
      const lower = goal.toLowerCase();
      templateIds = [];
      if (/code|analysis|workflow|代码|分析/.test(lower)) templateIds.push("code_execution");
      if (/biology|gene|pathway|生物|基因|通路/.test(lower)) templateIds.push("biology_interpreter");
      if (/figure|plot|visual|图|可视化/.test(lower)) templateIds.push("visualization");
      if (!templateIds.length) templateIds.push("biology_interpreter");
      templateIds.push("reviewer");
    }
    const requiresConfirmation = mode !== "automatic"
      || templateIds.some((templateId) => ["code_execution", "visualization"].includes(templateId));
    const stepFor = (templateId: string, position: number) => {
      const template = mockAgentTemplates.find((item) => item.id === templateId) ?? mockAgentTemplates[0];
      const acp = template.backend === "acp";
      return {
        id: `${id}:${templateId}`,
        workflow_id: id,
        position,
        agent_id: `${id}:${templateId}`,
        template_id: templateId,
        role: template.role,
        backend: template.backend,
        model: acp ? "acp-test" : null,
        prompt_template: template.description,
        input_schema_json: "{}",
        output_schema_json: "{}",
        input_contract_json: "{}",
        output_contract_json: "{}",
        permissions_json: JSON.stringify({ tools: acp ? ["codex_project_exec", "read_file"] : ["read_file"], paths: ["project://**"], network: false, write: acp }),
        context_policy_json: "{}",
        budget_json: JSON.stringify({ max_tokens: acp ? 32000 : 16000, max_tool_calls: 24 }),
        spec_json: JSON.stringify({ name: template.display_name }),
        timeout_secs: acp ? 900 : 600,
        created_at: 1,
        updated_at: 1,
      };
    };
    return {
      workflow: {
        id,
        project_id: "default",
        workspace_id: project.root,
        frame_id: lastDelegationSessionId,
        name: goal,
        description: "Controlled multi-Agent execution plan",
        goal,
        mode,
        status: "draft",
        max_parallel: mode === "manual" ? 1 : 2,
        requires_confirmation: requiresConfirmation,
        plan_json: "{}",
        version: 1,
        enabled: true,
        approved_at: null,
        created_at: 1,
        updated_at: 1,
      },
      steps: templateIds.map(stepFor),
      attempts: [],
      delegation_enabled: sessionDelegationEnabled[lastDelegationSessionId] ?? false,
    };
  };
  const agentWorkflowAttempt = (snapshot: any, status: string) => ({
    id: "attempt-1",
    workflow_id: snapshot.workflow.id,
    step_id: snapshot.steps[0].id,
    attempt: 1,
    request_id: "request-1",
    backend: "acp",
    status,
    request_json: "{}",
    response_json: status === "running" ? null : "{}",
    output_json: status === "succeeded"
      ? JSON.stringify({ summary: "Analysis and tests completed." })
      : "{}",
    artifact_ids_json: "[]",
    evidence_json: "[]",
    error: null,
    agent_session_id: "agent-session-1",
    child_frame_id: "agent-child-1",
    input_tokens: status === "succeeded" ? 1200 : 0,
    output_tokens: status === "succeeded" ? 300 : 0,
    tool_calls: status === "succeeded" ? 4 : 0,
    cost_microunits: status === "succeeded" ? 25000 : 0,
    cancel_requested: status === "cancelled",
    started_at: 2,
    finished_at: status === "running" ? null : 3,
    created_at: 2,
    updated_at: status === "running" ? 2 : 3,
  });
  const executeMockAgentWorkflow = async (snapshot: any) => {
    snapshot.workflow.status = "running";
    snapshot.attempts = [agentWorkflowAttempt(snapshot, "running")];
    const cancellationDemo = snapshot.workflow.goal.includes("CANCEL DEMO");
    await new Promise((resolve) => setTimeout(resolve, cancellationDemo ? 5_000 : 60));
    if (snapshot.workflow.status === "cancelled") {
      return { workflow_id: snapshot.workflow.id, status: "cancelled", steps: [] };
    }
    snapshot.workflow.status = "succeeded";
    snapshot.workflow.version += 2;
    snapshot.attempts = [agentWorkflowAttempt(snapshot, "succeeded")];
    return { workflow_id: snapshot.workflow.id, status: "succeeded", steps: [] };
  };
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
  let mockCustomCredentials: Array<{
    id: string;
    name: string;
    envVar: string;
    present: boolean;
  }> = [];
  let nextCustomCredential = 1;
  const mockChannels = {
    feishu_enabled: false,
    feishu_bound: false,
    feishu_international: false,
    feishu_app_id: "",
    feishu_has_secret: false,
    feishu_state: "stopped",
    feishu_detail: "",
    weixin_enabled: false,
    weixin_bound: false,
    weixin_state: "stopped",
    weixin_detail: "",
  };
  let mockFeishuPollCount = 0;
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
        auth: "none",
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
      config_json: "{\"alias\":\"gpu-server\"}",
      capabilities_json: "{\"gpu_summary\":\"NVIDIA A100\",\"scheduler\":\"slurm\",\"python_executable\":\"/opt/python/bin/python\",\"rscript_executable\":\"/opt/R/bin/Rscript\",\"r_jsonlite\":true}",
      last_probe_at: 1783482300,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482300,
    },
  ];
  (window as any).__mockExecutionContexts = executionContexts;
  const sessionExecutionContexts: Record<string, string[]> = {};
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
  (window as any).__mockRuns = runs;
  let monitorRunFrameId: string | null = null;
  let resolveMonitorRun: ((frameId: string) => void) | null = null;
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
          case "star_library_text": {
            const sessionId = String(arg("sessionId") ?? "");
            const text = String(arg("text") ?? "");
            const existing = libraryItems.find((item) => item.kind === "text"
              && item.source_session_id === sessionId && item.code === text);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "text",
              title: text.split("\n").find((line) => line.trim())?.trim() ?? "Text",
              language: null,
              code: text,
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
                  text: "[Open bound report](D:/ZZM/03.%20figures/report.md')\n\n[Open bound manuscript](/abs/path/D:/ZZM/paper/manuscript.docx)\n\n[Open bound references](references.bib)",
                  tool_name: null,
                  ok: null,
                  resources: [
                    {
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
                    },
                    {
                      id: "resource-link-docx",
                      ordinal: 1,
                      originalReference: "/abs/path/D:/ZZM/paper/manuscript.docx",
                      artifactId: "resource-artifact-docx",
                      artifactVersionId: "resource-version-docx",
                      displayName: "manuscript.docx",
                      kind: "docx",
                      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                      status: "ready",
                      error: null,
                    },
                    {
                      id: "resource-link-bib",
                      ordinal: 2,
                      originalReference: "references.bib",
                      artifactId: "resource-artifact-bib",
                      artifactVersionId: "resource-version-bib",
                      displayName: "references.bib",
                      kind: "text",
                      mimeType: "text/x-bibtex",
                      status: "ready",
                      error: null,
                    },
                  ],
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
              max_iter: 100,
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
          case "list_agent_templates":
            return mockAgentTemplates;
          case "list_agent_workflows":
            return mockAgentWorkflows;
          case "create_agent_workflow": {
            if (!(sessionDelegationEnabled[lastDelegationSessionId] ?? false)) {
              throw new Error("Sub-Agent delegation is off for this conversation.");
            }
            const mode = String(arg("mode") ?? "assisted");
            if (mode !== "manual") await new Promise((resolve) => setTimeout(resolve, 120));
            const snapshot = agentWorkflowSnapshot(
              String(arg("goal") ?? ""),
              mode,
              Array.isArray(arg("templateIds")) ? arg("templateIds") : [],
            );
            mockAgentWorkflows = [snapshot, ...mockAgentWorkflows];
            if (mode === "automatic" && !snapshot.workflow.requires_confirmation) {
              snapshot.workflow.status = "approved";
              snapshot.workflow.version += 1;
              void executeMockAgentWorkflow(snapshot);
            }
            return snapshot;
          }
          case "revise_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            snapshot.workflow.goal = String(arg("goal") ?? snapshot.workflow.goal);
            snapshot.workflow.name = snapshot.workflow.goal;
            snapshot.workflow.mode = String(arg("mode") ?? snapshot.workflow.mode);
            const replacement = agentWorkflowSnapshot(
              snapshot.workflow.goal,
              snapshot.workflow.mode,
              Array.isArray(arg("templateIds")) ? arg("templateIds") : [],
            );
            snapshot.steps = replacement.steps.map((step: any) => ({
              ...step,
              workflow_id: snapshot.workflow.id,
              id: step.id.replace(replacement.workflow.id, snapshot.workflow.id),
              agent_id: step.agent_id.replace(replacement.workflow.id, snapshot.workflow.id),
            }));
            snapshot.workflow.max_parallel = replacement.workflow.max_parallel;
            snapshot.workflow.requires_confirmation = replacement.workflow.requires_confirmation;
            snapshot.workflow.version += 1;
            if (snapshot.workflow.mode === "automatic" && !snapshot.workflow.requires_confirmation) {
              snapshot.workflow.status = "approved";
              snapshot.workflow.version += 1;
              void executeMockAgentWorkflow(snapshot);
            }
            return snapshot;
          }
          case "approve_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            snapshot.workflow.status = "approved";
            snapshot.workflow.version += 1;
            if (snapshot.workflow.mode === "automatic") void executeMockAgentWorkflow(snapshot);
            return snapshot;
          }
          case "run_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            return executeMockAgentWorkflow(snapshot);
          }
          case "cancel_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            snapshot.workflow.status = "cancelled";
            snapshot.attempts = snapshot.attempts.map((attempt: any) =>
              attempt.status === "running" ? agentWorkflowAttempt(snapshot, "cancelled") : attempt
            );
            return null;
          }
          case "retry_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            snapshot.workflow.status = "approved";
            snapshot.workflow.version += 1;
            if (snapshot.workflow.mode === "automatic") void executeMockAgentWorkflow(snapshot);
            return snapshot;
          }
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
          case "list_custom_credentials":
            return mockCustomCredentials.map((credential) => ({ ...credential }));
          case "channels_status":
            return { ...mockChannels };
          case "set_feishu_channel":
            mockChannels.feishu_enabled = Boolean(arg("enabled"));
            mockChannels.feishu_international = Boolean(arg("international"));
            mockChannels.feishu_app_id = String(arg("appId") ?? "");
            if (String(arg("appSecret") ?? "").trim()) {
              mockChannels.feishu_has_secret = true;
            }
            mockChannels.feishu_bound = Boolean(mockChannels.feishu_app_id && mockChannels.feishu_has_secret);
            mockChannels.feishu_state = mockChannels.feishu_enabled ? "running" : "stopped";
            return null;
          case "feishu_bind_start":
            mockFeishuPollCount = 0;
            return {
              flow_id: "mock-feishu-flow",
              qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="white"/><path d="M1 1h6v6H1zm2 2v2h2V3zM14 1h6v6h-6zm2 2v2h2V3zM1 14h6v6H1zm2 2v2h2v-2zM9 2h2v2H9zm2 3h2v2h-2zM8 8h3v3H8zm5 0h2v2h-2zm3 1h4v2h-4zM9 13h2v2H9zm3-2h2v4h-2zm3 2h2v2h-2zm3 0h2v4h-2zm-9 4h3v3H9zm5-1h3v2h-3zm1 3h5v1h-5z" fill="black"/></svg>'),
              expires_in_seconds: 600,
            };
          case "feishu_bind_poll":
            mockFeishuPollCount += 1;
            if (mockFeishuPollCount === 1) {
              return { state: "pending", retry_after_ms: 500, app_id: "" };
            }
            mockChannels.feishu_bound = true;
            mockChannels.feishu_has_secret = true;
            mockChannels.feishu_app_id = "cli_scan_created";
            mockChannels.feishu_international = Boolean(arg("international") ?? mockChannels.feishu_international);
            return { state: "confirmed", retry_after_ms: 0, app_id: mockChannels.feishu_app_id };
          case "feishu_bind_cancel":
            return null;
          case "feishu_unbind":
            mockChannels.feishu_bound = false;
            mockChannels.feishu_enabled = false;
            mockChannels.feishu_has_secret = false;
            mockChannels.feishu_app_id = "";
            mockChannels.feishu_state = "stopped";
            return null;
          case "set_weixin_channel":
            mockChannels.weixin_enabled = Boolean(arg("enabled"));
            mockChannels.weixin_state = mockChannels.weixin_enabled ? "running" : "stopped";
            return null;
          case "weixin_bind_start":
            return { qrcode: "mock-qr", qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="white"/><path d="M1 1h6v6H1zm2 2v2h2V3zM14 1h6v6h-6zm2 2v2h2V3zM1 14h6v6H1zm2 2v2h2v-2zM9 2h2v2H9zm2 3h2v2h-2zM8 8h3v3H8zm5 0h2v2h-2zm3 1h4v2h-4zM9 13h2v2H9zm3-2h2v4h-2zm3 2h2v2h-2zm3 0h2v4h-2zm-9 4h3v3H9zm5-1h3v2h-3zm1 3h5v1h-5z" fill="black"/></svg>') };
          case "weixin_bind_poll":
            mockChannels.weixin_bound = true;
            return "confirmed";
          case "weixin_unbind":
            mockChannels.weixin_bound = false;
            mockChannels.weixin_enabled = false;
            mockChannels.weixin_state = "stopped";
            return null;
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
          case "list_session_execution_context_ids": {
            const sessionId = String(arg("sessionId") ?? arg("session_id") ?? "");
            return [...(sessionExecutionContexts[sessionId] ?? [])];
          }
          case "set_session_execution_context_enabled": {
            const sessionId = String(arg("sessionId") ?? arg("session_id") ?? "");
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            const context = executionContexts.find((item) => item.id === contextId);
            if (!sessionId || !context || context.kind === "local") {
              throw new Error("Execution context not found");
            }
            const selected = new Set(sessionExecutionContexts[sessionId] ?? []);
            if (Boolean(arg("enabled"))) selected.add(contextId);
            else selected.delete(contextId);
            sessionExecutionContexts[sessionId] = [...selected].sort();
            return [...sessionExecutionContexts[sessionId]];
          }
          case "probe_execution_context": {
            const delay = nextProbeDelayMs;
            nextProbeDelayMs = 0;
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            return executionContexts.find((context) =>
              context.id === String(arg("contextId") ?? arg("context_id"))
            ) ?? null;
          }
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
          case "execute_runtime": {
            // Echo the routing back as console text so a test can assert which
            // runtime the code was sent to, the way the real worker would.
            const code = String(arg("code") ?? "");
            if (code.includes("stop(")) return `[error] ${code}`;
            return `[${arg("language")} @ ${arg("contextId")}] ${code}`;
          }
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
            if (run && monitorRunFrameId) {
              const frameId = monitorRunFrameId;
              setTimeout(() => {
                emit("agent", { kind: "ToolResult", frame_id: frameId, name: "monitor_run", ok: true, content: JSON.stringify(run) });
                emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
                resolveMonitorRun?.(frameId);
                resolveMonitorRun = null;
                monitorRunFrameId = null;
              }, 0);
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
          case "remove_model": {
            const id = arg("id") ?? "";
            mockModels = mockModels.filter((m) => m.id !== id);
            return mockModels;
          }
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
                  auth: "",
                  tools: [{ name: "biomart_query", mode: "allow", description: "" }],
                },
                ...mockMcpConnections.map((connection) => ({
                  key: connection.id,
                  name: connection.name,
                  kind: "custom",
                  enabled: connection.enabled,
                  skip_approvals: false,
                  transport: String(connection.transport?.kind ?? ""),
                  subtitle: connection.transport?.kind === "stdio"
                    ? String(connection.transport?.command ?? "")
                    : String(connection.transport?.url ?? ""),
                  auth: String(connection.transport?.auth ?? "none"),
                  tools: [],
                })),
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
          case "test_oauth_mcp_connection":
            if (mockOAuthPending) {
              await new Promise<void>((resolve) => {
                resolveMockOAuth = resolve;
              });
            }
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
          case "authorize_http_connection": {
            const connection = plain(arg("conn") ?? {});
            mockMcpConnections = [
              ...mockMcpConnections.filter((item) => item.id !== connection.id),
              connection,
            ];
            return null;
          }
          case "set_credential": {
            const id = String(arg("id") ?? "");
            mockCredentials[id] = String(arg("value") ?? "").trim().length > 0;
            mockCustomCredentials = mockCustomCredentials.map((credential) =>
              credential.id === id
                ? { ...credential, present: mockCredentials[id] }
                : credential,
            );
            return null;
          }
          case "add_custom_credential": {
            const credential = {
              id: `custom-${nextCustomCredential++}`,
              name: String(arg("name") ?? "").trim(),
              envVar: String(arg("envVar") ?? "").trim(),
              present: String(arg("value") ?? "").trim().length > 0,
            };
            mockCustomCredentials.push(credential);
            mockCredentials[credential.id] = credential.present;
            return { ...credential };
          }
          case "remove_custom_credential": {
            const id = String(arg("id") ?? "");
            mockCustomCredentials = mockCustomCredentials.filter((credential) => credential.id !== id);
            delete mockCredentials[id];
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
          case "list_dir": {
            const cwd = String(arg("path") ?? ".").replaceAll("\\", "/").replace(/^\.\//, "").replace(/\/$/, "") || ".";
            return workspaceEntries
              .filter((entry) => {
                const split = entry.path.lastIndexOf("/");
                const parent = split < 0 ? "." : entry.path.slice(0, split);
                return parent === cwd;
              })
              .map((entry) => ({
                name: entry.path.slice(entry.path.lastIndexOf("/") + 1),
                is_dir: entry.is_dir,
                size: entry.size,
              }))
              .sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.localeCompare(b.name));
          }
          case "create_file": {
            const path = String(arg("path") ?? "");
            if (workspaceEntries.some((entry) => entry.path === path)) throw new Error(`workspace entry '${path}' already exists`);
            workspaceEntries.push({ path, is_dir: false, size: 0 });
            return null;
          }
          case "create_directory": {
            const path = String(arg("path") ?? "");
            if (workspaceEntries.some((entry) => entry.path === path)) throw new Error(`workspace entry '${path}' already exists`);
            workspaceEntries.push({ path, is_dir: true, size: 0 });
            return null;
          }
          case "rename_entry": {
            const path = String(arg("path") ?? "");
            const newPath = String(arg("newPath") ?? "");
            workspaceEntries = workspaceEntries.map((entry) => entry.path === path || entry.path.startsWith(`${path}/`)
              ? { ...entry, path: `${newPath}${entry.path.slice(path.length)}` }
              : entry);
            return null;
          }
          case "delete_entry": {
            const path = String(arg("path") ?? "");
            workspaceEntries = workspaceEntries.filter((entry) => entry.path !== path && !entry.path.startsWith(`${path}/`));
            return null;
          }
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
            if (path.toLowerCase().endsWith(".r")) {
              // Multi-line on purpose: #307 collapsed a script's newlines into one
              // paragraph, which a single-line fixture cannot catch.
              return { path, mime: "text/x-r", text: workspaceR, base64: null };
            }
            if (path.toLowerCase().endsWith(".py")) {
              return { path, mime: "text/x-python", text: 'import scanpy as sc\nadata = sc.read("counts.h5ad")\n', base64: null };
            }
            if (path.toLowerCase().endsWith(".toml")) {
              return { path, mime: "application/octet-stream", text: '[project]\nname = "x"\n', base64: null };
            }
            if (path.toLowerCase().endsWith(".ipynb")) {
              const text = JSON.stringify({
                metadata: { kernelspec: { language: "python" } },
                cells: [
                  { cell_type: "markdown", source: ["## Saved notebook output\n"] },
                  {
                    cell_type: "code",
                    source: ["display(result)\n"],
                    outputs: [
                      {
                        output_type: "display_data",
                        data: {
                          "text/html": '<style>.saved-table{color:green}</style><table id="saved-table" class="saved-table"><tr><td>safe HTML result</td></tr></table><img id="external-image" src="https://example.invalid/pixel.png" onerror="parent.__notebookPwned=true"><script>parent.__notebookPwned=true</script>',
                        },
                      },
                      {
                        output_type: "display_data",
                        data: {
                          "image/svg+xml": '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><script>parent.__notebookPwned=true</script><rect width="80" height="30" fill="teal"/><text x="8" y="20">SVG result</text></svg>',
                        },
                      },
                      {
                        output_type: "execute_result",
                        data: { "text/latex": "\\frac{a}{b}" },
                      },
                      {
                        output_type: "display_data",
                        data: { "image/png": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=" },
                      },
                    ],
                  },
                ],
              });
              return { path, mime: "application/x-ipynb+json", text, base64: null };
            }
            if (path.toLowerCase().endsWith(".unknown")) {
              return { path, mime: "application/octet-stream", text: null, base64: "AA==" };
            }
            if (path.toLowerCase().includes(".pdf")) {
              return { path, mime: "application/pdf", text: null, base64: pdfBase64 };
            }
            if (path.toLowerCase().includes(".docx")) {
              return { path, mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", text: null, base64: docxBase64 };
            }
            if (path.toLowerCase().includes(".png")) {
              return { path, mime: "image/png", text: null, base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=" };
            }
            if (path.toLowerCase().endsWith(".md")) {
              return { path, mime: "text/markdown", text: workspaceMd[path] ?? "# Draft manuscript\n\nOriginal body paragraph.\n", base64: null };
            }
            if (path.toLowerCase().includes(".json")) {
              return { path, mime: "application/json", text: '{"model":{"name":"wisp","enabled":true}}', base64: null };
            }
            if (path.toLowerCase().includes(".html")) {
              return { path, mime: "text/html", text: '<style>#mode::after{content:"Desktop"}@media(max-width:900px){#mode::after{content:"Mobile"}}</style><div id="mode"></div>', base64: null };
            }
            return { path, mime: "text/csv", text: "a,b\n1,2", base64: null };
          }
          case "read_file_bytes": {
            const path = String(arg("path") ?? "").toLowerCase();
            if (path.includes(".pdf")) return base64Bytes(pdfBase64);
            if (path.includes(".docx")) return base64Bytes(docxBase64);
            if (path.includes(".xlsx") && xlsxBase64) return base64Bytes(xlsxBase64);
            if (path.includes(".pptx") && pptxBase64) return base64Bytes(pptxBase64);
            throw new Error("Binary fixture not found");
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
            if (arg("versionId") === "resource-version-docx") {
              return {
                path: "artifact-version:resource-version-docx",
                mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                text: null,
                base64: docxBase64,
              };
            }
            if (arg("versionId") === "resource-version-bib") {
              return {
                path: "artifact-version:resource-version-bib",
                mime: "text/x-bibtex",
                text: "@article{wisp,\n  title = {Wisp Science}\n}",
                base64: null,
              };
            }
            throw new Error("Artifact version not found");
          case "read_artifact_version_bytes":
            if (arg("versionId") === "resource-version-docx") {
              return base64Bytes(docxBase64);
            }
            throw new Error("Artifact version bytes not found");
          case "missing_files": {
            const paths = Array.isArray(arg("paths")) ? arg("paths") : [];
            return paths.filter((p) => String(p).includes("/.pdf") || String(p).includes("\\.pdf"));
          }
          case "append_review_note": {
            const src = String(arg("sourcePath") ?? "");
            const stem = (src.split(/[\\/]/).pop() ?? "notes").replace(/\.[^.]+$/, "") || "notes";
            return `reviews/${stem}.md`;
          }
          case "write_file": {
            // Persist so a follow-up read_file returns the edited body (proves the
            // inline editor's save + reload cycle end-to-end).
            workspaceMd[String(arg("path") ?? "")] = String(arg("content") ?? "");
            return null;
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
          case "get_session_delegation_enabled":
            return sessionDelegationEnabled[String(arg("sessionId") ?? "")] ?? false;
          case "set_session_delegation_enabled": {
            const sessionId = String(arg("sessionId") ?? "");
            lastDelegationSessionId = sessionId;
            sessionDelegationEnabled[sessionId] = Boolean(arg("enabled"));
            for (const snapshot of mockAgentWorkflows) {
              if (snapshot.workflow.frame_id === sessionId) {
                snapshot.delegation_enabled = sessionDelegationEnabled[sessionId];
              }
            }
            return sessionDelegationEnabled[sessionId];
          }
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
          case "side_chat": {
            const question = String(arg("question") ?? "");
            if (question === "SIDESCROLLTEST") {
              return Array.from(
                { length: 40 },
                (_, index) => `Side answer line ${index + 1}`,
              ).join("\n\n");
            }
            return `Side answer: ${question}`;
          }
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
            if (String(msg).includes("MONITORRUN")) {
              return await new Promise<string>((resolve) => {
                monitorRunFrameId = fid;
                resolveMonitorRun = resolve;
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Attach the existing Run monitor." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "monitor_run", preview: "run-local-002" });
                }, 30);
              });
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
          case "test_reviewer_backend": {
            const reviewer = plain(arg("reviewer") ?? {});
            const config = reviewer.review_backend ?? { kind: "http_model", profile_id: reviewer.model_id ?? "" };
            if (config.kind === "acp_agent") {
              const profile = mockAcpAgents.find((agent) => agent.id === config.profile_id);
              if (!profile) throw new Error("The Reviewer ACP Agent profile no longer exists.");
              return {
                backend: "acp_agent",
                model: profile.label,
                status: "passed",
                summary: "The reported sample count matches the tool output.",
              };
            }
            const profile = mockModels.find((model) => model.id === config.profile_id)
              ?? mockModels.find((model) => model.active)
              ?? mockModels[0];
            return {
              backend: "http_model",
              model: profile?.model ?? profile?.label ?? "default",
              status: "passed",
              summary: "The reported sample count matches the tool output.",
            };
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
          case "create_file":
          case "create_directory":
          case "rename_entry":
          case "delete_entry": return null;
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
