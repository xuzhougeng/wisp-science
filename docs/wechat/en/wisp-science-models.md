# Wisp Science Basics: Model Configuration

When you first open Wisp Science, you may wonder why it asks for an API address and key when you already have an AI account. Which model name should you enter? After saving it, how do you know the conversation uses that model?

Wisp Science manages your research workspace separately from model access. Keep the same project files and analysis records while selecting among models you can access. This tutorial starts with a working first connection, then introduces image capabilities and switching models.

> Screenshots come from the real frontend with demonstration settings and conversations, using the English interface. The model list is not a ranking or proof of access for an account. Follow the configuration supplied by your provider.

**Understand the app, model service, and API key.**

| Component | Responsibility | What you need |
| --- | --- | --- |
| Wisp Science | Projects, conversations, tool calls, and work records | Install the app and open a project |
| Model service | Receive requests and generate replies or tool calls | A reachable address, supported protocol, and model ID |
| API key | Identify the account or quota used for a request | A key from the provider's console |

Website logins, chat subscriptions, and API access may be provisioned separately. Being able to chat on a website does not necessarily mean you have an API key for Wisp. Confirm that you have API access details.

A laboratory gateway also works when you use the address, model ID, and key supplied by its administrator. Do not put a web-chat URL in the API address field.

**Open Models and check existing configurations.**

Go to **Settings → Models**. Review existing models or click **Add API access**. Model setup in onboarding is another entry point; skipping it does not prevent you from configuring a model here later.

![Models settings showing configured models and Add API access](../../assets/tutorials/en/models/01-overview.png)

*Figure 1: Check existing models first. A display name helps identify a model's purpose; requests still depend on its model ID, protocol, and address.*

The same page has an **ACP Agents** category for external agent processes. This tutorial covers HTTP API models used by the built-in Wisp agent. If your setup instructions provide a process launch command, see [ACP configuration](wisp-science-acp.md).

**Enter shared API access, then add its models.**

Click **Add API access** and enter the provider's **Base URL** and **API key**. Add the models this key can call below. One API access form can contain multiple models, each with its own protocol, model ID, and capabilities.

![API access form with shared URL and key above individual model settings](../../assets/tutorials/en/models/02-api-access.png)

*Figure 2: Models added together share the address and key. The screenshot illustrates the fields without exposing a key. Check suggested models against your account's permissions too.*

Get these fields right first:

| Field | What to enter | Common confusion |
| --- | --- | --- |
| Base URL | The API base address supplied by the provider | Usually do not append a complete route such as `/v1/chat/completions` yourself |
| API key | A key generated in the provider console | It is not your website login password |
| Protocol | OpenAI-compatible, OpenAI Responses, or Anthropic, as documented by the provider | OpenAI-compatible describes an interface that other providers may implement |
| Model ID | The exact name accepted by the service | A display name is customizable; the model ID is not arbitrary |
| Display name | A recognizable label, such as Laboratory primary model | This does not change the model called |
| Endpoint suffix | An extra path explicitly required by the provider | Usually leave it empty rather than repeatedly appending complete routes |

If uncertain, check the protocol and base address together against the provider's documentation. Do not cycle through appending `/v1`, `/responses`, and `/messages` whenever an error occurs.

A new address needs a valid key. If a key is already stored for that Base URL, leaving the field empty can reuse it. Pasting another key creates separate access for the new model batch. Use distinct display names when the same model is accessed through different accounts.

Wisp stores API keys in the operating system keyring. Share the address, protocol, and model ID in screenshots or configuration notes, but leave out the actual key.

Click **Validate**, inspect the result, and then click **Save** after validation succeeds. Validation does not replace saving. You can validate again from a saved model's edit page. Passing this check does not replace testing specific tool-calling or image tasks.

**Use documented output and context limits.**

After saving access, click an individual model in the list to edit output length, context window, reasoning settings, and related options.

**Max output tokens** limits the content generated in one response. **Context window** describes the overall input-and-output capacity. Increasing these numbers does not make the model more capable.

For exact IDs recognized by the built-in model catalog, Wisp supplies the recorded ceilings. For an unknown alias or gateway model, enter limits documented by the provider. Saving output above a known ceiling produces an error; an oversized context setting is clamped to the known limit.

Reasoning effort and Fast mode also depend on provider and model support. Leave defaults in place for the first test, then adjust them for actual tasks. Fast mode may affect quota usage; check your service's description before enabling it.

**Choose the model for this conversation.**

Open a new conversation and select the saved model in the picker near the message box.

![The conversation model picker](../../assets/tutorials/en/models/03-picker.png)

*Figure 3: This picker selects the current conversation's model. Switching a conversation that already has messages requests confirmation. The model default in Settings is used for new conversations.*

Start with a small question that does not require the web or research tools:

> Explain CSV files in three sentences, then give a CSV example with two data rows. Do not read files or visit websites for this task.

This helps distinguish a model connection problem from an external-tool setup problem. After a complete response, try a small file from your project.

Asking “What model are you?” is not sufficient verification: a model's natural-language self-description does not replace configuration records. Check the picker and, where available, the turn's [trajectory and usage records](wisp-science-trajectory.md).

Switching a populated conversation changes subsequent requests for that conversation. A request already running continues with the configuration used when the turn started; the new configuration takes effect on later turns.

**Configure image input and image analysis when you need them.**

Research often involves microscopy images, screenshots, and plots. Text responses do not establish that a model accepts images.

- Enable **Supports image input** for a chat model with vision support.
- Assign **Use for image analysis** if it should also describe images for a non-visual primary model.
- **Use for image generation** is a separate role that produces images; it does not replace image analysis.

Upload a simple, clearly labeled image without sensitive information and try:

> Describe only the visible axes, legend, and trends. Identify any text you cannot read. Do not invent experimental conditions or infer causality.

Verify visible information before moving to scientific interpretation. If the primary model cannot read images and no image-analysis model is available, configure the missing capability before retrying.

**Troubleshoot where the failure occurs.**

| Symptom | Check first |
| --- | --- |
| 401 / 403 | Key validity, enabled API access, and permission for this model |
| 404 or webpage content | Base URL, protocol, model ID, and whether a website homepage was entered by mistake |
| Timeout or connection failure | Connectivity and the model API proxy in Settings → Network |
| Validation succeeds but a task fails | Support for the task's tool calls, image input, or response length |
| Context limit exceeded | Try `/compact`, or start a new session explaining what should be continued |
| Reply is cut off | Check maximum output, use Resume if appropriate, or reduce the scope of this request |

A first configuration can have a small goal: save one connection, select it in a new conversation, and obtain a complete response. Then explore [MCP](wisp-science-mcp.md), [Skills](wisp-science-skills.md), and [Browser](wisp-science-browser.md), with a clearer idea of where a later problem occurs.

> See [Model Configuration](../../model-configuration.md) for details. This tutorial reflects the implementation when written; labels may vary between versions. Example prompts do not represent model requests that have already been executed.
