import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Models the native backend, NEVER implements an iframe fallback. Guest
// protocol tests below exercise the separate shell document independently.
function nativeMock() {
  const w = window as any;
  w.__WISP_MCP_APP_BACKEND__ = "native-child";
  const original = w.__TAURI__.core.invoke;
  w.__nativeCalls = [];
  w.__nativeActive = null;
  let fence = 0;
  w.__TAURI__.core.invoke = async (command: string, args: any = {}) => {
    if (!command.includes("mcp_app_child") && command !== "mcp_app_host_info") return original(command, args);
    w.__nativeCalls.push({ command, args });
    if (command === "mcp_app_host_info") return { backend:"native-child", ownerEpoch:"test-owner" };
    if (command === "open_mcp_app_child") {
      if (args.mountSerial <= fence) throw new Error("stale-instance");
      fence = args.mountSerial;
      if (w.__nativeFail) throw new Error("simulated native controller failure");
      const handle = { ownerEpoch:"test-owner", mountSerial:args.mountSerial, childLabel:`mcp-app-child-${args.mountSerial}` };
      w.__nativeActive = handle;
      if (w.__nativeDelay) await new Promise((resolve) => setTimeout(resolve, w.__nativeDelay));
      return handle;
    }
    if (command === "close_mcp_app_child") {
      fence = Math.max(fence, args.mountSerial);
      if (w.__nativeActive?.mountSerial === args.mountSerial) w.__nativeActive = null;
      return true;
    }
    if (command === "update_mcp_app_child_bounds") return true;
    if (command === "request_mcp_app_child_action") {
      if (args.method === "wisp/motif-get-selection") return { recordName:"test DNA", recordId:"dna1", molecule:"dna", start:1, end:4, strand:"forward", sequence:"ACGT" };
      return {};
    }
    throw new Error(`Unexpected isolated command ${command}`);
  };
}
async function start(page: Page) {
  await page.addInitScript(tauriMock);
  await page.addInitScript(nativeMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.locator(".composer-inner textarea").first().fill("open isolated app");
  await page.getByRole("button", { name:"Send", exact:true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__skillInvokeLog?.some((c: any) => c.cmd === "send_message"))).toBe(true);
  return page.evaluate(() => {
    const args = (window as any).__skillInvokeLog.filter((c: any) => c.cmd === "send_message").at(-1).args;
    return String(args instanceof Map ? args.get("sessionId") : args.sessionId);
  });
}
async function present(page: Page, frame: string, title = "Isolated figures", tool = "figures_open", uri = "ui://figures/view") {
  await page.evaluate(({frame,title,tool,uri}) => (window as any).__tauriEmit("agent", {kind:"ToolPresentation", frame_id:frame, presentation_kind:"mcp_app", payload:{tool:{name:tool,title}, resource:{uri,text:"<!doctype html><body>Guest in native renderer</body>",_meta:{}}, arguments:{}, result:{content:[]}}}), {frame,title,tool,uri});
}
async function calls(page: Page, command: string) { return page.evaluate(command => (window as any).__nativeCalls.filter((c: any) => c.command === command), command); }
async function latestBounds(page: Page) { return (await calls(page, "update_mcp_app_child_bounds")).at(-1)?.args.bounds; }

test("isolated backend mounts no primary iframe; Tab switch destroys and rebuilds", async ({page}) => {
  const frame = await start(page);
  await present(page, frame);
  await expect(page.locator('[data-mcp-backend="native-child"]')).toBeVisible();
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  await expect(page.locator(".center-mcp-app iframe")).toHaveCount(0);
  await expect(page.locator("#wisp-mcp-app-parking")).toHaveCount(0);
  await present(page, frame, "Second App", "other_open", "ui://other/view");
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(2);
  expect((await calls(page,"close_mcp_app_child")).some((c:any)=>c.args.reason==="suspend")).toBe(true);
  await page.locator(".center-tab").filter({hasText:"Isolated figures"}).click();
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(3);
  await page.locator(".center-tab").filter({hasText:"Isolated figures"}).locator("..").locator(".center-tab-close").click();
  await expect.poll(async () => (await calls(page,"close_mcp_app_child")).some((c:any)=>c.args.reason==="user_close")).toBe(true);
});

test("native creation fails closed and can retry, never falls back", async ({page}) => {
  const frame = await start(page);
  await page.evaluate(() => (window as any).__nativeFail = true);
  await present(page, frame);
  await expect(page.getByRole("alert").filter({hasText:"simulated native controller failure"})).toBeVisible();
  await expect(page.locator(".center-mcp-app iframe")).toHaveCount(0);
  await page.evaluate(() => (window as any).__nativeFail = false);
  await page.getByRole("button",{name:"Reload App",exact:true}).click();
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
});

test("late create cannot resurrect a closed Tab; native rectangles exclude host controls", async ({page}) => {
  const frame = await start(page);
  await page.evaluate(() => (window as any).__nativeDelay = 500);
  await present(page, frame);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  await page.locator(".center-tab").filter({hasText:"Isolated figures"}).locator("..").locator(".center-tab-close").click();
  await expect.poll(() => page.evaluate(() => (window as any).__nativeActive)).toBeNull();
  await page.waitForTimeout(650); // controlled mock late-create boundary
  expect(await page.evaluate(() => (window as any).__nativeActive)).toBeNull();
  await page.evaluate(() => (window as any).__nativeDelay = 0);
  await present(page, frame);
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
  await expect.poll(async () => {
    const b = await latestBounds(page); const rect = await page.locator(".mcp-app-native-content").boundingBox();
    return Math.abs(b.y - rect!.y) + Math.abs(b.x - rect!.x);
  }).toBeLessThan(.5);
  const b = await latestBounds(page);
  const reload = await page.getByRole("button",{name:"Reload App",exact:true}).boundingBox();
  expect(reload!.y + reload!.height).toBeLessThanOrEqual(b.y);
});

test("host modal hides child before interaction; Escape restores only active instance", async ({page}) => {
  const frame = await start(page); await present(page,frame);
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
  await page.evaluate(() => {
    const el = document.createElement("div");el.id="test-host-overlay";el.setAttribute("role","dialog");el.setAttribute("aria-modal","true");el.style.cssText="position:fixed;inset:0;z-index:999999;background:white";document.body.append(el);
    window.addEventListener("keydown", (e) => {if(e.key==="Escape") {e.preventDefault();el.remove();}}, {once:true,capture:true});
  });
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:false});
  await page.keyboard.press("Escape");
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
  await page.setViewportSize({width:1000,height:720});
  await expect.poll(() => latestBounds(page)).toMatchObject({viewportWidth:1000,viewportHeight:720});
});

test("approval scope click preserves native child visibility and submits the selected scope", async ({page}) => {
  const frame = await start(page);
  await present(page, frame);
  await page.evaluate(frame => (window as any).__tauriEmit("confirm-request", {
    frame_id: frame, tool: "figure_library_source_status", preview: "{}", message: "Approval required",
  }), frame);
  const scope = page.getByLabel("Approval scope");
  await expect(scope).toBeVisible();
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
  await page.evaluate(() => { (window as any).__nativeCalls = []; });
  // selectOption alone skips the pointerdown that used to hide/show WebView2.
  await scope.click();
  await page.keyboard.press("Escape");
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  expect((await calls(page, "update_mcp_app_child_bounds")).filter((c: any) => !c.args.bounds.visible)).toEqual([]);
  await expect(scope).toBeFocused();
  await page.locator(".approval-scope > span").click();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  expect((await calls(page, "update_mcp_app_child_bounds")).filter((c: any) => !c.args.bounds.visible)).toEqual([]);
  await scope.selectOption("project");
  await page.getByRole("button", {name:"Allow for this project", exact:true}).click();
  await expect.poll(() => page.evaluate(() => {
    const call = (window as any).__skillInvokeLog.find((c: any) => c.cmd === "confirm_response");
    return call?.args instanceof Map ? Object.fromEntries(call.args) : call?.args;
  })).toMatchObject({approved:true, scope:"project"});
});

test("Motif selection crosses only the restricted host action bridge", async ({page}) => {
  const frame = await start(page); await present(page,frame,"Motif","motif_open_workbench","ui://motif/view");
  await expect.poll(() => latestBounds(page)).toMatchObject({visible:true});
  await page.locator(".center-mcp-app-toolbar button").nth(1).click();
  await expect.poll(async () => (await calls(page,"request_mcp_app_child_action")).at(-1)?.args.method).toBe("wisp/motif-get-selection");
  await expect(page.locator(".composer-inner textarea").first()).toHaveValue(/ACGT/);
  await expect(page.locator(".center-mcp-app iframe")).toHaveCount(0);
});

test("child shell preserves sandbox/CSP and directly forwards guest protocol", async ({page}) => {
  await page.addInitScript(() => {
    const w = window as any; w.__shellCalls=[];
    w.__TAURI_INTERNALS__ = { invoke:async (command:string,args:any={}) => {
      w.__shellCalls.push({command,args});
      if(command==="mcp_app_child_bootstrap") return {handle:{},instanceId:"mcp-app:fake:ui://test",version:"test",hostContext:{theme:"light"},payload:{tool:{name:"test"},resource:{text:`<!doctype html><body><div id="result">waiting</div><script>
      addEventListener('message',e=>{if(e.data.id===1)parent.postMessage({jsonrpc:'2.0',method:'tools/call',id:2,params:{name:'paginate',arguments:{page:2}}},'*');if(e.data.id===2)document.querySelector('#result').textContent=e.data.result.structuredContent.page;});
      parent.postMessage({jsonrpc:'2.0',method:'ui/initialize',id:1,params:{}},'*');<\/script>`},result:{content:[]}}};
      if(command==="mcp_app_child_request") return {jsonrpc:"2.0",id:args.request.id,result:args.request.method==="tools/call"?{structuredContent:{page:2}}:{hostInfo:{name:"wisp-science"}}};
      return {};
    }};
  });
  await page.goto("/mcp-app/shell.html");
  await expect(page.locator("iframe")).toHaveAttribute("sandbox","allow-scripts");
  await expect(page.locator("iframe")).toHaveAttribute("referrerpolicy","no-referrer");
  await expect(page.frameLocator("iframe").locator("#result")).toHaveText("2");
  const requests = await page.evaluate(() => (window as any).__shellCalls.filter((c:any)=>c.command==="mcp_app_child_request"));
  expect(requests.map((c:any)=>c.args.request.method)).toEqual(["ui/initialize","tools/call"]);
  await page.evaluate(() => window.postMessage({jsonrpc:"2.0",method:"tools/call",id:99,params:{name:"forged"}},"*"));
  await page.waitForTimeout(50);
  expect(await page.evaluate(() => (window as any).__shellCalls.filter((c:any)=>c.command==="mcp_app_child_request").length)).toBe(2);
});
