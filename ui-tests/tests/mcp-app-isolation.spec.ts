import { trackInvokes, waitForInvokes, settleRendering } from "./async-observers";
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
  // The mock completes the opening turn asynchronously. In an optimized build
  // its later Done event could otherwise clear the App approval injected below.
  await expect(page.getByText("Hello from mock wisp-science.", { exact:true })).toBeVisible();
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
  await trackInvokes(page, ["open_mcp_app_child"]);
  await page.evaluate(() => (window as any).__nativeDelay = 500);
  await present(page, frame);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  await page.locator(".center-tab").filter({hasText:"Isolated figures"}).locator("..").locator(".center-tab-close").click();
  await expect.poll(() => page.evaluate(() => (window as any).__nativeActive)).toBeNull();
  await waitForInvokes(page);
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
  // Accept the current native option without rejecting the host approval.
  // macOS Chromium forwards Escape from its select popup to the page.
  await page.keyboard.press("Enter");
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
  await settleRendering(page);
  expect(await page.evaluate(() => (window as any).__shellCalls.filter((c:any)=>c.command==="mcp_app_child_request").length)).toBe(2);
});

test("child shell forwards context attach, replacement and clear before acknowledging", async ({page}) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__contextRequests = [];
    w.__TAURI_INTERNALS__ = { invoke: async (command: string, args: any = {}) => {
      if (command === "mcp_app_child_bootstrap") return {
        handle: {}, instanceId: "mcp-app:fake:ui://figures/view", version: "test",
        hostContext: {}, payload: { tool: {name: "figures", title: "Figures"},
          resource: {text: `<!doctype html><body><button id="attach">Attach</button><button id="replace">Replace</button><button id="clear">Clear</button><div id="ack">waiting</div><script>
          let id = 1;
          const send = params => parent.postMessage({jsonrpc:'2.0',method:'ui/update-model-context',id:id++,params},'*');
          document.querySelector('#attach').onclick = () => send({content:[{type:'text',text:'Selected figure A'}],structuredContent:{templateId:'A'}});
          document.querySelector('#replace').onclick = () => send({content:[{type:'text',text:'Selected figure B'}]});
          document.querySelector('#clear').onclick = () => send({});
          addEventListener('message', e => {if(e.data.result)document.querySelector('#ack').textContent='ack:'+e.data.id;});
          <\/script>`}, result: {content: []} }
      };
      if (command === "mcp_app_child_request") {
        w.__contextRequests.push(args.request);
        return {jsonrpc: "2.0", id: args.request.id, result: {}};
      }
      return {};
    }};
  });
  await page.goto("/mcp-app/shell.html");
  const guest = page.frameLocator("iframe");
  await guest.getByRole("button", {name:"Attach", exact:true}).click();
  await expect(guest.locator("#ack")).toHaveText("ack:1");
  await guest.getByRole("button", {name:"Replace", exact:true}).click();
  await expect(guest.locator("#ack")).toHaveText("ack:2");
  await guest.getByRole("button", {name:"Clear", exact:true}).click();
  await expect(guest.locator("#ack")).toHaveText("ack:3");
  expect(await page.evaluate(() => (window as any).__contextRequests)).toEqual([
    {id:1, method:"ui/update-model-context", params:{content:[{type:"text",text:"Selected figure A"}],structuredContent:{templateId:"A"}}},
    {id:2, method:"ui/update-model-context", params:{content:[{type:"text",text:"Selected figure B"}]}},
    {id:3, method:"ui/update-model-context", params:{}},
  ]);
});

test("native composer displays context without starting a turn and clears on removal or final close", async ({page}) => {
  const frame = await start(page);
  await present(page, frame);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  await page.evaluate(frame => {
    const w = window as any, original = w.__TAURI__.core.invoke;
    const instance = `mcp-app:${frame}:ui://figures/view`;
    const notice = (summary: string, cleared: boolean, frameId = frame) => w.__tauriEmit("agent", {
      kind:"AppContextUpdate",frame_id:frameId,context_id:instance,instance_id:instance,
      app_name:"Isolated figures",update_mode:cleared?"clear":"replace",state:cleared?"cleared":"active",
      summary,structured_preview:null,
    });
    w.__submitNativeContext = (text: string, frameId = frame) => notice(text, false, frameId);
    w.__clearOtherNativeContext = () => w.__tauriEmit("agent", {
      kind:"AppContextUpdate",frame_id:frame,context_id:`mcp-app:${frame}:ui://other/view`,
      instance_id:`mcp-app:${frame}:ui://other/view`,app_name:"Other App",
      update_mode:"clear",state:"cleared",summary:"",structured_preview:null,
    });
    w.__TAURI__.core.invoke = async (command: string, args: any = {}) => {
      const result = await original(command, args);
      if ((command === "update_mcp_app_context" && args.context && Object.keys(args.context).length === 0)
        || (command === "close_mcp_app_child" && args.reason === "user_close")) notice("", true);
      return result;
    };
  }, frame);
  const turns = () => page.evaluate(() => (window as any).__skillInvokeLog.filter((c:any)=>c.cmd==="send_message").length);
  const before = await turns();
  const attachment = page.getByTestId("mcp-app-context-attachment");
  await page.evaluate(() => (window as any).__submitNativeContext("Selected figure A", "other-session"));
  await expect(attachment).toHaveCount(0);
  await page.evaluate(() => (window as any).__submitNativeContext("Selected figure A"));
  await expect(attachment).toContainText("MCP App context");
  await expect(attachment.locator(".composer-reference-card")).toHaveAttribute("title","Selected figure A");
  await page.evaluate(() => (window as any).__submitNativeContext("Selected figure B"));
  await expect(attachment).toHaveCount(1);
  await expect(attachment.locator(".composer-reference-card")).toHaveAttribute("title","Selected figure B");
  await page.evaluate(() => (window as any).__clearOtherNativeContext());
  await expect(attachment).toHaveCount(1);
  await expect(attachment.locator(".composer-reference-card")).toHaveAttribute("title","Selected figure B");
  await attachment.getByRole("button", {name:"Remove attachment"}).click();
  await expect(attachment).toHaveCount(0);
  await page.evaluate(() => (window as any).__submitNativeContext("Selected figure C"));
  await expect(attachment).toHaveCount(1);
  await page.locator('.center-tab[data-center-path^="mcp-app:"]').locator("..").locator(".center-tab-close").click();
  await expect(attachment).toHaveCount(0);
  expect(await turns()).toBe(before);
});
test("restored App binds fresh resources without replaying a tool call", async ({page}) => {
  const frame = await start(page);
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI__.core.invoke;
    w.__restoreCalls = [];
    w.__TAURI__.core.invoke = async (command: string, args: any) => {
      w.__restoreCalls.push(command);
      if (command === "prepare_mcp_app") return {
        tool:{name:"figures_open",title:"Restored figures"},
        resource:{uri:"ui://figures/view",text:"<body>fresh resource</body>"},
        arguments:{}, result:{content:[]}, _wispHistoricalResult:true,
      };
      return original(command, args);
    };
  });
  await page.evaluate(frame => (window as any).__tauriEmit("agent", {
    kind:"ToolPresentation", frame_id:frame, presentation_kind:"mcp_app",
    payload:{tool:{name:"figures_open",title:"Restored figures"},
      resource:{uri:"ui://figures/view",text:"<body>old resource</body>"},
      arguments:{},result:{content:[]},
      _wispMcpBinding:{version:1,frame_id:frame,project_id:"p",connector_id:"fixture"}},
  }), frame);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  const opened = (await calls(page,"open_mcp_app_child"))[0];
  expect(opened.args.payload.resource.text).toContain("fresh resource");
  await expect(page.locator(".mcp-app-isolation-controls")).toContainText("results are historical");
  expect(await page.evaluate(() => (window as any).__restoreCalls.filter((c:string)=>c === "mcp_app_call_tool"))).toHaveLength(0);
});

test("forced reconnect warns about unknown outcomes and needs confirmation", async ({page}) => {
  const frame = await start(page);
  await present(page,frame);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(1);
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI__.core.invoke;
    w.__restartCalls = [];
    w.__TAURI__.core.invoke = async (command:string,args:any) => {
      if (command === "restart_session_mcp") { w.__restartCalls.push(args); return null; }
      return original(command,args);
    };
  });
  page.once("dialog", async dialog => { expect(dialog.message()).toContain("will not be replayed"); await dialog.dismiss(); });
  await page.getByRole("button",{name:"Reconnect plugins",exact:true}).click();
  expect(await page.evaluate(() => (window as any).__restartCalls)).toHaveLength(0);
  page.once("dialog", dialog => dialog.accept());
  await page.getByRole("button",{name:"Reconnect plugins",exact:true}).click();
  await expect.poll(() => page.evaluate(() => (window as any).__restartCalls)).toHaveLength(1);
  await expect.poll(() => calls(page,"open_mcp_app_child")).toHaveLength(2);
  expect((await page.evaluate(() => (window as any).__restartCalls))[0].confirmOutcomeUnknown).toBe(true);
});
