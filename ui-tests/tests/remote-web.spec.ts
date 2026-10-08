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
  "s-9": { items: [{ role: "assistant", text: `Other project.\n\n${WIDE_TABLE}`, tool_name: null, input: null, ok: null, status: null }], approvals: [] },
};
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
      return { ...snapshots[session], project_id, session_id: session, running: false, stopping: false, read_only: false, model_id: "m", error: null, request_id: null };
    case "native_conversation_image":
      if (args.resource_id !== "res-1" && !/\.png$/.test(args.path ?? "")) throw new Error("image preview unavailable");
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
async function remote(page: Page): Promise<Call[]> {
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
    socket.send(await seal({ type: "hello", nonce, name: "lab-mac", version: "1.18.0" }));
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
