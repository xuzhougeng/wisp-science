// Remote web access page (crates/wisp-sync/src/remote.{html,js}), served with
// the relay's real CSP. The desktop is a fake host on the other side of the
// page's WebSocket, speaking the same sealed-frame protocol, so no relay
// binary, desktop or network is needed.
import { test, expect, type Page } from "@playwright/test";
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const source = (name: string) => readFileSync(resolve(__dirname, "../../crates/wisp-sync/src", name), "utf8");
const csp = /CONTENT_SECURITY_POLICY,\s*"((?:[^"\\]|\\[\s\S])*)"/.exec(source("remote.rs"))![1].replace(/\\\r?\n\s*/g, "");

const CODE = "0001-0203-0405-0607-0809-0a0b-0c0d-0e0f";
const SID = "cf4778d13d24d0dd1313bca1709267bb"; // Same vector as remote.rs.
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const enc = new TextEncoder();
const label = (text: string) => enc.encode(text);
const concat = (a: Uint8Array, b: Uint8Array) => new Uint8Array([...a, ...b]);

type Call = { command: string; project_id: string | null; args: any };
/// What the fake desktop offers and how it behaves. A host with no `commands`
/// is a desktop older than the page, which must still work as before.
type Host = { commands?: string[]; mute?: boolean; connections?: number };
const CURRENT = [
  "native_conversation_inbox", "native_conversation_enqueue", "native_conversation_queue_action", "remote_attach_image",
];

const REPLY = [
  "## Result",
  "",
  "The **QC** step passed for `sample-A`. See [the report](results/report.md) or https://example.org/docs.",
  "",
  "| sample | status |",
  "| --- | --- |",
  "| A | pass |",
  "",
  "1. Align reads",
  "   - trimmed first",
  "2. Call variants",
  "",
  "```python",
  "print('<b>not html</b>')",
  "```",
  "",
  "![Volcano plot](figures/volcano.png)",
  '<img src=x onerror="document.title=\'pwned\'">',
].join("\n");

// Wider than a phone: it has to scroll inside its own wrapper.
const WIDE_TABLE = [
  `| ${Array.from({ length: 12 }, (_, i) => `sample_${i}_abundance`).join(" | ")} |`,
  `| ${Array.from({ length: 12 }, () => "---").join(" | ")} |`,
  `| ${Array.from({ length: 12 }, (_, i) => `${i}.123456`).join(" | ")} |`,
].join("\n");
const LONG_TITLE = "Mass spec of the aging cohort plasma proteome, second batch, reanalysed with the corrected sample sheet";

const snapshots: Record<string, any> = {
  "s-1": {
    items: [
      { role: "user", text: "Run QC", tool_name: null, input: null, ok: null, status: null },
      { role: "tool", text: "ok", tool_name: "shell", input: "make qc", ok: true, status: null, call_id: "c1" },
      {
        role: "assistant", text: REPLY, tool_name: null, input: null, ok: null, status: null,
        resources: [{
          id: "res-1", ordinal: 0, originalReference: "figures/volcano.png", artifactId: "a1", artifactVersionId: "v1",
          displayName: "volcano.png", kind: "image", mimeType: "image/png", status: "ready", error: null,
        }],
      },
    ],
    approvals: [{ approval_id: "ap-1", frame_id: "s-1", message: "Run a shell command", tool: "shell", preview: "rm -rf build" }],
  },
  "s-2": { items: [{ role: "assistant", text: "Second conversation.", tool_name: null, input: null, ok: null, status: null }], approvals: [] },
  // A turn still at work: commentary between two runs of steps.
  "s-3": {
    running: true,
    items: [
      { role: "user", text: "Profile the uploads", tool_name: null, input: null, ok: null, status: null },
      { role: "reasoning", text: "Look first.", tool_name: null, input: null, ok: null, status: null },
      { role: "assistant", text: "Inspecting the files.", tool_name: null, input: null, ok: null, status: null },
      { role: "tool", text: "no such file", tool_name: "shell", input: "head a.csv", ok: false, status: null, call_id: "t1" },
      { role: "tool", text: "id,title", tool_name: "shell", input: "head uploads/a.csv", ok: true, status: null, call_id: "t2" },
      { role: "assistant", text: "Now profiling.", tool_name: null, input: null, ok: null, status: null },
      { role: "tool", text: "53 rows", tool_name: "shell", input: "wc -l uploads/a.csv", ok: true, status: null, call_id: "t3" },
      { role: "tool", text: "", tool_name: "write", input: "profile.py", ok: null, status: null, call_id: "t4" },
    ],
    approvals: [],
  },
  "s-9": { items: [{ role: "assistant", text: `Other project.\n\n${WIDE_TABLE}`, tool_name: null, input: null, ok: null, status: null }], approvals: [] },
  // A turn at work, for queuing the next message behind it.
  "s-4": {
    running: true,
    items: [{ role: "user", text: "Align the reads", tool_name: null, input: null, ok: null, status: null }],
    approvals: [],
    queue: { items: [], outcomes: [], can_cut_in: true },
  },
  // More turns than one page holds.
  "s-6": {
    items: [{ role: "user", text: "Latest question", tool_name: null, input: null, ok: null, status: null }],
    approvals: [],
    next_before_seq: 40,
  },
  "s-7": {
    items: [{ role: "assistant", text: "Two runs started.", tool_name: null, input: null, ok: null, status: null }],
    approvals: [],
    run_cards: [
      {
        id: "r-2", title: "Align reads", kind: "local", status: "running", command: "bwa mem ref.fa reads.fq", created_at: 1750000200,
        started_at: Math.floor(Date.now() / 1000) - 125, ended_at: null, exit_code: null, stdout_tail: "chunk 1\nchunk 2", stderr_tail: null,
      },
      {
        id: "r-1", title: "Index reference", kind: "local", status: "failed", command: "bwa index ref.fa", created_at: 1750000100,
        started_at: 1750000100, ended_at: 1750000103, exit_code: 1, stdout_tail: "", stderr_tail: "ref.fa: no such file",
      },
    ],
  },
};
// Earlier pages of s-6, by the cursor that asks for them.
const earlier: Record<number, any> = {
  40: {
    items: [
      { role: "user", text: "First question", tool_name: null, input: null, ok: null, status: null },
      { role: "tool", text: "ok", tool_name: "shell", input: "ls", ok: true, status: null, call_id: "e1" },
      { role: "tool", text: "ok", tool_name: "shell", input: "pwd", ok: true, status: null, call_id: "e2" },
      { role: "assistant", text: "First answer", tool_name: null, input: null, ok: null, status: null },
    ],
    next_before_seq: 12,
  },
  12: { items: [{ role: "user", text: "Very first", tool_name: null, input: null, ok: null, status: null }], next_before_seq: null },
};
// Conversations waiting for the reader, across projects.
let inbox: any[] = [];
let uploads = 0;
const sessions: Record<string, any[]> = {
  "p-1": [
    { id: "s-1", project_id: "p-1", project_name: "RNA-seq", title: "QC run", ts: 1750000000, activity_at: 1750000300, status: "needs_you" },
    { id: "s-2", project_id: "p-1", project_name: "RNA-seq", title: "Figure polish", ts: 1750000000, activity_at: 1750000200, status: "complete" },
  ],
  "p-2": [{ id: "s-9", project_id: "p-2", project_name: "Proteomics", title: LONG_TITLE, ts: 1750000000, activity_at: 1750000100, status: "running" }],
};
const directories: Record<string, any[]> = {
  ".": [
    { name: "results", is_dir: true, size: 0 },
    { name: "figures", is_dir: true, size: 0 },
    { name: "notes.txt", is_dir: false, size: 12 },
    { name: "data.bin", is_dir: false, size: 2048 },
  ],
  results: [
    { name: "report.md", is_dir: false, size: 64 },
    { name: "big.log", is_dir: false, size: 5 * 1024 * 1024 },
    { name: "paper.pdf", is_dir: false, size: 4096 },
  ],
  figures: [{ name: "volcano.png", is_dir: false, size: 68 }],
};
const files: Record<string, any> = {
  "notes.txt": { mime: "text/plain", text: "plain <i>notes</i>", base64: null, truncated: false, total_bytes: 12 },
  "results/report.md": { mime: "text/markdown", text: "# Report\n\nSee ![fig](../figures/volcano.png) and [log](big.log).", base64: null, truncated: false, total_bytes: 64 },
  "results/big.log": { mime: "text/plain", text: "first lines", base64: null, truncated: true, total_bytes: 5 * 1024 * 1024 },
  "data.bin": { mime: "application/octet-stream", text: null, base64: null, truncated: false, total_bytes: 2048 },
  // The desktop extracts documents to Markdown; the original bytes stay there.
  "results/paper.pdf": { mime: "application/pdf", text: "## Abstract\n\nExtracted *text*.", base64: null, truncated: false, total_bytes: 4096 },
};

function answer({ command, project_id, args }: Call): any {
  const session = args.session_id as string;
  switch (command) {
    case "list_projects":
      return [
        { id: "p-1", name: "RNA-seq", session_count: 2, running_count: 0, needs_you_count: 1 },
        { id: "p-2", name: "Proteomics", session_count: 1, running_count: 1, needs_you_count: 0 },
      ];
    case "remote_sessions":
      return sessions[project_id!];
    case "native_conversation_snapshot":
      return { running: false, ...(args.before_seq == null ? snapshots[session] : earlier[args.before_seq]), project_id, session_id: session, stopping: false, read_only: false, model_id: "m", error: null, request_id: null };
    case "native_conversation_inbox":
      if (Object.keys(args).length) throw new Error("Inbox takes no arguments");
      return inbox;
    case "native_conversation_enqueue":
      if (!snapshots[session].running) throw new Error("Queue a follow-up only while a turn is running");
      snapshots[session].queue.items.push({ id: String(snapshots[session].queue.items.length + 7), digest: `d-${args.message}`, state: "queued", message: args.message, attachments: [], references: [] });
      return { queued: true, id: "7" };
    case "native_conversation_queue_action": {
      const queue = snapshots[session].queue;
      if (args.action.kind !== "cancel") throw new Error("Remote browsers can only cancel a queued message");
      if (!queue.items.some((item: any) => item.id === args.id && item.digest === args.digest)) throw new Error("This queued turn has already started or changed; refresh the queue");
      queue.items = queue.items.filter((item: any) => item.id !== args.id);
      return { session_id: session, id: args.id };
    }
    case "remote_attach_image": {
      // The picture arrives re-encoded as a JPEG, never under a name of its own.
      if (Object.keys(args).sort().join() !== "base64,session_id") throw new Error("unknown field");
      if (!Buffer.from(args.base64, "base64").subarray(0, 3).equals(Buffer.from([0xff, 0xd8, 0xff]))) throw new Error("Only JPEG, PNG and WebP pictures can be attached remotely");
      const name = `photo-${++uploads}.jpg`;
      return { path: `uploads/${name}`, name };
    }
    case "native_conversation_image":
      if (args.resource_id !== "res-1" && !/\.(png|jpg)$/.test(args.path ?? "")) throw new Error("image preview unavailable");
      return { path: "thumbnail", mime: "image/png", text: null, base64: PNG, truncated: false, total_bytes: 68 };
    case "native_conversation_panel_file_directory":
      if (!directories[args.path]) throw new Error("No such directory");
      return { project_id, session_id: session, context_id: "local", path: args.path, entries: directories[args.path] };
    case "native_conversation_panel_file_read":
      if (!files[args.path]) throw new Error("No such file");
      return { project_id, session_id: session, context_id: "local", requested_path: args.path, content: { path: args.path, ...files[args.path] } };
    case "native_conversation_create":
      return "s-2";
    case "native_conversation_send":
      return { request_id: args.request_id, session_id: session, epoch: "e" };
    case "native_conversation_approve":
    case "native_conversation_seen":
      return null;
    default:
      throw new Error("This command is not available remotely");
  }
}

/// Serves the page and stands in for relay + desktop. Returns every request
/// the browser made, after authentication and replay checks.
async function remote(page: Page, host: Host = {}): Promise<Call[]> {
  const calls: Call[] = [];
  const secret = Uint8Array.from(CODE.replace(/-/g, "").match(/../g)!, (hex) => parseInt(hex, 16));
  const raw = await webcrypto.subtle.digest("SHA-256", concat(label("wisp-remote/key/v1"), secret));
  const key = await webcrypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
  const seal = async (value: unknown) => {
    const iv = webcrypto.getRandomValues(new Uint8Array(12));
    const body = await webcrypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: label("wisp-remote/v1/h2c") }, key, enc.encode(JSON.stringify(value)));
    return Buffer.concat([iv, new Uint8Array(body)]).toString("base64");
  };
  await page.route("**/remote", (route) =>
    route.fulfill({ contentType: "text/html; charset=utf-8", headers: { "content-security-policy": csp }, body: source("remote.html") }));
  await page.route("**/remote.js", (route) => route.fulfill({ contentType: "text/javascript; charset=utf-8", body: source("remote.js") }));
  await page.routeWebSocket(new RegExp(`/v1/remote/client/${SID}$`), async (socket) => {
    const nonce = webcrypto.randomUUID();
    let seq = 0;
    let chain = Promise.resolve();
    socket.onMessage((frame) => {
      chain = chain.then(async () => {
        const bytes = Buffer.from(String(frame), "base64");
        const plain = await webcrypto.subtle.decrypt({ name: "AES-GCM", iv: bytes.subarray(0, 12), additionalData: label("wisp-remote/v1/c2h") }, key, bytes.subarray(12));
        const request = JSON.parse(new TextDecoder().decode(plain));
        if (request.nonce !== nonce || request.seq <= seq) return;
        seq = request.seq;
        if (host.mute) return; // A connection that died without closing.
        const call = { command: request.command, project_id: request.project_id, args: request.args };
        calls.push(call);
        let result = null;
        let error = null;
        try {
          result = answer(call);
        } catch (failure) {
          error = (failure as Error).message;
        }
        socket.send(await seal({ type: "response", response: { schema: "wisp.native-settings.v1", id: request.id, result, error } }));
      });
    });
    host.connections = (host.connections ?? 0) + 1;
    host.mute = false; // A redial reaches a live relay again.
    socket.send(await seal({ type: "hello", nonce, name: "lab-mac", version: "1.18.0", commands: host.commands }));
  });
  return calls;
}

function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (message) => message.type() === "error" && problems.push(message.text()));
  page.on("pageerror", (error) => problems.push(String(error)));
  return problems;
}

test.use({ locale: "en-US" });

test("projects and conversations switch from the sidebar, with Back and reload", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`/remote#${CODE}`);

  await expect(page.locator("#status")).toHaveText("Connected");
  await expect(page.locator("#side")).toBeHidden();
  await page.getByRole("link", { name: /RNA-seq/ }).click();
  await expect(page).toHaveURL(new RegExp(`#${CODE}/p/p-1$`));
  await expect(page.locator("#title")).toHaveText("RNA-seq");
  await expect(page.locator("#main")).toHaveText("Pick a conversation or start a new one.");
  await expect(page.locator("#composer")).toBeHidden();
  const rows = page.locator("#sessions a.row");
  await expect(rows).toHaveText([/QC run.*Needs you/, /Figure polish/]);

  await rows.nth(0).click();
  await expect(page).toHaveURL(new RegExp(`#${CODE}/p/p-1/s/s-1$`));
  await expect(page.locator("#title")).toHaveText("QC run");
  await expect(page.locator("#subtitle")).toHaveText("RNA-seq");
  await expect(rows.nth(0)).toHaveAttribute("aria-current", "page");
  await expect(page.locator("#main .msg.user")).toHaveText("Run QC");
  await expect(page.locator("#composer")).toBeVisible();

  // Switch conversation, then project, without leaving the page.
  await rows.nth(1).click();
  await expect(page.locator("#main")).toContainText("Second conversation.");
  await expect(rows.nth(1)).toHaveAttribute("aria-current", "page");
  await expect(rows.nth(0)).not.toHaveAttribute("aria-current", "page");
  await page.locator("#project").selectOption("p-2");
  await expect(page).toHaveURL(new RegExp(`#${CODE}/p/p-2$`));
  await expect(rows).toHaveText([/Mass spec.*Running/]);
  await rows.nth(0).click();
  await expect(page.locator("#main")).toContainText("Other project.");

  // Browser history walks back through the same places.
  await page.goBack();
  await page.goBack();
  await expect(page.locator("#main")).toContainText("Second conversation.");
  await expect(page.locator("#project")).toHaveValue("p-1");
  await expect(rows).toHaveCount(2);
  await page.reload();
  await expect(page.locator("#main")).toContainText("Second conversation.");
  await expect(page.locator("#title")).toHaveText("Figure polish");

  await page.locator("#back").click();
  await expect(page).toHaveURL(new RegExp(`#${CODE}/p/p-1$`));
  await page.locator("#back").click();
  await expect(page).toHaveURL(new RegExp(`#${CODE}$`));
  await expect(page.getByRole("link", { name: /Proteomics/ })).toContainText("Running");

  // Opening a finished conversation clears its "needs you" flag.
  expect(calls.filter((call) => call.command === "native_conversation_seen").map((call) => call.args.session_id)).toEqual(
    expect.arrayContaining(["s-1", "s-2", "s-9"]));
  expect(problems).toEqual([]);
});

test("replies render Markdown and conversation images without trusting their HTML", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-1`);

  const reply = page.locator("#main .md");
  await expect(reply.locator("h4")).toHaveText("Result");
  await expect(reply.locator("strong")).toHaveText("QC");
  await expect(reply.locator("p code")).toHaveText("sample-A");
  await expect(reply.locator("th")).toHaveText(["sample", "status"]);
  await expect(reply.locator("td")).toHaveText(["A", "pass"]);
  await expect(reply.locator("ol > li")).toHaveCount(2);
  await expect(reply.locator("ol > li").first().locator("ul > li")).toHaveText("trimmed first");
  await expect(reply.locator("pre code")).toHaveText("print('<b>not html</b>')");
  await expect(reply.getByRole("link", { name: "https://example.org/docs" })).toHaveAttribute("rel", "noopener noreferrer");
  await expect(reply.getByRole("link", { name: "the report" })).toHaveAttribute("href", `#${CODE}/p/p-1/s/s-1/f/results%2Freport.md`);

  // The picture is the captured message resource, as a data: thumbnail.
  await expect(reply.locator(".img img")).toHaveAttribute("src", `data:image/png;base64,${PNG}`);
  await expect(reply.locator(".img .cap")).toHaveText("Volcano plot");
  expect(calls.find((call) => call.command === "native_conversation_image")!.args).toEqual({ session_id: "s-1", resource_id: "res-1", path: null });
  // Raw HTML in a reply stays text.
  await expect(reply).toContainText(`<img src=x onerror="document.title='pwned'">`);
  await expect(page.locator("#main img")).toHaveCount(1);
  await expect(page).toHaveTitle("QC run · Wisp");

  // Tool details stay open across refreshes; approvals and sending still work.
  await page.locator("details.tool summary").click();
  await expect(page.locator("details.tool pre")).toHaveText("make qc\n\nok");
  await page.locator("#draft").fill("continue");
  await page.locator("#send").click();
  await expect(page.locator("#draft")).toHaveValue("");
  await expect(page.locator("details.tool")).toHaveAttribute("open", "");
  await page.getByRole("button", { name: "Approve" }).click();
  await expect.poll(() => calls.filter((call) => call.command === "native_conversation_approve").map((call) => call.args)).toEqual([
    { session_id: "s-1", approval_id: "ap-1", approved: true },
  ]);
  expect(calls.find((call) => call.command === "native_conversation_send")!.args.message).toBe("continue");
  expect(problems).toEqual([]);
});

test("a turn's steps fold as on the desktop: newest open while running, one row once done", async ({ page }) => {
  const problems = watchConsole(page);
  await remote(page);
  await page.goto(`/remote#${CODE}/p/p-1/s/s-3`);

  const folds = page.locator("#main > .transcript > details.steps");
  const commentary = page.locator("#main > .transcript > .md");
  await expect(folds.locator(":scope > summary")).toHaveText(["Ran 2 steps", "Working…"]);
  await expect(commentary).toHaveText(["Inspecting the files.", "Now profiling."]);
  await expect(folds.nth(0)).not.toHaveAttribute("open");
  await expect(folds.nth(1)).toHaveAttribute("open", "");
  await expect(folds.nth(1).locator("details.tool .state")).toHaveText(["Done", "Running"]);
  // A lone step is its own row.
  await expect(page.locator("#main > .transcript > details.tool:not(.steps) > summary")).toHaveText("ThinkingDone");

  // Closing the live run survives the refresh that finishes its last step.
  await folds.nth(1).locator(":scope > summary").click();
  const live = snapshots["s-3"];
  live.items[7] = { ...live.items[7], ok: true };
  await expect(folds.nth(1).locator("details.tool .state")).toHaveText(["Done", "Done"]);
  await expect(folds.nth(1)).not.toHaveAttribute("open");

  snapshots["s-3"] = { ...live, running: false, items: [...live.items, { role: "assistant", text: "All done.", tool_name: null, input: null, ok: null, status: null }] };
  await expect(folds.locator(":scope > summary")).toHaveText(["Ran 4 steps"]);
  await expect(commentary).toHaveText(["All done."]);
  await expect(folds).not.toHaveAttribute("open");
  await folds.locator(":scope > summary").click();
  await expect(folds.locator("details.tool .state")).toHaveText(["Done", "Failed", "Done", "Done", "Done"]);
  await expect(folds.locator(".md")).toHaveText(["Inspecting the files.", "Now profiling."]);
  expect(problems).toEqual([]);
});

test("the files pane lists folders and previews text and images only", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-1`);
  await expect(page.locator("#panel")).toBeHidden();

  await page.locator("#files").click();
  await expect(page).toHaveURL(new RegExp(`/s/s-1/d/\\.$`));
  await expect(page.locator("#files")).toHaveAttribute("aria-pressed", "true");
  const entries = page.locator("#panel-body a.row");
  await expect(entries).toHaveText([/results/, /figures/, /notes\.txt.*12 B/, /data\.bin.*2\.0 KB/]);
  // The conversation stays beside the files on a wide screen.
  await expect(page.locator("#main .msg.user")).toBeVisible();
  await expect(page.locator("#sessions")).toBeVisible();

  await entries.nth(2).click();
  await expect(page.locator("#panel-body pre")).toHaveText("plain <i>notes</i>");
  await expect(page.locator("#crumbs")).toHaveText("Files / notes.txt");
  await page.locator("#back").click();
  await entries.nth(3).click();
  await expect(page.locator("#panel-body")).toHaveText("This file type cannot be previewed here. Open it on the desktop.");

  await page.locator("#crumbs a").click();
  await entries.nth(0).click();
  await expect(page.locator("#crumbs")).toHaveText("Files / results");
  await expect(entries).toHaveText([/report\.md/, /big\.log.*5\.0 MB/, /paper\.pdf/]);
  await entries.nth(1).click();
  await expect(page.locator("#panel-body .note")).toHaveText("Large file (5.0 MB); only the beginning is shown.");
  await expect(page.locator("#panel-body pre")).toHaveText("first lines");
  await page.goBack();
  await entries.nth(2).click();
  await expect(page.locator("#panel-body .md h4")).toHaveText("Abstract");
  await expect(page.locator("#panel-body .md em")).toHaveText("text");

  // A Markdown file renders, resolving its picture and links from its folder.
  await page.goBack();
  await entries.nth(0).click();
  await expect(page.locator("#panel-body .md h3")).toHaveText("Report");
  await expect(page.locator("#panel-body .md img")).toHaveAttribute("src", `data:image/png;base64,${PNG}`);
  await expect(page.locator("#panel-body .md").getByRole("link", { name: "log" })).toHaveAttribute("href", /\/f\/results%2Fbig\.log$/);

  await page.locator("#crumbs a").first().click();
  await entries.nth(1).click();
  await entries.nth(0).click();
  await expect(page).toHaveURL(/\/f\/figures%2Fvolcano\.png$/);
  await expect(page.locator("#panel-body .preview img")).toHaveAttribute("src", `data:image/png;base64,${PNG}`);
  await expect(page.locator("#title")).toHaveText("volcano.png");

  // A link in the reply opens the same preview.
  await page.locator("#panel-close").click();
  await expect(page.locator("#panel")).toBeHidden();
  await page.locator("#main .md").getByRole("link", { name: "the report" }).click();
  await expect(page.locator("#panel-body .md h3")).toHaveText("Report");

  // Only local, text or thumbnail reads were asked of the desktop.
  for (const call of calls.filter((entry) => entry.command.includes("_panel_file_"))) {
    expect(Object.keys(call.args).sort()).toEqual(["path", "session_id"]);
  }
  expect(calls.filter((call) => call.command === "native_conversation_image" && call.args.path).map((call) => call.args.path)).toEqual(
    expect.arrayContaining(["figures/volcano.png"]));
  expect(problems).toEqual([]);
});

test("a phone shows one pane at a time and Back steps out of it", async ({ page }) => {
  const problems = watchConsole(page);
  await remote(page);
  await page.setViewportSize({ width: 390, height: 780 });
  await page.goto(`/remote#${CODE}`);

  await page.getByRole("link", { name: /RNA-seq/ }).click();
  await expect(page.locator("#sessions a.row")).toHaveCount(2);
  await expect(page.locator("#main")).toBeHidden();
  await page.locator("#sessions a.row").first().click();
  await expect(page.locator("#main .msg.user")).toBeVisible();
  await expect(page.locator("#side")).toBeHidden();
  await expect(page.locator("#composer")).toBeVisible();
  // Nothing scrolls sideways at phone width.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);

  // Files cover the conversation, which keeps its scroll position underneath.
  await page.locator("#main").evaluate((node) => (node.scrollTop = 40));
  await page.locator("#files").click();
  await expect(page.locator("#panel-body a.row")).toHaveCount(4);
  expect(await page.evaluate(() => {
    const panel = document.getElementById("panel")!.getBoundingClientRect();
    const chat = document.querySelector(".chat")!.getBoundingClientRect();
    return panel.width === window.innerWidth && panel.top === chat.top && panel.bottom === chat.bottom;
  })).toBe(true);
  await page.locator("#panel-body a.row").first().click();
  await expect(page.locator("#title")).toHaveText("results");
  await page.locator("#back").click();
  await expect(page.locator("#panel-body a.row")).toHaveCount(4);
  await page.locator("#back").click();
  await expect(page.locator("#panel")).toBeHidden();
  expect(await page.locator("#main").evaluate((node) => node.scrollTop)).toBe(40);
  await page.locator("#back").click();
  await expect(page.locator("#sessions a.row")).toHaveCount(2);
  await page.locator("#new").click();
  await expect(page).toHaveURL(/\/p\/p-1\/s\/s-2$/);
  await expect(page.locator("#main")).toContainText("Second conversation.");
  expect(problems).toEqual([]);
});

test("a long title and a wide table stay inside their pane on a phone", async ({ page }) => {
  await remote(page);
  await page.setViewportSize({ width: 390, height: 780 });
  await page.goto(`/remote#${CODE}/p/p-2`);
  const row = page.locator("#sessions a.row");
  await expect(row).toContainText("Mass spec");
  const fits = () => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth
    && [...document.querySelectorAll("#sessions, #main")].every((pane) => pane.scrollWidth <= pane.clientWidth));
  // The title is cut with an ellipsis; the row and its badge stay on screen.
  expect(await fits()).toBe(true);
  await expect(row.locator(".badge")).toBeInViewport({ ratio: 1 });

  await row.click();
  await expect(page.locator("#main th").first()).toBeVisible();
  expect(await fits()).toBe(true);
  // The table is the part that scrolls sideways, not the conversation.
  expect(await page.locator("#main .table-wrap").evaluate((wrap) => wrap.scrollWidth > wrap.clientWidth)).toBe(true);
});

test("a missing or malformed code asks for one instead of connecting", async ({ page }) => {
  await remote(page);
  await page.goto("/remote");
  await expect(page.getByRole("heading", { name: "Connect to your Wisp" })).toBeVisible();
  await page.goto("/remote#not-a-code/p/p-1");
  await expect(page.getByRole("alert")).toHaveText("Invalid code (32 hex digits).");
  // Typed with other separators, the code still connects and stays out of the route.
  await page.getByLabel("Connection code").fill(CODE.replace(/-/g, " / "));
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page).toHaveURL(new RegExp(`#${CODE.replace(/-/g, "")}$`));
  await expect(page.locator("#status")).toHaveText("Connected");
  await expect(page.getByRole("link", { name: /RNA-seq/ })).toBeVisible();
});

/// Sends the page to the background or back, as a phone does when its user
/// switches apps.
const setVisible = (page: Page, visible: boolean) => page.evaluate((state) => {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}, visible ? "visible" : "hidden");
const users = (page: Page) => page.locator("#main .msg.user");

test("the next message queues behind a running turn and can be withdrawn", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page, { commands: CURRENT });
  await page.setViewportSize({ width: 390, height: 780 });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-4`);

  const send = page.locator("#send");
  await expect(send).toHaveText("Queue");
  await expect(page.locator("#stop")).toBeVisible();
  await page.locator("#draft").fill("then plot the coverage");
  await send.click();
  await expect(page.locator("#draft")).toHaveValue("");
  const queued = page.locator("#main .queued");
  await expect(queued).toHaveText("then plot the coverage");
  expect(calls.find((call) => call.command === "native_conversation_enqueue")!.args).toMatchObject({ session_id: "s-4", message: "then plot the coverage" });
  expect(calls.some((call) => call.command === "native_conversation_send")).toBe(false);
  // The composer and its three controls still fit a phone.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);

  await queued.getByRole("button", { name: "Cancel queued message" }).click();
  await expect(queued).toHaveCount(0);
  expect(calls.find((call) => call.command === "native_conversation_queue_action")!.args).toEqual(
    { session_id: "s-4", id: "7", digest: "d-then plot the coverage", action: { kind: "cancel" } });

  // Once the turn ends the same button sends, and suggestions fill the draft without sending.
  snapshots["s-4"] = { ...snapshots["s-4"], running: false, follow_ups: ["Plot the top genes", "Export the table"] };
  await expect(send).toHaveText("Send");
  await expect(page.locator("#stop")).toBeHidden();
  const chips = page.getByRole("group", { name: "Suggested next messages" }).getByRole("button");
  await expect(chips).toHaveText(["Plot the top genes", "Export the table"]);
  await chips.nth(1).click();
  await expect(page.locator("#draft")).toHaveValue("Export the table");
  expect(calls.some((call) => call.command === "native_conversation_send")).toBe(false);
  await send.click();
  await expect.poll(() => calls.find((call) => call.command === "native_conversation_send")?.args.message).toBe("Export the table");
  expect(problems).toEqual([]);
});

test("an older desktop is offered only what it supports", async ({ page }) => {
  snapshots["s-4"] = { ...snapshots["s-4"], running: true, follow_ups: [] };
  inbox = [{ id: "s-1", project_id: "p-1", project_name: "RNA-seq", title: "QC run", status: "needs_you" }];
  const calls = await remote(page);
  await page.goto(`/remote#${CODE}/p/p-1/s/s-4`);
  await expect(page.locator("#stop")).toBeVisible();
  await expect(page.locator("#send")).toBeHidden();
  await expect(page.locator("#attach")).toBeHidden();
  await expect(page.locator("#inbox")).toBeHidden();
  expect(calls.some((call) => CURRENT.includes(call.command))).toBe(false);
  inbox = [];
});

test("the header counts other conversations that need you and jumps to the next", async ({ page }) => {
  const problems = watchConsole(page);
  inbox = [
    { id: "s-1", project_id: "p-1", project_name: "RNA-seq", title: "QC run", status: "needs_you" },
    { id: "s-9", project_id: "p-2", project_name: "Proteomics", title: LONG_TITLE, status: "needs_you" },
  ];
  await remote(page, { commands: CURRENT });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-2`);

  const bell = page.locator("#inbox");
  await expect(bell).toHaveAccessibleName("2 other conversations need you");
  await expect(bell.locator(".count")).toHaveText("2");
  await expect(page).toHaveTitle("(2) Figure polish · Wisp");
  await bell.click();
  await expect(page).toHaveURL(new RegExp(`/p/p-1/s/s-1$`));
  // The open conversation is in front of the reader, so it is not counted.
  await expect(bell).toHaveAccessibleName("1 other conversation needs you");
  await expect(page).toHaveTitle("(1) QC run · Wisp");
  await bell.click();
  await expect(page).toHaveURL(new RegExp(`/p/p-2/s/s-9$`));
  await expect(page.locator("#project")).toHaveValue("p-2");
  // In a background tab the title is all there is, so the open one counts again.
  await setVisible(page, false);
  await expect(page).toHaveTitle(/^\(2\) Mass spec/);
  await setVisible(page, true);
  await expect(page).toHaveTitle(/^\(1\) Mass spec/);

  inbox = [];
  await expect(bell).toBeHidden({ timeout: 15_000 });
  await expect(page).toHaveTitle(/^Mass spec/);
  expect(problems).toEqual([]);
});

test("coming back to the page catches up at once and replaces a dead connection", async ({ page }) => {
  const host: Host = { commands: CURRENT };
  const calls = await remote(page, host);
  await page.goto(`/remote#${CODE}/p/p-1/s/s-2`);
  await expect(page.locator("#main")).toContainText("Second conversation.");
  const polls = () => calls.filter((call) => call.command === "native_conversation_snapshot").length;

  // In the background the page stops its 4 s polling once the pending round is done.
  await setVisible(page, false);
  const before = polls();
  await expect.poll(polls, { timeout: 8_000 }).toBeGreaterThan(before);
  const settled = polls();
  const original = snapshots["s-2"];
  snapshots["s-2"] = { ...original, items: [...original.items, { role: "assistant", text: "Finished while you were away.", tool_name: null, input: null, ok: null, status: null }] };
  await page.waitForTimeout(5_000);
  expect(polls()).toBe(settled);
  await expect(page.locator("#main")).not.toContainText("Finished while you were away.");
  await setVisible(page, true);
  await expect(page.locator("#main")).toContainText("Finished while you were away.", { timeout: 2_000 });

  // A socket that stopped answering without closing is closed and redialled.
  host.mute = true;
  await setVisible(page, false);
  await setVisible(page, true);
  await expect.poll(() => host.connections, { timeout: 10_000 }).toBe(2);
  await expect(page.locator("#status")).toHaveText("Connected");
  snapshots["s-2"] = { ...original, items: [...original.items, { role: "assistant", text: "Back online.", tool_name: null, input: null, ok: null, status: null }] };
  await expect(page.locator("#main")).toContainText("Back online.");
  await expect(page.locator("#main [role=alert]")).toHaveCount(0);
  snapshots["s-2"] = original;
});

test("earlier messages load a page at a time, above what is already shown", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page, { commands: CURRENT });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-6`);

  const more = page.getByRole("button", { name: "Load earlier messages" });
  await expect(users(page)).toHaveText(["Latest question"]);
  await more.click();
  await expect(users(page)).toHaveText(["First question", "Latest question"]);
  // An earlier turn folds its steps like any finished one.
  await expect(page.locator("#main details.steps > summary")).toHaveText("Ran 2 steps");
  await expect(page.locator("#main .md")).toHaveText("First answer");
  await more.click();
  await expect(users(page)).toHaveText(["Very first", "First question", "Latest question"]);
  await expect(more).toBeHidden();
  expect(calls.filter((call) => call.command === "native_conversation_snapshot" && call.args.before_seq != null).map((call) => call.args.before_seq)).toEqual([40, 12]);

  // A new turn moves the latest page on; older pages start over rather than leave a gap.
  snapshots["s-6"] = { ...snapshots["s-6"], next_before_seq: 55 };
  await expect(users(page)).toHaveText(["Latest question"]);
  await expect(more).toBeVisible();
  expect(problems).toEqual([]);
});

test("runs show their state and the end of their output, updated in place", async ({ page }) => {
  const problems = watchConsole(page);
  await remote(page, { commands: CURRENT });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-7`);

  const runs = page.locator("#main details.run");
  await expect(runs.locator(".name")).toHaveText(["Index reference", "Align reads"]);
  await expect(runs.locator(".state")).toHaveText(["Failed", "Running"]);
  await expect(runs.nth(0)).not.toHaveAttribute("open");
  await expect(runs.nth(1)).toHaveAttribute("open", "");
  await expect(runs.nth(1).locator(".sub")).toHaveText(/^2m \d+s$/);
  await expect(runs.nth(1).locator("pre")).toHaveText("$ bwa mem ref.fa reads.fq\n\nchunk 1\nchunk 2");
  await runs.nth(0).locator("summary").click();
  await expect(runs.nth(0).locator("pre")).toHaveText("$ bwa index ref.fa\n\nref.fa: no such file");
  await expect(runs.nth(0).locator(".sub")).toHaveText("3s");

  // New output and the final state arrive in the same row.
  await runs.nth(1).evaluate((node) => ((node as HTMLElement).dataset.kept = "yes"));
  const [live, failed] = snapshots["s-7"].run_cards;
  snapshots["s-7"] = { ...snapshots["s-7"], run_cards: [{ ...live, status: "succeeded", ended_at: live.started_at + 130, exit_code: 0, stdout_tail: "chunk 1\nchunk 2\nchunk 3" }, failed] };
  await expect(runs.nth(1).locator(".state")).toHaveText("Done");
  await expect(runs.nth(1).locator("pre")).toContainText("chunk 3");
  await expect(runs.nth(1).locator(".sub")).toHaveText("2m 10s");
  await expect(runs.nth(1)).toHaveAttribute("data-kept", "yes");
  await expect(runs.nth(0)).toHaveAttribute("open", "");
  expect(problems).toEqual([]);
});

test("a picture is shrunk in the browser, attached and sent with the message", async ({ page }) => {
  const problems = watchConsole(page);
  const calls = await remote(page, { commands: CURRENT });
  await page.setViewportSize({ width: 390, height: 780 });
  await page.goto(`/remote#${CODE}/p/p-1/s/s-2`);

  const picture = { name: "IMG_0001.png", mimeType: "image/png", buffer: Buffer.from(PNG, "base64") };
  await expect(page.getByRole("button", { name: "Add a picture" })).toBeVisible();
  await expect(page.locator("#send")).toBeDisabled();
  await page.locator("#photo").setInputFiles([picture, picture]);
  const thumbs = page.locator("#photos .photo");
  await expect(thumbs).toHaveCount(2);
  await expect(thumbs.first().locator("img")).toHaveAttribute("src", /^data:image\/jpeg;base64,\/9j\//);
  await thumbs.first().getByRole("button", { name: "Remove picture" }).click();
  await expect(thumbs).toHaveCount(1);
  // Something that is not a picture is refused here, before anything is sent.
  await page.locator("#photo").setInputFiles({ name: "notes.png", mimeType: "image/png", buffer: Buffer.from("not a picture") });
  await expect(page.locator("#main [role=alert]")).toHaveText("This picture could not be processed");
  expect(calls.filter((call) => call.command === "remote_attach_image")).toHaveLength(2);

  // A picture alone is a message.
  await expect(page.locator("#send")).toBeEnabled();
  await page.locator("#send").click();
  await expect(thumbs).toHaveCount(0);
  await expect(page.locator("#photos")).toBeHidden();
  expect(calls.find((call) => call.command === "native_conversation_send")!.args).toMatchObject({ session_id: "s-2", message: "", attachments: ["uploads/photo-2.jpg"] });

  // The sent picture shows in the message; the line that names its file does not.
  const original = snapshots["s-2"];
  snapshots["s-2"] = { ...original, items: [...original.items, { role: "user", text: "What is this?\n\nUploaded files: uploads/photo-2.jpg", attachments: ["uploads/photo-2.jpg"], tool_name: null, input: null, ok: null, status: null }] };
  await expect(users(page)).toHaveText("What is this?photo-2.jpg");
  await expect(users(page).locator("img")).toHaveAttribute("src", `data:image/png;base64,${PNG}`);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  snapshots["s-2"] = original;
  expect(problems).toEqual([]);
});

test("the page can live on the home screen: it remembers the code until told to forget", async ({ page }) => {
  await remote(page, { commands: CURRENT });
  await page.goto(`/remote#${CODE}`);
  await expect(page.locator("#status")).toHaveText("Connected");
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute("href", "remote.webmanifest");

  // The installed page starts without the link.
  await page.goto("about:blank");
  await page.goto("/remote");
  await expect(page).toHaveURL(new RegExp(`#${CODE}$`));
  await expect(page.locator("#status")).toHaveText("Connected");
  await expect(page.getByRole("link", { name: /RNA-seq/ })).toBeVisible();

  await page.getByRole("button", { name: "Use another code" }).click();
  await expect(page.getByRole("heading", { name: "Connect to your Wisp" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("heading", { name: "Connect to your Wisp" })).toBeVisible();
});
