# Model configuration

In General settings, **Suggest follow-up questions** is enabled by default.
After a completed reply, Wisp uses that conversation's current model to offer
three optional next questions. That secondary call reads only the four most
recent user turns, so completing a long, tool-heavy conversation does not load
and duplicate its full history. The suggestion panel can be hidden per reply,
or the setting can be turned off to skip the extra model call entirely. Wisp
does not request suggestions for failed, cancelled, paused, or tool-only turns;
the current turn must contain a visible final answer.

The desktop transcript also treats ordinary tool output as a preview: tool
results are limited to 4,000 characters and streamed terminal output to 64 KiB
(with carriage-return progress updates folded in place). Complete answer, plan,
and question cards are not clipped. The durable model transcript is unchanged;
these limits apply to the WebView presentation and its replay data.

Long conversations use the same bounded tail whether they stay open or are
reopened. After a live transcript grows past 40 completed user turns, the
WebView unloads older reactive rows and keeps the latest 20; **Load earlier
messages** restores the durable history from SQLite. This presentation limit
does not change model context, exports, artifacts, or the saved transcript.

wisp-science calls remote LLM APIs through model profiles. Desktop users
configure these in **Settings -> Models**. **Add API access** is bring-your-own-key:
enter the shared **Base URL** and API key first, then add every chat and image
model that key can call. Each model row independently selects its **Protocol**,
model ID, optional endpoint suffix, display name, and capabilities. Models
created in one **Add API access** form share that form's Base URL and key, even
when their protocols or endpoint suffixes differ. Leave the key blank to reuse
a key already stored for that Base URL. Paste a different key to keep the new
models on a separate credential — the same host can have several keys, each
with its own model batch. Use distinct **display names** when two batches use
the same model IDs. Changing a stored key on one profile updates other profiles
that currently share that same key and Base URL; it does not overwrite a
different key on the same host.
A models.dev catalog baked in at build time maps exact model IDs to the
vendor's documented ceilings: for a catalog-known model the form auto-fills
**Max output tokens** and **Context window** and shows the ceiling next to the
inputs. Saving a max-output value above the documented ceiling is rejected with
an inline error instead of failing mid-turn with a provider 400, and a context
window above the ceiling is clamped down on save. Models absent from the
catalog keep manually entered values.

When editing a chat model, **Max output tokens** and **Context window** support
continuous typing and deletion without losing focus. Changes take effect when
you save the profile; the same catalog ceilings still apply.

The composer model picker binds the selected HTTP model to the current
conversation. Switching one populated conversation asks for confirmation and
does not change any other conversation. Empty conversations switch immediately
without a warning. The active profile in Settings remains the default for new
conversations. In **Settings → AI configuration → Models**, choose **Set as
default** to save that default immediately; **Default** identifies the selected
profile. The list has no Save/Cancel footer. Add/edit forms still require Save,
and cancelling an edit does not undo an earlier default change. Hovering a model row in the picker overlays the reasoning effort
inside the model information area without reserving a separate column, and
reveals its **Edit** button. The button opens a flyout to the right of the model
menu listing the effort levels the model family is documented to support (same
curated list as the model form in Settings).
Choosing a level saves it as the profile's default — it applies to every
conversation using that model and is not scoped to the current conversation.
Choosing "default" clears the value so the provider decides.

The add/edit model page includes collapsed **Request headers (advanced)**
(请求附加信息（高级）). It explains that these settings add HTTP request headers
to requests sent to the model service, including connection validation. Keep the
defaults unless the service requires specific values. Each header has its own
explanation, editable field, and live example of `Header-Name: value`; switching
it off replaces the example with an explicit “will not be sent” state.

**Send client identifier (User-Agent)** is on by default. The **Client identifier
(User-Agent value)** field lets you customize the application identifier sent
to the service.
Turn the switch off to omit the header entirely. While enabled, leave the field
blank to send `wisp-science`, or enter a client
identifier such as `research-client/1.0` using printable ASCII characters.
The value is saved per model and used for connection validation, chat and vision
requests, auxiliary model calls, and image/video model requests. When adding
several models under one API access, each receives the entered value; you can
then edit them independently. Clearing the field restores the default.

For **OpenCode**, choose the **OpenCode** preset in Settings → Models. It
prefills only the Base URL, defaulting to Go at
`https://opencode.ai/zen/go/v1`. To use Zen, change it to
`https://opencode.ai/zen/v1`.

The model row starts empty. Paste your API key, enter the model ID you want,
and select its API protocol before validating and saving. You can add more
rows yourself. Wisp does not preconfigure an OpenCode model list or a fixed
model-to-protocol mapping; changing between Go and Zen preserves your entries.
Consult the current [Go](https://opencode.ai/docs/go/#api-endpoints) or
[Zen](https://opencode.ai/docs/zen/#endpoints) endpoint documentation for the
model ID and protocol supported by your chosen service.

**ChatGPT Plus/Pro (Codex)** is a subscription login, separate from an OpenAI
Platform API key. In **Settings → Models**, choose **ChatGPT Plus/Pro**.
Browser sign-in opens ChatGPT and listens on `127.0.0.1:1455` for the
redirect. Device-code sign-in shows a one-time code and works when the browser
cannot reach this machine, including SSH and WSL. Paste the final redirect URL
if the local callback does not arrive. Wisp stores the access token, refresh
token, expiry, and ChatGPT account id in the OS keyring, refreshes the access
token automatically, and sends chat requests to
`https://chatgpt.com/backend-api/codex/responses`. The model ID is whatever
that subscription can call, such as `gpt-5.5`. A sign-in from
`wisp-science login codex` is stored on the same machine and can be reused
from this page.

**SuperGrok / X Premium+ (xAI)** is also a subscription login, separate from an
xAI API key. In **Settings → Models**, choose **SuperGrok**. Wisp starts an
xAI device-code sign-in and opens the `accounts.x.ai` page. Approve it there,
and enter the one-time code if the page asks for it. This works over SSH and
WSL because no local callback is needed. Wisp stores the access token, refresh
token, expiry, and xAI token endpoint in the OS keyring, refreshes the access
token automatically, and sends chat requests to
`https://api.x.ai/v1/chat/completions`. The token is only ever sent to HTTPS
hosts on `x.ai`. The default model is `grok-4.6`. xAI may refuse some
subscription tiers with HTTP 403 even after a successful sign-in. In that case,
use an xAI API key instead. A sign-in from `wisp-science login xai` can be
reused from this page.

Use the bare API model ID, without OpenCode's client-side provider prefix.
Context and output ceilings continue to come from Wisp's baked models.dev
catalog, using exact model IDs and the appropriate Go or Zen namespace.

**Send conversation identifier** controls the session header for every HTTP model. It defaults
to enabled for OpenCode's `/zen` routes and disabled elsewhere. You can explicitly
enable or disable it per model, including custom gateways. **Header name for the conversation ID**
defaults to `x-opencode-session` when blank; enter another valid HTTP header name
if your service requires one. Authentication and transport header names cannot
be replaced. Turning either sending switch off retains its configured text but
omits that header from model requests and connection validation.

The session header's value is generated by Wisp, not entered in the model form.
Desktop conversations retain their ID across turns, retries, model switches,
and restarts. Vision, compaction, Reader, Reviewer, and other conversation
helpers carry the same identity; independent child conversations have their
own IDs. Connection validation uses a temporary identity for that operation.
CLI/RPC retain theirs in `.wisp/session-id` alongside `.wisp/session.json`;
CLI `/new` rotates it. A local HTTP proxy does not change these identities.
For a custom gateway, enable **Send conversation identifier** and set the header name expected
by that gateway. The same controls apply to image/video model validation,
generation, and status polling; session headers are not attached to generated
media download URLs.

Leave User-Agent blank to identify the client as `wisp-science`. Do not enter
`x-opencode-session` in that field: it changes only User-Agent, not other HTTP
headers. Configure session sending separately; API keys continue to use the
existing keyring storage.

In the model editor, reasoning guidance sits directly below its selector.
Image input and analysis share a group; image and video generation each have
their own group with the explanation below the checkbox. These groups stack
vertically in narrow windows.

OpenAI Chat Completions and Responses profiles also have a **Fast mode** toggle
on the model form, next to reasoning effort. Off uses the provider default and
omits the field; on sends top-level `service_tier: "priority"` on ordinary
turns, tool calls, retries, and continue-generation. It is independent of
reasoning effort and may increase quota usage. The model toggle is the default
for new conversations. A lightning button beside the composer model picker
shows the effective state and stores an independent per-conversation override;
turning it back to the profile default clears that override. The button is
disabled during a running turn and hidden for unsupported providers and ACP
Agents. ACP Fast Mode remains a separate Agent session configuration.

The built-in Reader used by `#` session references inherits that profile's
model, not its reasoning effort. Retrieval turns disable DeepSeek thinking
(the V4 default is thinking-on at `high`) and cap each transcript chunk well
below a 1M context window so a long session from another project cannot fill
a single JSON-extraction call. The first pass is still capped at 2048 output
tokens; if the JSON is truncated or lands in a thinking field, Reader retries
once with the profile's full output budget and parses the last complete JSON
object from either field. If structured retrieval still fails, a head-and-tail
excerpt of the cited transcript is injected so the main turn can still use the
reference.

Model profiles describe model access and capabilities for the **built-in Wisp
agent**. External coding agents (Codex / Claude via ACP) are configured under
**Settings → Models → ACP Agents** — see [ACP Agents](acp-agents.md). Do not put
an ACP launch command in an HTTP model profile.

For image workflows, mark an API profile as **Supports image input** and
optionally **Use for image analysis**. Image attachments are sent directly to a
visual input model. When the input model is non-visual, Wisp first calls the
assigned vision model and passes its text observations to the input model.
`view_image` and image reads use the assigned vision model in the same way.
Raster image input supports PNG, JPEG, GIF, and WebP. New attachments and image
reads are decoded and their actual format is checked before model input. Files
up to 5 MiB with both sides at most 2048 pixels keep their original bytes.
Images with a longer side automatically receive a proportional JPEG input copy
bounded to 2048 pixels, even when the compressed file is small. For files above
5 MiB, Wisp pauses before the model request and asks
whether to create a temporary JPEG input copy with a longest edge of 2048
pixels. The project file is never modified, and the confirmation warns that
fine details may be lost. Automatic resizing also includes a size-change and
detail-loss notice in the image's model-visible label. Inspect smaller crops
when fine details matter. Invalid image data is rejected locally. Source images
above 50 MiB remain rejected. This applies to new image input, not images already
stored in conversation history.

When switching a populated conversation to a non-visual model, the confirmation
explains that previously sent images will be omitted from future requests to
that model. This substitution happens only while preparing the API request; it
does not delete or rewrite the saved conversation. A new image attached after
the switch is analyzed through the assigned vision model. Without an assigned
vision model, Wisp rejects that new image before starting the main model turn.

Image generation is a separate model role. Create an OpenAI-compatible profile
with the exact model ID supplied by your provider, then enable **Use for
image generation**. Custom IDs and gateway aliases (for example `gpt-image-2.5`
or `vendor/custom-image-v3`) are accepted without a Wisp model-name allowlist;
availability and supported parameters remain the provider's responsibility.
Known model names are only auto-selection hints, not acceptance gates.
The edit form for image profiles hides chat-only fields
(max output tokens, context window, reasoning effort, and vision) and shows
image defaults instead: size and quality for OpenAI-compatible models, or aspect ratio,
resolution, and quality for `grok-imagine-image-2.0`. `generate_image` uses
those defaults when a request does not specify size or quality. Custom IDs use
a minimal Images API request by default; GPT-Image-specific `output_format`
and automatic size/quality values are not forced onto unknown models.
For xAI, use
Base URL `https://api.x.ai` and the OpenAI Chat Completions protocol. The built-in **Scientific Illustrator** calls the
provider's Image API (`/images/generations`) and saves a PNG under `figures/`
when that role is assigned and PNG or image-model generation is requested. An
explicit SVG/vector/editable request always uses the specialist's direct-SVG
path, even when an image model is configured: it writes SVG, renders that
exact SVG to a PNG preview, inspects the preview, and iterates on the SVG.
An explicit PNG request requires the configured image-generation model; it is
not silently replaced with SVG. The configured generation tool is also
available in ordinary built-in-agent
conversations, so a direct request for the Scientific Illustrator, `gpt-image-2`,
or `grok-imagine-image-2.0` can generate the image without preselecting the
specialist. While the request runs, the conversation shows an image placeholder
and replaces it with the generated PNG. When the user does not specify a
format, the specialist uses the assigned image-generation profile to create
PNG if present. Otherwise it uses the same SVG -> PNG preview -> SVG
correction workflow and delivers SVG under `figures/`. Image-only profiles do
not appear in chat, Reviewer, specialist, delegation, or side-chat model
pickers.

The persistent `image_generation_capable` role is separate from the currently
assigned image profile: deselecting an image profile does not accidentally make
it a chat model. Renaming a profile to a different model resets the old role
unless image generation is explicitly selected for the new ID. Existing
profiles without the new marker retain backwards-compatible known-name and
assignment hints. Chat catalog limits still use exact model-ID matching.

The **Validate** action sends the form's image role to the backend and performs
authenticated `GET /models/{id}` (falling back to `GET /models` when the
individual lookup is unavailable). It never probes image-only models with
`/chat/completions` or `/responses`, and does not generate a billable image.
Successful metadata validation confirms model visibility, not successful image
generation. A gateway without compatible model metadata may still fail this
non-generating check; Wisp does not hide that failure or fabricate success.

An image-generation assignment does not also provide image analysis.
These image models may consume an input image for editing, but their Image API
returns generated pixels rather than the textual observations required by
`view_image` and a non-visual chat model. Configure a chat/Responses profile
with **Supports image input** and **Use for image analysis** for that role; it
may use the same provider credentials, but it remains a separate API
capability.

The **Validate** action checks image-model access through the provider's model
metadata endpoint. If a compatible gateway does not implement the single-model
route and returns `404` or `405`, Wisp checks its model-list endpoint instead.
It does not send the image-only model to Responses/Chat Completions and does not
generate a billable validation image.

Video generation is another separate model role. Create an OpenAI-compatible
profile with model ID `grok-imagine-video`, `grok-imagine-video-1.5`, or
`grok-imagine-video-1.5-preview`, then enable **Use for video generation**.
The edit form shows video defaults instead of chat-only fields: duration
(1–15 seconds, default 5), aspect ratio (`16:9`, `9:16`, `1:1`, `4:3`, `3:4`,
default `16:9`), and resolution (`480p`, `720p`, `1080p`, default `720p`).
A call without overrides uses those profile defaults.

Video generation is asynchronous. The `generate_video` tool submits the job to
the provider's `/v1/videos/generations` endpoint, receives a `request_id`,
then polls `/v1/videos/{request_id}` every 5 seconds (up to 10 minutes) until
the status is `done`, and downloads the temporary `video.url` immediately
before it expires. A `failed` or `expired` status surfaces as a tool error.
Transient `auth_unavailable` / `503` submission failures are retried up to
three times. The finished MP4 is saved under `media/` (for example
`media/clip.mp4`) and the tool result references that path so the final answer
can link to it. Generation usually takes 1–2 minutes. Video-only profiles do
not appear in chat, Reviewer, specialist, delegation, or side-chat model
pickers, and the **Validate** action probes them through the same model
metadata endpoint as image models — no billable video is generated.

## API protocols

| Protocol | Use when | Per-model fields |
| --- | --- | --- |
| OpenAI Chat Completions | DeepSeek, GLM, local gateways, or any `/chat/completions` compatible endpoint | Protocol, Model ID, optional endpoint suffix, optional Fast default |
| OpenAI Responses | Reasoning/tool-call models through `/v1/responses` | Protocol, Model ID, optional endpoint suffix, optional Fast default |
| Anthropic | Claude-compatible models through `/v1/messages` | Protocol, Model ID, optional endpoint suffix |

Enter the API root as the shared Base URL. Do not append `/v1`,
`/chat/completions`, `/responses`, or `/v1/messages`; Wisp adds the matching
request path for the selected protocol. If a service exposes one protocol or a
specific image model below a distinct path, put that path in the model's
optional **Endpoint suffix**. Wisp joins the suffix to the Base URL first, then
adds the selected protocol's request path. For OpenAI-compatible services, Wisp
tries both `/chat/completions` and `/v1/chat/completions` when the base URL has
no explicit version or endpoint path. It only falls back when the first route
is missing or returns an obvious non-API response, so authentication and
rate-limit failures are not duplicated.

For example, one DeepSeek API key can be represented by the shared Base URL
`https://api.deepseek.com`. Models using OpenAI Chat Completions or OpenAI
Responses leave the endpoint suffix blank. A model using DeepSeek's Anthropic
entry selects the Anthropic protocol and sets its endpoint suffix to
`/anthropic`, producing the effective Base URL
`https://api.deepseek.com/anthropic` before Wisp adds `/v1/messages`.
To put a second DeepSeek key on the same host, open **Add API access** again,
keep `https://api.deepseek.com`, and paste the other key instead of leaving
it blank.

OpenAI-compatible reasoning streams are normalized into one reasoning channel.
Empty `content` placeholders sent alongside Alibaba/DashScope
`reasoning_content` chunks are ignored, so a continuous thought process remains
one disclosure in the conversation. If a compatible relay resends the full
`content` or `reasoning_content` snapshot on every SSE chunk instead of a
fragment, Wisp keeps only the new suffix so the assembled reply and live UI
events stay linear.

If a provider ends a turn after returning only reasoning tokens—without visible
text or a tool call—Wisp reports a resumable error instead of showing the turn
as silently processed. Completed tool results remain in the conversation; use
**Resume** to request the missing final reply without replaying those tools. If
this repeats in a long conversation, send `/compact` before resuming to fold old
turns while preserving an archive of the full history.

Wisp also treats an SSE `error` payload and a Responses API status other than
`completed` as a failed, resumable turn, even when a compatible relay keeps the
HTTP status at 200 or appends a `[DONE]` marker. Partial output is not committed
as a final answer, completed tool results remain available, and follow-up
questions are not generated for that interrupted turn.

**Settings → Session → Automatically compact long conversations** is enabled by
default. Following mangopi-cli's model-boundary approach, Wisp checks the
estimated context before every native-agent model call, including later calls
after large tool results and ephemeral host/reviewer injections. At 80% it
archives the complete pre-compact history and targets the trigger minus an
adaptive headroom (twice the measured per-iteration growth, at least ~16K
tokens, at most 20% of the window), so slow conversations keep more context
while fast tool loops still land well clear of the next trigger. Older tool
output, reasoning, and images are safely pruned first without shortening user
messages or visible assistant answers — protection is counted in agent rounds
(user messages and tool-call batches), so a single instruction followed by
hundreds of tool calls still leaves old rounds prunable; oversized recent tool
payloads become bounded excerpts that point to the archive. If semantic turns
must be removed, Wisp summarizes a sanitized projection of the original
history, then retains one incrementally updated summary checkpoint plus at
most two recent turns in an 8K-token tail. The compacted working set is
appended as a new context epoch; earlier message rows stay frozen so rewind,
branch, and file-undo anchors still resolve. Raw images and large tool
results are not replayed to the summary model. The internal summary
instruction is never added to the conversation, and a failed compaction
leaves the previous epoch as head and stops before Wisp can send the
known-oversized main request; after such a failure, automatic retries are
suppressed until the estimate grows by another tenth of the window, so a
doomed compaction is not repaid at every model boundary. Tool
results are also capped to a 16 KiB head/tail excerpt when they enter model
context (the full result is still shown in the tool event), preventing one
read, grep, browser, or MCP response from consuming the whole window. Each
automatic or manual compaction leaves a persistent **Context automatically
compacted** / **Context compacted** flag in the conversation with the before
and after request-token estimates and the new epoch number. Turning the setting off keeps the warning,
manual `/compact`, and overflow recovery dialog available. ACP agents are not
modified because their remote transcripts are owned by the ACP process.

After a native-agent reply, the composer footer shows the estimated percentage
of the active model's context window. The limit tracks the model the session
is currently bound to: switching models or editing a profile's context window
re-bases the gauge immediately, without waiting for the next reply. Open it
for a detail card aligned to the
composer width that splits the same calibrated request estimate into system
prompt, built-in tool definitions, rules, selected Skills, MCP and other
dynamic tools, subagent definitions, and conversation content. These buckets
are mutually exclusive and sum to the value used by automatic compaction.
Select any bucket except Conversation to inspect the exact prompt/rule text or
the tool, Skill, MCP, and subagent definitions included in the latest native
request. Conversation remains a size-only category so the usage card does not
duplicate the chat transcript.
Older native usage rows that only stored a total attribute that window to
Conversation until the next reply refreshes the full breakdown. ACP sessions
expose only the total reported by the remote agent, so Wisp labels that value
as an agent-reported total instead of inventing a breakdown it cannot observe.
See [conversation history](conversation-history.md#model-view) for the model-view
toggle and the in-context marks on compacted turns.

## Usage dashboard

**Settings → Usage** shows global input, output, reasoning, and cached-token
totals, a 53-week activity chart with **Daily**, **Weekly**, and **Cumulative**
views, an input-plus-output token share by model, and a ranked list of SKILL
(`use_skill`) and MCP (`mcp:*`) tool calls beneath the model chart. Usage is
grouped by project workspace. Open a workspace to inspect its sessions, which
are loaded 20 at a time with Previous/Next pagination; sub-agent rounds remain
folded into their root session.

New usage rounds persist the model and timestamp used for that request. Older
usage events did not contain those fields, so their dashboard model falls back
to the session's saved model binding and their activity date falls back to the
session's latest activity date.

When the provider explicitly rejects a built-in Wisp-agent request for
exceeding its context window, the conversation opens a recovery dialog instead
of leaving the raw error as a dead end. **Compact and continue** archives the
full history, folds older turns, and resumes after the retained tool results.
**Continue in a new conversation** starts a clean session and attaches a
bounded Reader summary of the old conversation as context. **Pause
conversation** preserves the error and completed work without making another
request. Pressing Escape immediately after the dialog opens is equivalent to
pausing; it closes only this recovery surface.

For OpenAI-compatible and Responses API profiles, Wisp sends its internal
`python` REPL tool as `wisp_python` and maps returned calls back to `python`.
This avoids the reserved `python` function-name collision on Codex models,
including when the request is translated by gateways such as CLIProxyAPI.

API keys are stored in the OS keyring. They are not stored in SQLite.

The desktop app stores model profile metadata in `.wisp/wisp.sqlite`. Existing single-model installs are migrated into a `default` model profile the first time settings are loaded.

## Headless CLI

The `wisp-science` headless CLI uses environment variables and supports the
same API protocols. `wisp-science login codex` signs in with a ChatGPT
Plus/Pro subscription (`--method device` for a one-time code). After that,
`WISP_PROVIDER=openai_codex` uses the stored subscription and does not need
`WISP_API_KEY`. `wisp-science login xai` does the same for a SuperGrok or
X Premium+ subscription with an xAI device code; then use
`WISP_PROVIDER=xai_oauth`.

```powershell
$env:WISP_PROVIDER = "openai"           # openai, openai_responses, openai_codex, xai_oauth, or anthropic
$env:WISP_API_URL  = "https://api.deepseek.com"
$env:WISP_MODEL    = "deepseek-v4-flash"
$env:WISP_API_KEY  = "<your provider key>"
# Optional dedicated vision model when the primary chat model cannot see images:
$env:WISP_VISION_PROVIDER = "openai"
$env:WISP_VISION_API_URL  = "https://api.openai.com/v1"
$env:WISP_VISION_MODEL    = "gpt-4o-mini"
cargo run -p wisp-cli
```

The full CLI environment-variable table, eval/RPC commands, and bundled MCP
launch flags are in [development](development.md). Desktop setup, including
ACP agents and remote MCP connections, is in
[basic configuration](basic-configuration.md).
