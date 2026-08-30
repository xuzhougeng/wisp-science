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
    { id: "s3", title: "你能做啥", ts: 1719880000, pinned: true },
    { id: "s4", title: "Branch: 换个思路再试一次", ts: 1719885000, branched_from: "s2" },
    { id: "s5", title: "Branch: 再分一支", ts: 1719884000, branched_from: "s4" },
  ];
  const folders = [{ id: "d1", name: "Research" }];
  let libraryItems = [
    {
      id: "library-code-1",
      kind: "code",
      title: "sc.pl.umap(adata, color=\"leiden\")",
      language: "python",
      code: "import scanpy as sc\nsc.pl.umap(adata, color=\"leiden\")",
      content_type: null,
      source_project_id: "default",
      source_project_name: "Mock project",
      source_session_id: "s1",
      source_session_title: "Mock session",
      source_path: null,
      created_at: Math.floor(Date.now() / 1000) - 86400,
    },
  ];
  const libraryVersions = {}; // item id -> appended edit versions (#455)

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
    latest_version: "0.10.0",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases",
    notes: "## What's new\n\n- Sidebar update prompt with changelog\n- Fixed streaming thinking bounce\n- **Breaking:** renamed `foo` to `bar`",
  };
  let mockUpdateCheckEnabled = true;
  // Category = file-name prefix before "--"; plain date files sit in the
  // default ("About you") category.
  let memoryFiles = [
    { name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 },
    { name: "Projects--2026-07-02.md", preview: "Wisp: two-column memory pane in settings.", bytes: 96 },
    { name: "Projects--2026-06-28.md", preview: "Library rerun tracking shipped in #474.", bytes: 88 },
    { name: "Preferences--2026-07-03.md", preview: "Reply in Chinese; keep diffs minimal.", bytes: 72 },
  ];
  let globalMemories = [{ id: "global-memory-existing", content: "Prefer SI units across projects." }];
  let memoryEnabled = true;
  let autoFailureAnalysis = { enabled: false, failure_rate_threshold: 30, minimum_failures: 2 };
  const lastMessageBySession = {};
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
      use_for_image_generation: false,
      use_for_video_generation: false,
    },
  ];
  // Preview fixtures, keyed by extension, so the file-preview kinds (#307:
  // code / toml / notebook) can be exercised without a real workspace.
  const mockFiles = {
    r: [
      "#### ---- Init ---- ####",
      "library(Seurat)",
      'in_dir <- "../data/ifnb_pbmc"  # 输入目录',
      "seu <- CreateSeuratObject(counts = counts, meta.data = metadata)",
      "for (i in 1:10) {",
      "  message(sprintf('step %d', i))",
      "}",
    ].join("\n"),
    py: "import scanpy as sc\n\ndef load(path: str):\n    # 读取 h5ad\n    return sc.read_h5ad(path)\n",
    toml: '[project]\nname = "ifnb-pbmc"\nchannels = ["conda-forge"]\n\n[tasks]\nrun = "Rscript 01-metacell.R"\n',
    html: "<h1>Remote report</h1><p>Rendered straight from the SSH host.</p>",
    ipynb: JSON.stringify({
      metadata: { kernelspec: { language: "python" } },
      cells: [
        { cell_type: "markdown", source: ["## Metacell QC\n", "\n", "Counts per **resolution**.\n"] },
        {
          cell_type: "code",
          source: ["import scanpy as sc\n", "adata = sc.read_h5ad('pbmc.h5ad')\n", "adata"],
          outputs: [
            { output_type: "stream", name: "stdout", text: ["AnnData object with n_obs = 2638\n"] },
            { output_type: "error", ename: "ValueError", evalue: "bad", traceback: ["\u001b[0;31mValueError\u001b[0m: bad input"] },
          ],
        },
      ],
    }),
  };
  // serde-wasm-bindgen hands invoke a Map, not a plain object, so `args?.path`
  // is always undefined — read both shapes.
  function argValue(args, key) {
    return args instanceof Map ? args.get(key) : args?.[key];
  }
  function mockFile(path) {
    const name = String(path ?? "report.csv");
    const ext = name.split(".").pop().toLowerCase();
    if (["png", "jpg", "jpeg", "gif", "webp", "svg"].includes(ext)) {
      // Labeled grid so region-crop accuracy is verifiable by eye.
      const cells = [];
      for (let r = 0; r < 6; r++) {
        for (let c = 0; c < 8; c++) {
          cells.push(`<rect x="${c * 100}" y="${r * 100}" width="100" height="100" fill="${(r + c) % 2 ? "#eef" : "#fff"}" stroke="#99a"/>`);
          cells.push(`<text x="${c * 100 + 50}" y="${r * 100 + 55}" font-size="20" text-anchor="middle" fill="#334">${String.fromCharCode(65 + r)}${c}</text>`);
        }
      }
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600">${cells.join("")}</svg>`;
      return { path: name, mime: "image/svg+xml", text: null, base64: btoa(svg), truncated: false };
    }
    const text = mockFiles[ext];
    if (text !== undefined) return { path: name, mime: "text/plain", text, base64: null, truncated: false };
    return { path: name, mime: "text/csv", text: "gene,score\nFX-cell,0.91", base64: null, truncated: false };
  }
  const mockCredentials = {
    openalex_api_key: false,
    infinisynapse_api_key: false,
    scimaster_api_key: false,
    ncbi_api_key: false,
    ncbi_email: false,
  };
  let mockCustomCredentials = [];
  let nextCustomCredential = 1;
  // frame_id -> enabled execution context ids, mirroring session_execution_contexts.
  const sessionContexts = {};
  // s1 demonstrates an ACP-bound conversation; the others use built-in agents.
  const mockAcpSessions = new Set(["s1"]);
  // frame_id -> built-in plan mode flag (settings key frame_plan_mode:<id>).
  const mockPlanMode = {};
  // Live ACP ask_user requests: request_id -> frame_id.
  const askUserFrames = {};
  // Agent-created SSH trust edges (ssh_trust_edges_v1). One edge whose
  // destination context no longer exists, to exercise the orphan rendering.
  const mockTrustEdges = [
    { source_context_id: "ssh:cpu1", destination_context_id: "ssh:gpu2", destination_target: "researcher@gpu2.lab", destination_port: 22, key_path: ".ssh/wisp-gpu2-ed25519", managed: true, verified_at: 1719900000 },
    { source_context_id: "ssh:gpu2", destination_context_id: "ssh:cpu1", destination_target: "researcher@cpu1.lab", destination_port: null, key_path: null, managed: false, verified_at: 1719800000 },
  ];
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

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, rawArgs) => {
        // serde_wasm_bindgen renders Rust structs as JS objects but `json!`
        // maps as JS Maps, so property access alone silently misses half the
        // call sites.
        const args = rawArgs instanceof Map ? Object.fromEntries(rawArgs) : rawArgs;
        switch (cmd) {
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
          case "list_codex_sessions":
            return (globalThis.__mockCodexSessions ??= [
              { path: "/mock/.codex/sessions/2026/07/01/rollout-a.jsonl", session_id: "codex-a", title: "Fix the renderer crash", cwd: "/mock/project", message_count: 12, last_active_at: 1751340000, state: "new" },
              { path: "/mock/.codex/sessions/2026/07/02/rollout-b.jsonl", session_id: "codex-b", title: "Refactor session store", cwd: "/mock/other", message_count: 5, last_active_at: 1751426400, state: "imported" },
            ]);
          case "list_claude_sessions":
            return (globalThis.__mockClaudeSessions ??= Array.from({ length: 27 }, (_, index) => ({
              path: `/mock/.claude/projects/mock-project/claude-${String(index + 1).padStart(2, "0")}.jsonl`,
              session_id: `claude-${String(index + 1).padStart(2, "0")}`,
              title: `Claude task ${String(index + 1).padStart(2, "0")}`,
              cwd: "/mock/project",
              message_count: index + 2,
              last_active_at: 1752000000 - index,
              state: "new",
            })));
          case "preview_codex_session":
            return [
              { role: "user", text: "Fix the renderer crash\nIt fails after opening a second window." },
              { role: "assistant", text: "I will inspect the renderer lifecycle first." },
            ];
          case "preview_claude_session":
            return [
              { role: "user", text: `Review ${args?.path ?? "this conversation"}` },
              { role: "assistant", text: "I will start with the relevant files." },
            ];
          case "import_codex_sessions": {
            const paths = args?.paths ?? [];
            let imported = 0;
            const syncedPaths = [];
            for (const item of globalThis.__mockCodexSessions ?? []) {
              if (paths.includes(item.path) && item.state !== "imported") {
                item.state = "imported";
                imported += 1;
                syncedPaths.push(item.path);
              }
            }
            return { imported, updated: 0, skipped: paths.length - imported, failed: 0, synced_paths: syncedPaths };
          }
          case "import_claude_sessions": {
            const paths = args?.paths ?? [];
            let imported = 0;
            const syncedPaths = [];
            for (const item of globalThis.__mockClaudeSessions ?? []) {
              if (paths.includes(item.path) && item.state !== "imported") {
                item.state = "imported";
                imported += 1;
                syncedPaths.push(item.path);
              }
            }
            return { imported, updated: 0, skipped: paths.length - imported, failed: 0, synced_paths: syncedPaths };
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
          case "set_session_pinned": {
            const s = sessions.find((x) => x.id === args?.id);
            if (s) s.pinned = !!args?.pinned;
            return null;
          }
          case "list_projects":
            return [{ id: "default", name: project.name, workspace_dir: project.root, session_count: sessions.length, artifact_count: 3, updated_at: 1 }];
          case "list_recent_sessions":
            return sessions.map((s) => ({ id: s.id, project_id: "default", title: s.title, ts: s.ts }));
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: sessions.length, artifact_count: 3, updated_at: 1 };
          case "delete_project":
            return null;
          case "get_acp_session_state": {
            if (!mockAcpSessions.has(args?.frameId)) return null;
            // Matches the real command: `availableModes` only, never
            // `currentModeId`. Swap "plan" for another id to see a compat card.
            return { availableModes: [{ id: "default" }, { id: "plan" }] };
          }
          case "pick_directory":
            return "/Users/mock/Desktop/demo-project";
          case "pick_executable_file":
            return "/mock/picked/Rscript";
          case "load_session":
            return {
              items: [
                { role: "user", text: "查找文献 FX-cell", tool_name: null, ok: null },
                { role: "reasoning", text: "Search PubMed and preprints for FX-cell literature.", tool_name: null, ok: null },
                {
                  role: "plan",
                  text: JSON.stringify({
                    v: 1,
                    source: "acp",
                    entries: [
                      { content: "Search PubMed for FX-cell", status: "completed", priority: "medium" },
                      { content: "Screen the hits and write report.csv", status: "in_progress", priority: "high" },
                      { content: "Summarize the findings", status: "pending", priority: "low" },
                    ],
                  }),
                  tool_name: null,
                  ok: null,
                },
                { role: "tool", text: "12 hits written to report.csv", tool_name: "python", ok: true },
                { role: "tool", text: "", tool_name: "monitor_run", ok: true, input: "r1" },
                { role: "tool", text: "", tool_name: "monitor_run", ok: true, input: "r2" },
                {
                  role: "assistant",
                  text: "## FX-cell literature\n\n| gene | score |\n| --- | --- |\n| FX-cell | 0.91 |\n\nThe score follows $s = \\frac{1}{1 + e^{-x}}$ and GPT-style \\(a_i^2 + b_i^2\\) too.\n\n$$\\int_0^1 x^2 \\, dx = \\frac{1}{3}$$\n\nSee `report.csv` or {{artifact:00000001}}.\n\nFigures and tables:\n\n- `results/volcano_plot.png`\n- `results/heatmap.png`\n- `results/umap.png`\n- `results/pca.svg`\n- `results/qc_violin.jpg`\n- `results/marker_dotplot.png`\n- `results/deseq2_results.tsv`\n- `results/gsea.pdf`\n- `results/summary.md`\n- `results/counts.csv`\n\n```python\nimport pandas as pd\ndf = pd.read_csv('report.csv')\nprint(df.head())\n```",
                  tool_name: null,
                  ok: null,
                },
              ],
              next_before_seq: null,
              user_offset: 0,
            };
          case "list_demos":
            return [
              { id: "manifest_esr1_01_datasets", title: "Help me find RNA-seq knockdown datasets involving ESR1" },
              { id: "manifest_esr1_02_samples", title: "What specific samples are included in GSE153250" },
              { id: "manifest_esr1_03_rnaseq", title: "Connect to the remote compute host, locate the FASTQ data for GSE153250" },
              { id: "manifest_esr1_04_downstream", title: "Based on the upstream Counts data from GSE153250, perform transcriptome" },
              { id: "manifest_esr1_05_hypotheses", title: "Based on the Counts data from our study, along with the differential e" },
            ];
          case "load_demo":
            return {
              id: "manifest_esr1_03_rnaseq",
              title: "ESR1 RNA-seq",
              request: "Connect to the remote compute host, locate the FASTQ data for GSE153250, keep only the siESR1 and siNT groups.",
              response: "## GSE153250 RNA-seq Upstream Analysis — Complete\n\nKept 12 samples: 6 siNT + 6 siESR1.",
              thinking: "Identify sample groups, download FASTQs, run the upstream pipeline.",
              items: [
                {
                  role: "user",
                  text: "Connect to the remote compute host, locate the FASTQ data for GSE153250, keep only the siESR1 and siNT groups.",
                  tool_name: null,
                  ok: null,
                  input: "",
                },
                {
                  role: "tool",
                  text: JSON.stringify({
                    id: "demo-run-001",
                    frame_id: null,
                    context_id: "ssh:remote-host",
                    title: "Re-run pipeline with fixed STAR index",
                    kind: "ssh_direct",
                    status: "succeeded",
                    command: "cd ~/workspace/GSE153250 && bash pipeline.sh",
                    created_at: 1700000000,
                    started_at: 1700000001,
                    ended_at: 1700000120,
                    exit_code: 0,
                    stdout_tail: "Pipeline finished: 38606 genes, 12 samples",
                    stderr_tail: "",
                    remote_workdir: null,
                    timeout_secs: null,
                    last_polled_at: 1700000120,
                    last_poll_error: null,
                    progress_json: "{}",
                    env_snapshot_json: "{}",
                  }),
                  tool_name: "monitor_run",
                  ok: true,
                  input: "demo-run-001",
                },
                {
                  role: "assistant",
                  text: "## GSE153250 RNA-seq Upstream Analysis — Complete\n\nKept 12 samples: 6 siNT + 6 siESR1.",
                  tool_name: null,
                  ok: null,
                  input: "",
                },
              ],
            };
          case "get_settings":
            return { provider: "openai", api_url: "https://api.deepseek.com", model: "deepseek-v4-pro", label: "deepseek-v4-pro", has_api_key: true, locale: "en", max_iter: 100, auto_compact: true, max_tokens: 4096, reasoning_effort: "", supports_vision: true };
          case "list_models":
            return mockModels;
          case "get_storage_usage":
            return {
              data_dir: "C:\\mock\\AppData\\wisp-science",
              projects: [
                { id: "default", name: "wisp-science", path: "C:\\mock\\wisp-science", bytes: 96 * 1024 * 1024 },
              ],
              entries: [
                { key: "database", bytes: 23 * 1024 * 1024 },
                { key: "python", bytes: 428 * 1024 * 1024 },
                { key: "plugins", bytes: 5632 * 1024 },
                { key: "workspace", bytes: 96 * 1024 * 1024 },
                { key: "other", bytes: 300 * 1024 },
              ],
              total_bytes: (23 + 428 + 96) * 1024 * 1024 + 5632 * 1024 + 300 * 1024,
            };
          case "get_token_usage":
            return [
              { id: "s1", title: "查找文献, FX-cell", updated_at: 1719900000, input: 355570, output: 5548, reasoning: 0, cached: 252928 },
              { id: "s2", title: "我确认下你你有什么skill", updated_at: 1719890000, input: 131917, output: 1462, reasoning: 319, cached: 101888 },
              { id: "s3", title: "你能做啥", updated_at: 1719880000, input: 5085, output: 428, reasoning: 0, cached: 0 },
            ];
          case "credential_status":
            return Object.entries(mockCredentials);
          case "list_custom_credentials":
            return mockCustomCredentials.map((credential) => ({ ...credential }));
          case "channels_status":
            return { ...mockChannels };
          case "set_feishu_channel":
            mockChannels.feishu_enabled = !!args?.enabled;
            mockChannels.feishu_international = !!args?.international;
            mockChannels.feishu_app_id = args?.appId ?? "";
            if (args?.appSecret) mockChannels.feishu_has_secret = true;
            mockChannels.feishu_bound = !!(mockChannels.feishu_app_id && mockChannels.feishu_has_secret);
            mockChannels.feishu_state = mockChannels.feishu_enabled ? "running" : "stopped";
            return null;
          case "feishu_bind_start":
            return {
              flow_id: "mock-feishu-flow",
              qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="220" height="220"><rect width="220" height="220" fill="#3370ff"/></svg>'),
              expires_in_seconds: 600,
            };
          case "feishu_bind_poll":
            mockChannels.feishu_bound = true;
            mockChannels.feishu_has_secret = true;
            mockChannels.feishu_app_id = "cli_scan_created";
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
          case "save_model": {
            // Object.fromEntries above is shallow; the nested profile is still a Map.
            const raw = args?.profile;
            const p = raw instanceof Map ? Object.fromEntries(raw) : raw;
            const useForImageGeneration = Boolean(
              (args instanceof Map
                ? args.get("useForImageGeneration")
                : args?.useForImageGeneration) ?? p?.use_for_image_generation,
            );
            const useForVideoGeneration = Boolean(
              (args instanceof Map
                ? args.get("useForVideoGeneration")
                : args?.useForVideoGeneration) ?? p?.use_for_video_generation,
            );
            for (const model of mockModels) {
              if (model.id === p?.id) {
                Object.assign(model, p, {
                  use_for_image_generation: useForImageGeneration,
                  use_for_video_generation: useForVideoGeneration,
                });
              } else {
                if (useForImageGeneration) model.use_for_image_generation = false;
                if (useForVideoGeneration) model.use_for_video_generation = false;
              }
            }
            return mockModels;
          }
          case "reorder_models": {
            const raw = args?.ids;
            const ids = raw instanceof Map ? Array.from(raw.values()) : (raw ?? []);
            mockModels.sort((a, b) => {
              const ai = ids.indexOf(a.id), bi = ids.indexOf(b.id);
              return (ai < 0 ? Infinity : ai) - (bi < 0 ? Infinity : bi);
            });
            return mockModels;
          }
          case "remove_model":
          case "set_active_model":
            return mockModels;
          case "get_project_info":
            return project;
          case "get_onboarding_state":
            return { show: false, has_api_key: true };
          case "get_bootstrap_status":
            return { skills_loaded: 66, python_ok: true, mcp_catalog: 24, uv_ok: true, node_ok: true, npm_ok: true, sci_ok: true, pixi_ok: true, app_version: "0.4.0-mock", os: "windows", arch: "x86_64", startup: "total=120ms store=90ms window_ready=600000ms", workspace: project.root, errors: [] };
          case "get_capabilities":
            return {
              skills: [{ name: "bear-support", description: "Find papers supporting a claim." }],
              mcp_servers: ["mcp_pubmed"],
              memory_files: [],
              project,
              skill_counts: { bundled: 1, project: 0 },
              mcp_counts: { bundled: 1, project: 0 },
            };
          case "search_artifacts": {
            const q = String(args?.query ?? "").toLowerCase();
            return [
              { id: "u1", name: "counts.csv", kind: "text/csv", path: project.root + "/uploads/counts.csv", ts: 1719900000, project_id: "default", project_name: project.name, session_id: "s1", session_title: "查找文献, FX-cell", size_bytes: 2048, origin: "upload" },
              { id: "u2", name: "samples.xlsx", kind: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", path: project.root + "/uploads/samples.xlsx", ts: 1719890000, project_id: "default", project_name: project.name, session_id: "s1", session_title: "查找文献, FX-cell", size_bytes: 8192, origin: "upload" },
              { id: "a1", name: "report.csv", kind: "text/csv", path: project.root + "/report.csv", ts: 1719880000, project_id: "default", project_name: project.name, session_id: "s1", session_title: "查找文献, FX-cell", size_bytes: 4096, origin: "output" },
            ].filter((a) => !q || a.name.toLowerCase().includes(q));
          }
          // Registered in the DB, never mentioned in chat text — the rows the
          // transcript scan cannot see (harvest / delegation / MCP bridge).
          case "list_artifacts":
            return [
              { id: "h1", name: "deseq2_results.tsv", kind: "table", path: project.root + "/results/deseq2_results.tsv", ts: 1719870000 },
              { id: "h2", name: "volcano_plot.png", kind: "figure", path: project.root + "/results/volcano_plot.png", ts: 1719870100 },
              { id: "h3", name: "out.bam", kind: "data", path: "ssh://cpu1/scratch/out.bam", ts: 1719870200 },
            ];
          case "list_runs":
            return [
              {
                id: "r1", project_id: "default", frame_id: "s1", context_id: "local",
                title: "DESeq2 differential expression", kind: "script", status: "succeeded",
                command: "Rscript 02-deseq2.R", script_path: null,
                input_refs_json: "[]", output_specs_json: "[]",
                created_at: 1719869000, started_at: 1719869100, ended_at: 1719870200, exit_code: 0,
                stdout_tail: "converged in 4 iterations\n2381 genes at padj < 0.05",
                stderr_tail: null, remote_workdir: null, remote_handle_json: null,
                timeout_secs: null, last_polled_at: 1719870200, last_poll_error: null,
                progress_json: "",
                env_snapshot_json: JSON.stringify({
                  context_id: "local",
                  config: { rscript_executable: "/usr/bin/Rscript" },
                  capabilities: { r_jsonlite: true, rscript_executable: "/usr/bin/Rscript" },
                }),
              },
              {
                id: "r2", project_id: "default", frame_id: "s1", context_id: "ssh:cpu1",
                title: "STAR alignment", kind: "command", status: "running",
                command: "STAR --runThreadN 16 --genomeDir /ref/hg38",
                // Relative to now, so the running card shows a plausible elapsed time.
                created_at: Math.floor(Date.now() / 1000) - 95,
                started_at: Math.floor(Date.now() / 1000) - 90,
                ended_at: null, exit_code: null,
                stdout_tail: "..... started mapping\nMar 12 loading genome",
                stderr_tail: null, remote_workdir: "/scratch/wisp/r2",
                timeout_secs: 14400, last_poll_error: null,
                progress_json: "",
                env_snapshot_json: JSON.stringify({
                  context_id: "ssh:cpu1",
                  config: { host: "cpu1", python_executable: "/usr/bin/python3" },
                  capabilities: { python_executable: "/usr/bin/python3", r_jsonlite: false },
                }),
              },
            ];
          case "list_ssh_hosts":
            return [{ alias: "cpu1", user: "researcher", port: 22, identity_file: null, notes: "Mock CPU host" }];
          case "list_ssh_trust_edges":
            return [...mockTrustEdges];
          case "revoke_ssh_trust_edge": {
            const src = String(argValue(args, "sourceContextId") ?? "");
            const dst = String(argValue(args, "destinationContextId") ?? "");
            const index = mockTrustEdges.findIndex((edge) => edge.source_context_id === src && edge.destination_context_id === dst);
            let cleanup_error = null;
            if (index >= 0) {
              cleanup_error = mockTrustEdges[index].managed && dst === "ssh:gpu2"
                ? "destination: execution context no longer exists: ssh:gpu2"
                : null;
              mockTrustEdges.splice(index, 1);
            }
            return { edges: [...mockTrustEdges], cleanup_error };
          }
          case "list_execution_contexts":
            return [
              { id: "local", kind: "local", label: "Local", config_json: "{}", capabilities_json: "{}", last_probe_at: null, last_probe_status: null, last_probe_error: null, created_at: 1, updated_at: 1 },
              {
                id: "ssh:cpu1", kind: "ssh", label: "CPU1",
                config_json: JSON.stringify({ host: "cpu1", python_executable: "/usr/bin/python3", rscript_executable: "/usr/bin/Rscript" }),
                capabilities_json: JSON.stringify({ python_executable: "/usr/bin/python3", rscript_executable: "/usr/bin/Rscript", r_jsonlite: true }),
                last_probe_at: 1, last_probe_status: "ok", last_probe_error: null, created_at: 1, updated_at: 1,
              },
            ];
          case "list_session_execution_context_ids":
            return [...(sessionContexts[String(args?.sessionId ?? args?.session_id ?? "")] ?? [])];
          case "set_session_execution_context_enabled": {
            const sid = String(args?.sessionId ?? args?.session_id ?? "");
            const cid = String(args?.contextId ?? args?.context_id ?? "");
            const on = new Set(sessionContexts[sid] ?? []);
            if (args?.enabled) on.add(cid); else on.delete(cid);
            sessionContexts[sid] = [...on].sort();
            return sessionContexts[sid];
          }
          case "list_dir":
            return [
              { name: "data", is_dir: true, size: 0 },
              { name: "volcano_plot.png", is_dir: false, size: 51200 },
              { name: "report.csv", is_dir: false, size: 4096 },
              { name: "01-metacell.R", is_dir: false, size: 2048 },
              { name: "run.py", is_dir: false, size: 1024 },
              { name: "pixi.toml", is_dir: false, size: 512 },
              { name: "analysis.ipynb", is_dir: false, size: 8192 },
            ];
          case "read_file":
            return mockFile(argValue(args, "path"));
          case "read_file_bytes": {
            // media_url path: mock images are small SVGs, so plain UTF-8
            // bytes are enough for the blob object URL pipeline.
            const file = mockFile(argValue(args, "path"));
            if (!file.base64) throw new Error("mock binary read is image-only");
            const binary = atob(file.base64);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            return bytes;
          }
          case "read_artifact_version_bytes":
          case "read_artifact_bytes": {
            const file = mockFile(argValue(args, "path") ?? "volcano_plot.png");
            if (!file.base64) throw new Error("mock binary read is image-only");
            const binary = atob(file.base64);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            return bytes;
          }
          case "read_artifact": {
            const names = { h1: "deseq2_results.tsv", h2: "volcano_plot.png", h3: "out.bam" };
            return mockFile(names[String(argValue(args, "id") ?? "")] ?? "report.csv");
          }
          case "upload_file": {
            const filename = String(args?.filename ?? "upload.png");
            return {
              id: "up-" + Date.now(),
              name: filename,
              kind: "image/png",
              path: project.root + "/uploads/" + filename,
              ts: Math.floor(Date.now() / 1000),
              project_id: "default",
              project_name: project.name,
              session_id: "s1",
              session_title: "查找文献, FX-cell",
              size_bytes: 1024,
              origin: "upload",
            };
          }
          case "list_remote_dir":
            return {
              path: String(argValue(args, "path") ?? "~") === "~" ? "/home/researcher" : String(argValue(args, "path")),
              entries: [
                { name: "results", is_dir: true, size: 0 },
                { name: "report.html", is_dir: false, size: 3072 },
                { name: "analysis.ipynb", is_dir: false, size: 8192 },
                { name: "01-metacell.R", is_dir: false, size: 2048 },
                { name: "run.py", is_dir: false, size: 1024 },
              ],
            };
          case "read_remote_file":
            return mockFile(argValue(args, "path"));
          case "execute_runtime": {
            const code = String(argValue(args, "code") ?? "");
            const lang = String(argValue(args, "language") ?? "");
            const ctx = String(argValue(args, "contextId") ?? "");
            if (code.includes("stop(") || code.includes("raise ")) {
              return `[error] simulated failure in ${lang} @ ${ctx}`;
            }
            return `[${lang} @ ${ctx}] executed ${code.split("\n").length} line(s)\n${
              code.split("\n").map((l, i) => `[${i + 1}] ${l}`).join("\n")
            }`;
          }
          case "set_settings":
          case "new_session":
            return `s-${Math.random().toString(36).slice(2)}`;
          case "set_credential": {
            const id = String(argValue(args, "id") ?? "");
            mockCredentials[id] = String(argValue(args, "value") ?? "").trim().length > 0;
            mockCustomCredentials = mockCustomCredentials.map((credential) =>
              credential.id === id ? { ...credential, present: mockCredentials[id] } : credential,
            );
            return null;
          }
          case "add_custom_credential": {
            const credential = {
              id: `custom-${nextCustomCredential++}`,
              name: String(argValue(args, "name") ?? "").trim(),
              envVar: String(argValue(args, "envVar") ?? "").trim(),
              present: String(argValue(args, "value") ?? "").trim().length > 0,
            };
            mockCustomCredentials.push(credential);
            mockCredentials[credential.id] = credential.present;
            return { ...credential };
          }
          case "remove_custom_credential": {
            const id = String(argValue(args, "id") ?? "");
            mockCustomCredentials = mockCustomCredentials.filter((credential) => credential.id !== id);
            delete mockCredentials[id];
            return null;
          }
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
          case "get_update_check_enabled":
            return mockUpdateCheckEnabled;
          case "set_update_check_enabled":
            mockUpdateCheckEnabled = !!args?.enabled;
            return mockUpdateCheckEnabled;
          case "validate_settings":
            return "Validated openai with deepseek-v4-pro";
          case "get_memory_view":
            return { enabled: memoryEnabled, project_id: "default", project_name: "wisp-science", today_file: "2026-07-04.md", files: memoryFiles, global_memories: globalMemories };
          case "set_memory_enabled":
            memoryEnabled = !!args?.enabled;
            return { enabled: memoryEnabled, project_id: "default", project_name: "wisp-science", today_file: "2026-07-04.md", files: memoryFiles, global_memories: globalMemories };
          case "get_auto_failure_analysis_settings":
            return { ...autoFailureAnalysis };
          case "set_auto_failure_analysis_settings":
            autoFailureAnalysis = { ...autoFailureAnalysis, ...(args?.settings ?? {}) };
            return { ...autoFailureAnalysis };
          case "propose_turn_memory": {
            const sessionId = String(args?.sessionId ?? "");
            const explicit = /记住|remember/i.test(lastMessageBySession[sessionId] ?? "");
            if (args?.automatic && !explicit) return null;
            return {
              session_id: sessionId,
              turn_index: Number(args?.turnIndex ?? 0),
              scope: explicit ? "global" : "project",
              content: explicit ? "Prefer concise answers." : "Keep the verified project convention from this turn.",
              trigger: explicit ? "explicit" : "manual",
              tool_calls: 0,
              failed_tool_calls: 0,
              failure_rate: 0,
              global_memories: globalMemories,
            };
          }
          case "confirm_turn_memory":
            if (args?.scope === "global") {
              const existing = globalMemories.find((memory) => memory.id === String(args?.replaceId ?? ""));
              if (existing) existing.content = String(args?.content ?? "");
              else globalMemories.push({ id: `global-memory-${globalMemories.length + 1}`, content: String(args?.content ?? "") });
            }
            return { id: args?.scope === "global" ? (args?.replaceId || "global-memory-preview") : null, scope: args?.scope ?? "project" };
          case "update_global_memory": {
            const existing = globalMemories.find((memory) => memory.id === String(args?.id ?? ""));
            if (existing) existing.content = String(args?.content ?? "");
            return null;
          }
          case "delete_global_memory":
            globalMemories = globalMemories.filter((memory) => memory.id !== String(args?.id ?? ""));
            return null;
          case "write_memory_file": {
            const existing = memoryFiles.find((f) => f.name === args?.name);
            const content = args?.content ?? "";
            if (existing) {
              existing.preview = content.slice(0, 240);
              existing.bytes = content.length;
            } else if (args?.name) {
              memoryFiles.push({ name: args.name, preview: content.slice(0, 240), bytes: content.length });
            }
            return memoryFiles;
          }
          case "delete_memory_file":
            memoryFiles = memoryFiles.filter((f) => f.name !== args?.name);
            return memoryFiles;
          case "clear_memory":
            memoryFiles = [];
            return memoryFiles;
          case "read_memory_file":
            return memoryFiles.find((f) => f.name === args?.name)?.preview ?? "";
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "mock-frame";
            const msg = (args && args.message) || "";
            lastMessageBySession[fid] = msg;
            // Mirror enable_referenced_contexts: an @-referenced non-local
            // server turns itself on for the session.
            const on = new Set(sessionContexts[fid] ?? []);
            for (const ref of args?.references ?? []) {
              const cid = ref.kind === "context" ? ref.id : ref.kind === "runtime" ? ref.context_id : null;
              if (cid && cid !== "local") on.add(cid);
            }
            sessionContexts[fid] = [...on].sort();
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
            if (String(msg).includes("ASKUSER")) {
              // Both question sources from one dev trigger: the built-in tool
              // result card, then the ACP bridge's blocking request card.
              setTimeout(() => {
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "ask_user", preview: "Which aligner?" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "ask_user", ok: true, content: JSON.stringify({
                  v: 1, source: "native", question: "Which aligner should the pipeline use?", allow_freeform: true,
                  options: [{ label: "STAR", description: "splice-aware, needs more RAM" }, { label: "HISAT2", description: "lighter" }],
                  note: "Question submitted; end your turn.",
                }) });
                emit("agent", { kind: "Done", frame_id: fid });
                askUserFrames["ask-1"] = fid;
                emit("ask-user-request", { requestId: "ask-1", frameId: fid, payload: {
                  v: 1, source: "acp", question: "Overwrite the existing results directory?", allow_freeform: false,
                  options: [{ label: "Overwrite", description: "the previous run is lost" }, { label: "Write to a new directory" }],
                } });
              }, 80);
              return fid;
            }
            setTimeout(() => {
              emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Searching literature…" });
              emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "scimaster-cli search FX-cell" });
              emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: "12 hits" });
              emit("agent", { kind: "Text", frame_id: fid, delta: "Mock reply for: " + (args?.message ?? "") });
              emit("agent", { kind: "Usage", frame_id: fid, round: 1, input: 19800, output: 300, reasoning: 120, ctx_tokens: 12000, max_context: 1000000 });
              emit("agent", { kind: "Done", frame_id: fid });
            }, 80);
            return fid;
          }
          case "open_external_url":
            if (args?.url) window.open(args.url, "_blank", "noopener,noreferrer");
            return null;
          case "open_browser_extension_page":
            return { extension_path: "/mock/wisp/browser-extension", opened: false };
          case "browser_extension_status":
            return window.__browserExtensionStatus ?? {
              connected: false,
              current_version: null,
              bundled_version: "0.3.1",
              current_protocol: 0,
              required_protocol: 2,
              update_required: false,
              automatic_reload_available: false,
              extension_path: "/mock/wisp/browser-extension",
              extension_path_verified: true,
              integrity_verified: true,
              error: null,
            };
          case "update_browser_extension":
            return window.__browserExtensionUpdateResult ?? {
              outcome: "manual_reload_required",
              status: window.__browserExtensionStatus,
              opened: true,
              error: null,
            };
          case "extension_connected":
            return Boolean(window.__extensionConnected);
          case "list_library_items":
            return libraryItems;
          case "get_research_graph":
            return {
              nodes: [
                { id: "d1", project_id: "p1", kind: "decision", title: "Use DESeq2 over edgeR for the DE call", ref_id: null, metadata_json: JSON.stringify({ rationale: "Shrinkage estimator handles the 3-replicate design better", alternatives: "edgeR, limma-voom" }), created_at: 0, updated_at: 0 },
                { id: "p1", project_id: "p1", kind: "paper", title: "Love et al. 2014, Moderated estimation of fold change", ref_id: "10.1186/s13059-014-0550-8", metadata_json: JSON.stringify({ journal: "Genome Biology", year: 2014 }), created_at: 0, updated_at: 0 },
                { id: "a1", project_id: "p1", kind: "data_asset", title: "counts.tsv", ref_id: "data/counts.tsv", metadata_json: JSON.stringify({ rows: 24567, columns: 9 }), created_at: 0, updated_at: 0 },
                // run:/artifact: nodes + "produced" edges are what a run card reads.
                { id: "run:r1", project_id: "p1", kind: "run", title: "DESeq2 differential expression", ref_id: "r1", metadata_json: "{}", created_at: 0, updated_at: 0 },
                { id: "artifact:h1", project_id: "p1", kind: "artifact", title: "deseq2_results.tsv", ref_id: "h1", metadata_json: "{}", created_at: 0, updated_at: 0 },
                { id: "artifact:h2", project_id: "p1", kind: "artifact", title: "volcano_plot.png", ref_id: "h2", metadata_json: "{}", created_at: 0, updated_at: 0 },
              ],
              edges: [
                { id: "e1", project_id: "p1", source_id: "d1", target_id: "p1", relation: "cites", metadata_json: "{}", created_at: 0 },
                { id: "e2", project_id: "p1", source_id: "d1", target_id: "a1", relation: "applies to", metadata_json: "{}", created_at: 0 },
                { id: "run-artifact:r1:h1", project_id: "p1", source_id: "run:r1", target_id: "artifact:h1", relation: "produced", metadata_json: "{}", created_at: 0 },
                { id: "run-artifact:r1:h2", project_id: "p1", source_id: "run:r1", target_id: "artifact:h2", relation: "produced", metadata_json: "{}", created_at: 0 },
              ],
            };
          case "star_library_text": {
            const text = String(args?.text ?? "");
            const existing = libraryItems.find(
              (item) => item.kind === "text" && item.source_session_id === args?.sessionId && item.code === text,
            );
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "text",
              title: text.split("\n").find((line) => line.trim())?.trim() ?? "Text",
              language: null,
              code: text,
              content_type: null,
              source_project_id: "default",
              source_project_name: project.name,
              source_session_id: String(args?.sessionId ?? ""),
              source_session_title: "Mock session",
              source_path: null,
              created_at: Math.floor(Date.now() / 1000),
            };
            libraryItems.unshift(item);
            return item;
          }
          case "delete_library_item": {
            const before = libraryItems.length;
            libraryItems = libraryItems.filter((item) => item.id !== args?.id);
            delete libraryVersions[args?.id];
            return libraryItems.length !== before;
          }
          case "get_artifact_provenance":
            return {
              code: "import pandas as pd\ndf = search_fx_cell()\ndf.to_csv(\"report.csv\", index=False)",
              language: "python",
              output: "12 hits written to report.csv",
              exit_status: "ok",
              inputs: [],
              env: null,
            };
          case "get_library_item": {
            const item = libraryItems.find((entry) => entry.id === args?.id);
            return item ? { ...item, base64: null } : null;
          }
          case "list_library_item_versions": {
            const item = libraryItems.find((entry) => entry.id === args?.id);
            if (!item) return [];
            const original = {
              id: item.id,
              item_id: item.id,
              version_number: 1,
              parent_version_id: null,
              language: item.language,
              code: item.code,
              origin: "original",
              created_at: item.created_at,
            };
            return [original, ...(libraryVersions[item.id] ?? [])];
          }
          case "update_library_code": {
            const item = libraryItems.find((entry) => entry.id === args?.id);
            if (!item) return null;
            const edits = (libraryVersions[item.id] ??= []);
            const head = edits[edits.length - 1];
            const version = {
              id: `library-version-${item.id}-${edits.length + 2}`,
              item_id: item.id,
              version_number: (head?.version_number ?? 1) + 1,
              parent_version_id: head?.id ?? item.id,
              language: args?.language ?? head?.language ?? item.language,
              code: String(args?.code ?? ""),
              origin: "edit",
              created_at: Math.floor(Date.now() / 1000),
            };
            edits.push(version);
            return version;
          }
          case "get_session_plan_mode":
            if (mockAcpSessions.has(args?.sessionId)) return null;
            return mockPlanMode[args?.sessionId] ?? false;
          case "set_session_plan_mode":
            if (mockAcpSessions.has(args?.sessionId)) {
              throw new Error("This conversation is bound to an ACP agent; use its own plan mode.");
            }
            mockPlanMode[args?.sessionId] = !!args?.enabled;
            return mockPlanMode[args?.sessionId];
          case "respond_ask_user":
            setTimeout(() => {
              emit("ask-user-resolved", {
                frameId: askUserFrames[args?.requestId] ?? "",
                requestId: args?.requestId,
                expired: false,
              });
              delete askUserFrames[args?.requestId];
            }, 0);
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
