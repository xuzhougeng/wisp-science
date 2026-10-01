import { test, expect, type Locator, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeEnabled();
});

async function selection(input: Locator) {
  return input.evaluate((el: HTMLTextAreaElement) => ({
    start: el.selectionStart,
    end: el.selectionEnd,
    direction: el.selectionDirection,
  }));
}

async function sentMessages(page: Page) {
  return page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((call: any) => call.cmd === "send_message")
    .map((call: any) => call.args instanceof Map ? Object.fromEntries(call.args) : call.args));
}

test("composer rejects leaked arrow controls before insertion at any caret position", async ({ page }) => {
  const input = page.locator("#composer-input");
  for (const text of ["", "中文🧬结果"]) {
    await input.fill(text);
    for (const caret of [0, 2, text.length].filter((offset) => offset <= text.length)) {
      for (const data of ["\u001c", "\u001d", "\u001e", "\u001f", "\u001c\u001c"]) {
        const canceled = await input.evaluate((el: HTMLTextAreaElement, { caret, data }) => {
          el.setSelectionRange(caret, caret);
          const event = new InputEvent("beforeinput", {
            bubbles: true, cancelable: true, inputType: "insertText", data,
          });
          return !el.dispatchEvent(event);
        }, { caret, data });
        expect(canceled).toBe(true);
        await expect(input).toHaveValue(text);
        expect(await selection(input)).toMatchObject({ start: caret, end: caret });
      }
    }
  }
});

test("composer repairs repeated input leaks without beforeinput or event data", async ({ page }) => {
  const input = page.locator("#composer-input");
  const text = "中文🧬结果";
  await input.fill(text);
  for (const caret of [0, 2, text.length]) {
    for (const genericEvent of [false, true]) {
      for (let repeat = 0; repeat < 3; repeat++) {
        await input.evaluate((el: HTMLTextAreaElement, { caret, genericEvent }) => {
          el.setSelectionRange(caret, caret);
          // Model the native editor mutation: the input event itself is not
          // cancelable and has no data, as can happen in a WebView.
          el.setRangeText("\u001c\u001d\u001e\u001f", caret, caret, "end");
          el.dispatchEvent(genericEvent
            ? new Event("input", { bubbles: true })
            : new InputEvent("input", { bubbles: true, inputType: "insertText" }));
        }, { caret, genericEvent });
        await expect(input).toHaveValue(text);
        expect(await selection(input)).toMatchObject({ start: caret, end: caret });
      }
    }
  }
});

test("composer keeps mixed pasted text and adjusts a backward UTF-16 selection", async ({ page }) => {
  const input = page.locator("#composer-input");
  const clean = "中🧬文\talpha\n👩‍🔬 e\u0301";
  const dirty = "\u001c中🧬\u001d文\talpha\n👩‍🔬 e\u0301\u001e\u001f";
  const canceled = await input.evaluate((el: HTMLTextAreaElement, dirty) => {
    const before = new InputEvent("beforeinput", {
      bubbles: true, cancelable: true, inputType: "insertFromPaste", data: dirty,
    });
    const canceled = !el.dispatchEvent(before);
    if (!canceled) {
      el.value = dirty;
      el.setSelectionRange(2, 6, "backward");
      el.dispatchEvent(new InputEvent("input", {
        bubbles: true, inputType: "insertFromPaste", data: dirty,
      }));
    }
    return canceled;
  }, dirty);
  expect(canceled).toBe(false);
  await expect(input).toHaveValue(clean);
  expect(await selection(input)).toEqual({ start: 1, end: 4, direction: "backward" });
  await input.press("Enter");
  await expect.poll(() => sentMessages(page)).toMatchObject([{ message: clean }]);
});

test("composer preserves IME marked text until commit and never sends on confirmation", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.focus();
  const dirty = "中\u001d文🧬";
  await input.dispatchEvent("compositionstart", { data: "" });
  const canceled = await input.evaluate((el: HTMLTextAreaElement, dirty) => {
    const canceled = !el.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, cancelable: true, inputType: "insertCompositionText",
      data: dirty, isComposing: true,
    }));
    el.value = dirty;
    el.setSelectionRange(3, 3);
    el.dispatchEvent(new InputEvent("input", {
      bubbles: true, inputType: "insertCompositionText", data: dirty, isComposing: true,
    }));
    return canceled;
  }, dirty);
  expect(canceled).toBe(false);
  await expect(input).toHaveValue(dirty);
  await input.dispatchEvent("keydown", { key: "Enter", keyCode: 229, isComposing: true });
  expect(await sentMessages(page)).toEqual([]);
  await input.dispatchEvent("compositionend", { data: dirty });
  await expect(input).toHaveValue("中文🧬");
  expect(await selection(input)).toMatchObject({ start: 2, end: 2 });
  await input.press("Enter");
  await expect.poll(() => sentMessages(page)).toMatchObject([{ message: "中文🧬" }]);
});

test("composer retains native boundary navigation, selection, modifiers, and newlines", async ({ page }) => {
  const input = page.locator("#composer-input");
  const text = "中文🧬 result";
  await input.fill(text);
  await input.evaluate((el: HTMLTextAreaElement) => el.setSelectionRange(0, 0));
  for (let repeat = 0; repeat < 20; repeat++) await page.keyboard.down("ArrowLeft");
  await page.keyboard.up("ArrowLeft");
  expect(await selection(input)).toMatchObject({ start: 0, end: 0 });
  await input.press("ArrowRight");
  expect(await selection(input)).toMatchObject({ start: 1, end: 1 });
  await input.press("Shift+ArrowRight");
  expect(await selection(input)).toMatchObject({ start: 1, end: 2 });
  await input.press("ArrowLeft");
  expect(await selection(input)).toMatchObject({ start: 1, end: 1 });
  await input.evaluate((el: HTMLTextAreaElement) => el.setSelectionRange(el.value.length, el.value.length));
  for (let repeat = 0; repeat < 20; repeat++) await page.keyboard.down("ArrowRight");
  await page.keyboard.up("ArrowRight");
  expect(await selection(input)).toMatchObject({ start: text.length, end: text.length });
  await input.press(process.platform === "darwin" ? "Alt+ArrowLeft" : "Control+ArrowLeft");
  expect((await selection(input)).start).toBeLessThan(text.length);
  await expect(input).toHaveValue(text);
  await input.press("Shift+Enter");
  await expect(input).toHaveValue("中文🧬 \nresult");
  expect(await sentMessages(page)).toEqual([]);
});

for (const attachment of [false, true]) {
  test(`composer sends cleaned text${attachment ? " with an attachment" : ""}`, async ({ page }) => {
    const input = page.locator("#composer-input");
    if (attachment) {
      await page.setInputFiles("#composer-file-input", {
        name: "counts.csv", mimeType: "text/csv", buffer: Buffer.from("a,b\n1,2"),
      });
      await expect(page.locator(".composer-attachment.ready")).toHaveText("counts.csv");
    }
    await input.fill("\u001c\u001c现在\u001d比较结果\u001e\u001f");
    await expect(input).toHaveValue("现在比较结果");
    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect.poll(() => sentMessages(page)).toMatchObject([{
      message: attachment ? "现在比较结果\n\nUploaded files: uploads/counts.csv" : "现在比较结果",
      attachments: attachment ? ["uploads/counts.csv"] : [],
    }]);
    await expect(page.locator(".msg.user")).toContainText("现在比较结果");
    await expect(page.locator(".msg.user")).not.toContainText(/[\u001c-\u001f]/);
  });
}

test("composer does not send a draft containing only leaked controls", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.fill("\u001c\u001d\u001e\u001f");
  await expect(input).toHaveValue("");
  await input.press("Enter");
  await expect(page.getByRole("button", { name: "Send", exact: true })).toHaveClass(/is-empty/);
  await page.getByRole("button", { name: "Send", exact: true }).click();
  expect(await sentMessages(page)).toEqual([]);
});

test("composer sanitizes programmatic drafts before empty checks and slash commands", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.fill("seed");
  await input.press("Enter");
  await expect(page.getByTestId("follow-up-questions")).toBeVisible();
  const [first] = await sentMessages(page);

  // Follow-up selection writes directly to the draft signal, bypassing all
  // DOM input events, just like restoring a saved draft/rewound message.
  const prefill = async (text: string) => {
    await page.evaluate(({ frameId, text }) => {
      (window as any).__tauriEmit("agent", {
        kind: "FollowUps", frame_id: frameId, questions: [text, "Second", "Third"],
      });
    }, { frameId: first.sessionId, text });
    await page.locator(".follow-up-options button").first().click();
    await expect(input).toHaveValue(text);
  };

  await prefill("\u001c\u001d\u001e\u001f");
  await input.press("Enter");
  expect(await sentMessages(page)).toHaveLength(1);

  await prefill("\u001c/plan\u001d");
  await input.press("Enter");
  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((call: any) => call.cmd === "set_session_plan_mode").length)).toBe(1);
  expect(await sentMessages(page)).toHaveLength(1);

  await prefill("\u001c恢复\u001d正文\u001f");
  await input.press("Enter");
  await expect.poll(() => sentMessages(page)).toMatchObject([
    { message: "seed" }, { message: "恢复正文" },
  ]);
});
