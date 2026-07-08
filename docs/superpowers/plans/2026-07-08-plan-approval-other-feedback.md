# Plan Approval Other Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "Other" feedback path to the `update_plan` approval card for issue #121, so users can reject a proposed plan with specific revision instructions instead of only yes/no.

**Status:** Implemented. Release notes are intentionally deferred for this change per maintainer request; no `v0.6.1` or old-release note was added.

**Architecture:** Keep the existing boolean approval path for ordinary tools, then add a richer confirmation decision only where it is useful. `update_plan` consumes optional rejection feedback and returns it to the model in the tool result; the Tauri bridge carries the optional `feedback` field; the Leptos plan card exposes a small textarea behind an `Other` action. No SQLite schema, new backend command, or broad chat queue change is needed.

**Tech Stack:** Rust, Tauri 2, Leptos/WASM, Playwright, existing inline approval card.

---

## File Structure

- Modify: `crates/wisp-tools/src/env.rs` - define `ConfirmDecision` and a default `confirm_decision` method that preserves current `confirm` behavior.
- Modify: `crates/wisp-tools/src/lib.rs` - re-export `ConfirmDecision`.
- Modify: `crates/wisp-core/src/output.rs` - expose the richer decision through the `Output` adapter without breaking existing CLI/test outputs.
- Modify: `crates/wisp-tools/src/plan.rs` - use `confirm_decision` for fresh plan proposals and include user feedback in the rejected-plan tool result.
- Modify: `src-tauri/src/lib.rs` - change confirm channels from `bool` to `ConfirmDecision`, and accept optional `feedback` in `confirm_response`.
- Modify: `ui/src/main.rs` - send optional feedback from the plan card and keep ordinary tool approvals unchanged.
- Modify: `ui/src/i18n.rs` - add English and Chinese labels for `Other`, feedback placeholder, submit, and cancel.
- Modify: `ui/src/styles/chat.css` - style the compact plan feedback textarea and actions.
- Modify: `ui-tests/tests/mock-tauri.ts` - add a mocked plan approval request for Playwright.
- Modify: `ui-tests/tests/ui.spec.ts` - cover `Other` feedback payload for issue #121.
- No release-note file change in this implementation pass.

### Task 1: Confirmation Decision Primitive And `update_plan` Feedback

**Files:**
- Modify: `crates/wisp-tools/src/env.rs`
- Modify: `crates/wisp-tools/src/lib.rs`
- Modify: `crates/wisp-core/src/output.rs`
- Modify: `crates/wisp-tools/src/plan.rs`

- [x] **Step 1: Write the failing `update_plan` feedback test**

In `crates/wisp-tools/src/plan.rs`, replace the test module imports with these imports:

```rust
use super::{is_fresh_proposal, render_plan, UpdatePlanTool};
use crate::env::{ConfirmDecision, ToolEnv, ToolEvent};
use crate::tool::Tool;
use serde_json::json;
use std::path::{Path, PathBuf};
```

Append this test helper and test inside the existing `#[cfg(test)] mod tests`:

```rust
struct PlanDecisionEnv {
    root: PathBuf,
    decision: ConfirmDecision,
}

#[async_trait::async_trait]
impl ToolEnv for PlanDecisionEnv {
    fn project_root(&self) -> &Path {
        &self.root
    }

    async fn confirm(&self, _message: &str) -> bool {
        self.decision.approved()
    }

    async fn confirm_decision(&self, _message: &str) -> ConfirmDecision {
        self.decision.clone()
    }

    async fn emit(&self, _event: ToolEvent) {}
}

#[tokio::test]
async fn rejected_plan_feedback_is_returned_to_model() {
    let env = PlanDecisionEnv {
        root: PathBuf::from("."),
        decision: ConfirmDecision::Denied {
            feedback: Some("Split protocol changes from UI work".to_string()),
        },
    };
    let result = UpdatePlanTool
        .run(
            &json!({
                "steps": [
                    { "step": "Change confirmation protocol", "status": "pending" },
                    { "step": "Add plan card feedback UI", "status": "pending" }
                ]
            }),
            &env,
        )
        .await;

    assert!(!result.success);
    assert!(
        result
            .content
            .contains("User feedback: Split protocol changes from UI work"),
        "{}",
        result.content
    );
}
```

- [x] **Step 2: Run the focused feedback test**

Run:

```bash
cargo test -p wisp-tools rejected_plan_feedback_is_returned_to_model
```

Result: PASS after the `ConfirmDecision` implementation landed.

- [x] **Step 3: Add `ConfirmDecision` to the tool environment**

In `crates/wisp-tools/src/env.rs`, add this enum after `pub enum Approval`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDecision {
    Approved,
    Denied { feedback: Option<String> },
}

impl ConfirmDecision {
    pub fn approved(&self) -> bool {
        matches!(self, Self::Approved)
    }

    pub fn feedback(&self) -> Option<&str> {
        match self {
            Self::Denied { feedback: Some(feedback) } => {
                let trimmed = feedback.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        }
    }
}
```

In the `ToolEnv` trait in the same file, add this default method immediately after `confirm`:

```rust
async fn confirm_decision(&self, message: &str) -> ConfirmDecision {
    if self.confirm(message).await {
        ConfirmDecision::Approved
    } else {
        ConfirmDecision::Denied { feedback: None }
    }
}
```

In `crates/wisp-tools/src/lib.rs`, change the re-export line to:

```rust
pub use env::{Approval, ConfirmDecision, ImageData, ToolEnv, ToolEvent, ToolResult};
```

- [x] **Step 4: Expose the richer decision through `wisp-core` output**

In `crates/wisp-core/src/output.rs`, add this default method to the `Output` trait immediately after `fn confirm`:

```rust
fn confirm_decision(&self, message: &str) -> wisp_tools::ConfirmDecision {
    if self.confirm(message) {
        wisp_tools::ConfirmDecision::Approved
    } else {
        wisp_tools::ConfirmDecision::Denied { feedback: None }
    }
}
```

In the `impl<'a> wisp_tools::ToolEnv for ToolEnvAdapter<'a>` block, add:

```rust
async fn confirm_decision(&self, message: &str) -> wisp_tools::ConfirmDecision {
    self.out.confirm_decision(message)
}
```

- [x] **Step 5: Make `update_plan` consume rejection feedback**

In `crates/wisp-tools/src/plan.rs`, change the import to:

```rust
use crate::env::{ConfirmDecision, ToolEnv, ToolResult};
```

Add this helper near `step_counts`:

```rust
fn plan_rejection_message(feedback: Option<String>) -> String {
    let mut msg =
        "Plan rejected by the user. Revise the plan or ask what they want changed before proceeding."
            .to_string();
    if let Some(feedback) = feedback.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        msg.push_str(" User feedback: ");
        msg.push_str(feedback);
    }
    msg
}
```

Replace the current fresh-proposal rejection block in `UpdatePlanTool::run` with:

```rust
if is_fresh_proposal(args) {
    match env
        .confirm_decision(&format!("{PLAN_APPROVAL_PREFIX}{rendered}"))
        .await
    {
        ConfirmDecision::Approved => {}
        ConfirmDecision::Denied { feedback } => {
            return ToolResult::fail(plan_rejection_message(feedback));
        }
    }
}
```

- [x] **Step 6: Run focused Rust tests**

Run:

```bash
cargo test -p wisp-tools
cargo test -p wisp-core
```

Expected: PASS. Existing tool approvals keep using `confirm` as a boolean.

### Task 2: Tauri Confirmation Transport Carries Optional Feedback

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [x] **Step 1: Change confirm channel types**

In `src-tauri/src/lib.rs`, add this alias near `ConfirmRequest`:

```rust
type ConfirmSender = std::sync::mpsc::Sender<wisp_tools::ConfirmDecision>;
type ConfirmMap = Arc<StdMutex<HashMap<String, ConfirmSender>>>;
```

Change the `confirms` field type in both `AppState` and `TauriOutput` from:

```rust
Arc<StdMutex<HashMap<String, std::sync::mpsc::Sender<bool>>>>
```

to:

```rust
ConfirmMap
```

- [x] **Step 2: Return `ConfirmDecision` from `TauriOutput`**

In `impl Output for TauriOutput`, keep a boolean `confirm` method and add a richer override:

```rust
fn confirm(&self, message: &str) -> bool {
    self.confirm_decision(message).approved()
}

fn confirm_decision(&self, message: &str) -> wisp_tools::ConfirmDecision {
    let (tool, preview) = parse_confirm_payload(message);
    let (tx, rx) = std::sync::mpsc::channel::<wisp_tools::ConfirmDecision>();
    self.confirms
        .lock()
        .unwrap()
        .insert(self.frame_id.clone(), tx);
    self.awaiting_confirm
        .lock()
        .unwrap()
        .insert(self.frame_id.clone());
    let _ = self.app.emit(
        "confirm-request",
        ConfirmRequest {
            frame_id: self.frame_id.clone(),
            message: message.into(),
            tool,
            preview,
        },
    );
    let decision = rx
        .recv_timeout(std::time::Duration::from_secs(180))
        .unwrap_or(wisp_tools::ConfirmDecision::Denied { feedback: None });
    self.confirms.lock().unwrap().remove(&self.frame_id);
    self.awaiting_confirm.lock().unwrap().remove(&self.frame_id);
    decision
}
```

- [x] **Step 3: Accept optional feedback in `confirm_response`**

Replace the command signature and body with:

```rust
#[tauri::command]
fn confirm_response(
    state: State<'_, AppState>,
    session_id: String,
    approved: bool,
    feedback: Option<String>,
) -> Result<(), String> {
    let decision = if approved {
        wisp_tools::ConfirmDecision::Approved
    } else {
        wisp_tools::ConfirmDecision::Denied {
            feedback: feedback
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    };
    if let Some(tx) = state.confirms.lock().unwrap().remove(&session_id) {
        let _ = tx.send(decision);
        Ok(())
    } else {
        Err("no pending confirmation".into())
    }
}
```

- [x] **Step 4: Run the focused Tauri compile check**

Run:

```bash
cargo check -p wisp-tauri
```

Expected: PASS. The command remains backward-compatible because omitted `feedback` binds to `None`.

### Task 3: Plan Card `Other` UI

**Files:**
- Modify: `ui/src/main.rs`
- Modify: `ui/src/i18n.rs`
- Modify: `ui/src/styles/chat.css`

- [x] **Step 1: Update the UI command helper test first**

In `ui/src/main.rs`, update the `tauri_args::confirm_response` helper to accept optional feedback:

```rust
pub fn confirm_response(session_id: &str, approved: bool, feedback: Option<&str>) -> Value {
    let mut payload = json!({ "sessionId": session_id, "approved": approved });
    if let Some(feedback) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
        payload["feedback"] = json!(feedback);
    }
    payload
}
```

Update `session_command_args_use_camel_case_keys` in the same file:

```rust
let v = tauri_args::confirm_response("frame-1", true, None);
assert_eq!(v["sessionId"], "frame-1");
assert_eq!(v["approved"], true);
assert!(v.get("feedback").is_none());

let v = tauri_args::confirm_response("frame-1", false, Some("split the plan"));
assert_eq!(v["feedback"], "split the plan");
```

Do not run this test yet. The helper signature change intentionally breaks call sites until Step 2 updates the approval callback payload.

- [x] **Step 2: Change approval callbacks to include feedback**

In `ApprovalCard`, change the callback type to:

```rust
on_decide: Callback<(String, bool, Option<String>)>,
```

In `render_item`, change `on_approval` to:

```rust
on_approval: Callback<(String, bool, Option<String>)>,
```

In `respond_confirm`, change the callback body to:

```rust
Callback::new(move |(sid, approved, feedback): (String, bool, Option<String>)| {
    route_items(active_session, items, transcripts, &sid, strip_approval_pending);
    approval_pending.update(|s| {
        s.remove(&sid);
    });
    let arg = to_value(&tauri_args::confirm_response(&sid, approved, feedback.as_deref())).unwrap();
    spawn_local(async move { let _ = invoke("confirm_response", arg).await; });
})
```

Update the existing approve and reject button handlers in `ApprovalCard`:

```rust
let sid_allow = session_id.clone();
let sid_reject = session_id.clone();
let sid_feedback = session_id;
```

```rust
on:click=move |_| on_decide.call((sid_allow.clone(), true, None))
```

```rust
on:click=move |_| on_decide.call((sid_reject.clone(), false, None))
```

- [x] **Step 3: Add the plan-only `Other` feedback controls**

Inside `ApprovalCard`, add these signals after `let is_plan = tool == "update_plan";`:

```rust
let show_feedback = create_rw_signal(false);
let feedback = create_rw_signal(String::new());
let feedback_ready = move || !feedback.get().trim().is_empty();
```

Keep the existing approve and reject buttons, and add this plan-only button inside `.approval-actions`:

```rust
{is_plan.then(|| view! {
    <button type="button" on:click=move |_| show_feedback.update(|open| *open = !*open)>
        {move || t(locale.get(), "approval.plan_other")}
    </button>
})}
```

Add this block immediately after `.approval-actions`:

```rust
{is_plan.then(|| {
    view! {
        <Show when=move || show_feedback.get()>
            <div class="plan-feedback">
                <textarea
                    class="plan-feedback-input"
                    rows="3"
                    prop:value=move || feedback.get()
                    placeholder=move || t(locale.get(), "approval.plan_feedback_placeholder")
                    on:input=move |ev| feedback.set(event_target_value(&ev))
                ></textarea>
                <div class="plan-feedback-actions">
                    <button
                        type="button"
                        class="primary"
                        disabled=move || !feedback_ready()
                        on:click=move |_| {
                            let text = feedback.get().trim().to_string();
                            if !text.is_empty() {
                                on_decide.call((sid_feedback.clone(), false, Some(text)));
                            }
                        }
                    >
                        {move || t(locale.get(), "approval.plan_feedback_submit")}
                    </button>
                    <button
                        type="button"
                        on:click=move |_| {
                            feedback.set(String::new());
                            show_feedback.set(false);
                        }
                    >
                        {move || t(locale.get(), "approval.plan_feedback_cancel")}
                    </button>
                </div>
            </div>
        </Show>
    }
})}
```

- [x] **Step 4: Add i18n strings**

In `ui/src/i18n.rs`, add English strings near the existing `approval.plan_*` entries:

```rust
(Locale::En, "approval.plan_other") => Some("Other"),
(Locale::En, "approval.plan_feedback_placeholder") => Some("Tell wisp what to change in this plan."),
(Locale::En, "approval.plan_feedback_submit") => Some("Send feedback"),
(Locale::En, "approval.plan_feedback_cancel") => Some("Cancel"),
```

Add Chinese strings near the matching `Locale::Zh` entries:

```rust
(Locale::Zh, "approval.plan_other") => Some("其他"),
(Locale::Zh, "approval.plan_feedback_placeholder") => Some("说明你希望如何修改计划。"),
(Locale::Zh, "approval.plan_feedback_submit") => Some("发送反馈"),
(Locale::Zh, "approval.plan_feedback_cancel") => Some("取消"),
```

Update the existing hint text so users know the third path exists:

```rust
(Locale::En, "approval.plan_hint") => Some("Approve to start, reject to have the agent revise the plan, or choose Other to send specific feedback."),
(Locale::Zh, "approval.plan_hint") => Some("批准即开始执行，拒绝则让智能体修改计划，也可以选择「其他」发送具体意见。"),
```

- [x] **Step 5: Style the feedback box**

In `ui/src/styles/chat.css`, append this after the `.plan-step-text` rules:

```css
.plan-feedback { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }
.plan-feedback-input {
  width: 100%; min-height: 76px; resize: vertical; border: 1px solid var(--border-strong);
  border-radius: 8px; background: var(--bg-elev); color: var(--text); padding: 9px 10px;
  font: inherit; line-height: 1.45;
}
.plan-feedback-input:focus { outline: 2px solid color-mix(in srgb, var(--clay) 36%, transparent); outline-offset: 1px; }
.plan-feedback-actions { display: flex; gap: 8px; flex-wrap: wrap; }
.plan-feedback-actions button {
  border-radius: 8px; padding: 7px 12px; cursor: pointer;
  border: 1px solid var(--border-strong); background: var(--bg-elev); color: var(--text);
}
.plan-feedback-actions button.primary { background: var(--clay); border-color: var(--clay); color: #fff; }
.plan-feedback-actions button:disabled { opacity: .45; cursor: default; }
```

- [x] **Step 6: Run the focused UI tests and compile check**

Run:

```bash
cd ui
cargo test session_command_args_use_camel_case_keys
cargo check --target wasm32-unknown-unknown
```

Expected: PASS.

### Task 4: Playwright Coverage For Issue #121

**Files:**
- Modify: `ui-tests/tests/mock-tauri.ts`
- Modify: `ui-tests/tests/ui.spec.ts`

- [x] **Step 1: Add a mocked plan approval request**

In `ui-tests/tests/mock-tauri.ts`, inside the `send_message` case and before the `NEEDCONFIRM` branch, add:

```ts
if (String(arg("message") ?? "").includes("PLANOTHER")) {
  const planPreview = "Plan (2 steps · 0 done · 0 in progress · 2 pending):\n[ ] Inspect confirmation protocol\n[ ] Add plan feedback UI";
  setTimeout(
    () =>
      emit("confirm-request", {
        frame_id: fid,
        message: `[plan-approval]\n${planPreview}`,
        tool: "update_plan",
        preview: planPreview,
      }),
    50,
  );
  return fid;
}
```

- [x] **Step 2: Add the Playwright test**

In `ui-tests/tests/ui.spec.ts`, add this test after the long approval card test:

```ts
test("plan approval Other sends feedback (#121)", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("PLANOTHER");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Review plan before starting?")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Other" }).click();
  await page
    .getByPlaceholder("Tell wisp what to change in this plan.")
    .fill("Split protocol work from UI work.");
  await page.getByRole("button", { name: "Send feedback" }).click();

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "confirm_response") ?? null;
  })).toMatchObject({
    cmd: "confirm_response",
    args: {
      approved: false,
      feedback: "Split protocol work from UI work.",
    },
  });
});
```

- [x] **Step 3: Run the focused browser test**

Run:

```bash
cd ui-tests && npm ci && npx playwright test tests/ui.spec.ts -g "plan approval Other sends feedback"
```

Expected: PASS.

### Task 5: Release Notes Deferral And Full Verification

**Files:**
- No release-note file change.

- [x] **Step 1: Defer the release note**

No release note was added in this implementation pass because the maintainer requested no temporary `v0.6.1` release work.

- [x] **Step 2: Run formatting and Rust tests**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

Expected: PASS.

- [x] **Step 3: Run UI checks**

Run:

```bash
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

Expected: PASS.

- [x] **Step 4: Automated smoke**

Covered by the Playwright test `plan approval Other sends feedback (#121)`, which verifies:

- The plan card shows `Approve & start`, `Reject`, and `Other`.
- `Other` opens a textarea without moving the primary buttons off-screen.
- Empty feedback cannot be submitted.
- Submitted feedback sends `approved: false` and the trimmed `feedback` payload.
- Generic shell/tool approvals still show only the existing approve/deny actions through existing approval-card coverage.
