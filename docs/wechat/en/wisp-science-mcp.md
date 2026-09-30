# Wisp Science Basics: MCP

Research often means switching between tools: PubMed for papers, GEO for datasets, and project notes for organizing results. To help with this work, an AI needs access to the relevant data sources and tools.

MCP provides a common way to connect them. In Wisp Science, you can use built-in research connectors or add your own MCP services so that searching, reading, and organizing results happen in the same conversation.

This tutorial introduces the concept, then uses two examples to configure and verify connections.

**MCP connects AI applications to external tools.**

MCP stands for **Model Context Protocol**, an open standard connecting AI applications to external systems such as data sources, tools, and workflows. Its official introduction compares it to USB-C: a common interface lets different systems connect. See the [MCP introduction](https://modelcontextprotocol.io/docs/getting-started/intro).

In Wisp, the responsibilities are:

| Participant | Responsibility |
| --- | --- |
| Language model | Understand your request, select tools, and interpret results |
| Wisp Science | Manage connections, organize calls, handle permissions, and display results |
| MCP service | Provide capabilities such as webpage reading, note search, or database queries |

For a literature request, Wisp can pass the model's chosen query to the appropriate tool, then let the model organize the returned records.

MCP server capabilities include **Tools**, **Resources**, and **Prompts**. Support depends on each server and client. This tutorial focuses on connecting services and calling tools. See [MCP server capabilities](https://modelcontextprotocol.io/specification/2025-06-18/server/index).

For research, request actual PMIDs, dataset identifiers, and source URLs so you can verify them later. Tool output still needs to be evaluated against original records and papers.

**Start with a built-in research connector.**

Wisp includes connectors for sources such as PubMed, GEO, and UniProt. Open a project and go to **Settings → Connections** to inspect and enable them.

Click a connector to see its description, sources, and tool list. Expand a tool for parameters, required fields, and defaults. The [research connector catalog](https://xuzhougeng.github.io/wisp-science/mcp.html) describes coverage.

After configuring a working model, try:

> Use the PubMed tools to find five papers from 2024–2025 on plant single-cell transcriptomics. List title, year, journal, PMID, and source URL, with one sentence about the study organism or material. Include only papers actually returned by the tools, and explain any search failure.

You do not need to memorize tool names. The built-in agent discovers relevant tools and calls a matching capability. Specify the database, research topic, and desired output.

Built-in connectors ship with the app, but accessing upstream databases still requires network access. Some sources also have credential, quota, or access requirements.

**For a custom MCP service, identify whether you have a command or a URL.**

Open **Settings → Connections → Add connection**.

| Supplied configuration | Connection type | Prerequisites |
| --- | --- | --- |
| A command such as `uvx …` or `npx …` | Local command | The launcher, arguments, and any required environment variables |
| A service endpoint such as `https://…/mcp` | Remote URL | The MCP endpoint and any required authorization or headers |

A local command starts an MCP process on your computer and communicates over standard input and output. The process may still access online services.

A remote URL connects to an already-running MCP service. Enter the provider's **MCP endpoint**; a website homepage or ordinary API URL usually will not work.

Choose whichever example below is useful; you do not need both.

**Local command example: add Fetch for reading webpages.**

Fetch is a service in the official MCP examples repository. It reads webpages and converts them into text suitable for a model. Its documented launch command is `uvx mcp-server-fetch`. See the [Fetch instructions](https://github.com/modelcontextprotocol/servers/blob/main/src/fetch/README.md).

Install uv following its [official installation guide](https://docs.astral.sh/uv/getting-started/installation/), then run `uvx --version` in a terminal to check that the command is available.

Fill in Add connection:

| Field | Value |
| --- | --- |
| Name | `Fetch webpage reader` |
| Type | Local command |
| Command | `uvx` |
| Arguments | `mcp-server-fetch` |
| Environment variables | Usually empty for this example |

**Keep the command and arguments separate.** Enter only `uvx` in Command and `mcp-server-fetch` in Arguments.

Click **Test**, then **Save** after success, and verify that the connection is enabled. The first run may download dependencies and take longer.

If a terminal finds `uvx` but Wisp cannot, enter its full executable path. Find it with `command -v uvx` on macOS/Linux or `(Get-Command uvx).Source` in Windows PowerShell.

For Windows encoding errors, the Fetch documentation suggests the environment variable `PYTHONIOENCODING` with value `utf-8`.

Save the connection and start a new conversation:

> Use the Fetch webpage reader connection to read https://example.com. Report the page title, main content, and URL actually read. If the call fails, report the error.

Check both successful tool execution and whether the answer follows the returned content. Fetch is suited to webpage text; login-dependent or highly interactive tasks may need other tools.

**Remote URL example: connect Notion.**

If you keep research notes in Notion, you can connect its remote MCP service at `https://mcp.notion.com/mcp`, using OAuth for workspace authorization. See [Notion's connection instructions](https://developers.notion.com/guides/mcp/get-started-with-mcp).

| Field | Value |
| --- | --- |
| Name | `Notion research notes` |
| Type | Remote URL |
| URL | `https://mcp.notion.com/mcp` |
| Authentication | OAuth |

Click **Test** or **Save** to follow the browser authorization flow. Sign in, choose the workspace, and authorize access. Testing does not save the connection; save it afterward.

In a new conversation, try:

> Use Notion research notes to find pages with “Literature reading” in the title. Return only page titles and links.

Search and reading are useful initial checks of authorization scope. When you later want to create or update notes, specify the target page and the exact content.

Other remote services may use API keys instead of OAuth. Fill request headers according to their documentation. For a required Bearer token, use `Authorization` with value `Bearer YOUR_ACTUAL_KEY`. Such manual-header configurations normally select authentication **None**, meaning no OAuth flow; the header still authenticates the request.

Wisp stores connection-level environment values, header values, and OAuth tokens in the OS keyring. When editing, leaving an existing secret blank preserves it; removing its row clears it. See [Basic Configuration](https://github.com/xuzhougeng/wisp-science/blob/main/docs/basic-configuration.md).

**Specify the source, task, and deliverable.**

There is no per-turn MCP selector. Available connections depend on the current project's connection settings and enabled plugins. Your prompt can specify which source this task should use.

For public data:

> Use GEO tools to find public single-cell RNA-seq datasets for Arabidopsis root tips. List candidate GSE identifiers, topics, platforms, and source URLs, then explain their relevance to my question. Only organize metadata for now.

For a saved literature list:

> Use PubMed tools to find ten candidate papers on plant root cell atlases. Save titles, years, PMIDs, links, and screening reasons to a Markdown file in the project, then report its location.

The connector retrieves information; other Wisp tools can organize and save it. One research task can combine several tools without a single MCP service doing every step.

After changing connections, validate in a new session to avoid an older agent's connection state. Disable a service under **Settings → Connections**, then create a new session or wait for an idle agent rebuild. Tool details also offer Allow, Ask, and Deny permission rules.

**Troubleshoot at the point of failure.**

| Symptom | Check first |
| --- | --- |
| Local command not found | Installed uv/Node dependencies; use an absolute executable path if needed |
| First test keeps waiting | Dependency downloads and access to package sources |
| Remote 401 / 403 | Completed authorization, valid token, and resource permissions |
| Remote 404 | MCP endpoint rather than service homepage or ordinary API URL |
| Test succeeds but no tool is called | Saved and enabled connection; new session with its name and a specific task |
| Tools are listed but a query fails | Actual call error, parameters, permissions, quota, or upstream status |

For proxies, check **Settings → Network** for MCP settings. Existing connections need to reconnect; whether a local MCP process honors proxy variables also depends on that service.

Start with a familiar small task: five papers, a public webpage, or a note search. Inspect the called tool and returned results, then continue from those results to build a useful research workflow.

> This tutorial reflects Wisp documentation and implementation when written. Labels may vary by version. Example prompts demonstrate usage, not searches already executed.
