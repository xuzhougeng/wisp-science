"""mcp-bio: the single bundled bio-retrieval MCP server.

Aggregates all 23 domain servers (5 tier-1 drop-ins + 18 tier-2 domain
servers) into ONE stdio MCP process serving the union of their 247 tools
(minus any domains/tools gated by deferred.json — the single deferral knob).
Tool names are globally unique across domains (asserted at import).

The 23-domain partitioning lives on as:
  - ``domains.json`` — domain slug -> sorted tool names. Single source of
    truth for the skill-doc clustering (operon's bundledRegistry mirrors it;
    a tripwire test keeps the copies in sync).
  - the per-domain packages themselves (``mcp_pubmed``, ``mcp_variants``, …),
    which remain independently runnable for development.

Dispatch preserves each tier's wire behavior exactly:
  - tier-1: verbatim embedded schemas + sync handlers via anyio.to_thread —
    identical to Tier1Server (same low-level Server machinery, same
    validation and error shapes).
  - tier-2: pass-through to each domain's FastMCP instance
    (``fm.call_tool``), so content/structured-content conversion matches the
    standalone server.
"""

from __future__ import annotations

import importlib
import json
from importlib import resources

import anyio
import anyio.to_thread
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

from mcp_servers_common.tier1 import READ_ONLY

SERVER_NAME = "bio-mcp-server"

TIER1_PACKAGES = [
    "mcp_pubmed",
    "mcp_clinical_trials",
    "mcp_chembl",
    "mcp_biorxiv",
    "mcp_biomart",
]

TIER2_PACKAGES = [
    "mcp_cellguide",
    "mcp_variants",
    "mcp_clinical_genomics",
    "mcp_expression",
    "mcp_regulation",
    "mcp_protein_annotation",
    "mcp_rna",
    "mcp_structures_interactions",
    "mcp_omics_archives",
    "mcp_cancer_models",
    "mcp_genes_ontologies",
    "mcp_drug_regulatory",
    "mcp_research_resources",
    "mcp_chemistry",
    "mcp_human_genetics",
    "mcp_literature",
    "mcp_genomes",
    "mcp_zinc",
]


def load_domains() -> dict[str, list[str]]:
    """domain slug -> sorted tool names (the 23-domain partition)."""
    # encoding=utf-8: Windows defaults to the ANSI code page (e.g. GBK), which
    # cannot decode the UTF-8 JSON shipped in this package.
    with resources.files(__package__).joinpath("domains.json").open(
        "r", encoding="utf-8"
    ) as f:
        return json.load(f)


def load_deferred() -> dict:
    """Deferral gate, two independent criteria:

    - ``domains``/``tools``: NET-NEW upstream resources vs operon main,
      deferred pending separate legal review. The stacked PR empties these
      lists to enable them — tests and the startup self-check all key off
      this file, so that flip is the single knob.
    - ``license_tools``: upstream LICENSE forbids/restricts commercial use
      (KEGG academic-only, CADD non-commercial, PanglaoDB). Deliberately not
      on the stacked PR's flip — lifted per-upstream when legal clears the
      specific license.

    Fails CLOSED on typos: every named domain/tool must exist in
    domains.json — an unknown slug previously fell out of the gate silently
    with every tripwire still green (#2875 review)."""
    with resources.files(__package__).joinpath("deferred.json").open(
        "r", encoding="utf-8"
    ) as f:
        deferred = json.load(f)
    domains = load_domains()
    all_tools = {n for tools in domains.values() for n in tools}
    bad_domains = set(deferred.get("domains", [])) - set(domains)
    bad_tools = (set(deferred.get("tools", []))
                 | set(deferred.get("license_tools", []))) - all_tools
    if bad_domains or bad_tools:
        raise ValueError(
            "deferred.json names entries unknown to domains.json (a typo "
            f"here would fail OPEN): domains={sorted(bad_domains)} "
            f"tools={sorted(bad_tools)}")
    return deferred


def deferred_tool_names() -> set[str]:
    """All tool names excluded by the deferral gate (whole deferred domains
    plus individually deferred tools, on either criterion)."""
    deferred = load_deferred()
    domains = load_domains()
    names: set[str] = set(deferred.get("tools", []))
    names.update(deferred.get("license_tools", []))
    for d in deferred.get("domains", []):
        names.update(domains.get(d, []))
    return names


def _pkg_for_domain(domain: str) -> str:
    return "mcp_" + domain.replace("-", "_")


class BioAggregate:
    """Union of the tier-1 handler maps and tier-2 FastMCP instances."""

    def __init__(self) -> None:
        deferred = load_deferred()
        skip_pkgs = {_pkg_for_domain(d) for d in deferred.get("domains", [])}
        skip_tools = deferred_tool_names()

        # tier-1: verbatim schemas + sync handlers
        self.t1_schemas: list[dict] = []
        self.t1_handlers: dict[str, object] = {}
        # Per-domain serialization (reviews 3386234819, 3386420557): worker
        # -thread dispatch runs same-domain calls concurrently, but each
        # domain funnels into ONE process-wide client wrapping a
        # requests.Session (not thread-safe) with non-atomic stats writes.
        # One anyio.Lock per source package, acquired ON THE EVENT LOOP
        # before entering the thread pool — a parked same-domain call waits
        # as a coroutine instead of pinning one of anyio's ~40 shared worker
        # tokens, so a same-domain pile-up can never starve cross-domain
        # calls (the reason the worker dispatch exists).
        self.domain_locks: dict[str, anyio.Lock] = {}
        self.t1_locks: dict[str, anyio.Lock] = {}
        for pkg in TIER1_PACKAGES:
            if pkg in skip_pkgs:
                continue
            pkg_lock = self.domain_locks.setdefault(pkg, anyio.Lock())
            t1 = importlib.import_module(f"{pkg}.server").build_server()
            for name, handler in t1.handlers.items():
                if name in skip_tools:
                    continue
                if name in self.t1_handlers:
                    raise ValueError(f"duplicate tier-1 tool: {name} ({pkg})")
                self.t1_handlers[name] = handler
                self.t1_locks[name] = pkg_lock
            self.t1_schemas.extend(
                s for s in t1.schemas if s["name"] not in skip_tools
            )

        # tier-2: FastMCP instances; dispatch map tool -> fm
        self.t2_fm: dict[str, object] = {}
        self.t2_locks: dict[str, anyio.Lock] = {}
        for pkg in TIER2_PACKAGES:
            if pkg in skip_pkgs:
                continue
            pkg_lock = self.domain_locks.setdefault(pkg, anyio.Lock())
            fm = importlib.import_module(f"{pkg}.server").mcp
            for t in fm._tool_manager.list_tools():
                if t.name in skip_tools:
                    continue
                if t.name in self.t2_fm or t.name in self.t1_handlers:
                    raise ValueError(f"duplicate tool: {t.name} ({pkg})")
                self.t2_fm[t.name] = fm
                self.t2_locks[t.name] = pkg_lock

        served = set(self.t1_handlers) | set(self.t2_fm)
        mapped = {n for tools in load_domains().values() for n in tools}
        expected = mapped - skip_tools
        if served != expected:
            raise ValueError(
                f"domains.json/deferred.json out of sync with served tools: "
                f"only-served={sorted(served - expected)} "
                f"only-expected={sorted(expected - served)}"
            )

    def tool_names(self) -> set[str]:
        return set(self.t1_handlers) | set(self.t2_fm)


def build_server() -> tuple[Server, BioAggregate]:
    agg = BioAggregate()
    server = Server(SERVER_NAME)

    @server.list_tools()
    async def _list_tools() -> list[Tool]:
        tools = [
            Tool(name=s["name"], description=s["description"],
                 inputSchema=s["input_schema"], annotations=READ_ONLY)
            for s in agg.t1_schemas
        ]
        seen_fm = []
        for fm in agg.t2_fm.values():
            if any(fm is f for f in seen_fm):
                continue
            seen_fm.append(fm)
            # The dispatch map IS the served surface: fm.list_tools() returns
            # the instance's FULL registered set, unfiltered by deferred.json
            # — an individually-deferred tier-2 tool would be advertised yet
            # raise "Unknown tool" on call, re-exposing the gated upstream
            # (review 3383284164). List only what _call_tool dispatches.
            tools.extend(
                t for t in await fm.list_tools() if t.name in agg.t2_fm
            )
        return tools

    @server.call_tool()
    async def _call_tool(tool: str, arguments: dict | None):
        args = dict(arguments or {})
        handler = agg.t1_handlers.get(tool)
        if handler is not None:
            # Same-domain serialization on the EVENT LOOP (reviews
            # 3386234819, 3386420557): waiting here is a parked coroutine,
            # not a parked worker thread holding a shared limiter token.
            async with agg.t1_locks[tool]:
                text = await anyio.to_thread.run_sync(lambda: handler(args))
            return [TextContent(type="text", text=text)]
        fm = agg.t2_fm.get(tool)
        if fm is None:
            raise ValueError(f"Unknown tool: {tool}")
        # CRITICAL: never run tier-2 tools on the server's event loop. The
        # mcp SDK's FastMCP calls SYNC tool functions inline (no to_thread —
        # func_metadata.call_fn_with_arg_validation), and every tier-2 tool
        # does blocking HTTP, so one slow upstream would freeze the entire
        # server — including local-only tools (found by stress test: a broad
        # pride_search_projects wedged everything for 10+ minutes). Dispatch
        # the whole fm.call_tool coroutine into a worker thread with its own
        # event loop; sync AND async (cellguide) tools both work there, and
        # the main loop stays free to serve concurrent requests.
        def _run_in_worker() -> object:
            return anyio.run(fm.call_tool, tool, args)

        # Pass-through: FastMCP returns ContentBlocks or a structured dict;
        # the low-level Server accepts both (content / structuredContent).
        # Same-domain serialization on the EVENT LOOP — see t1 above
        # (reviews 3386234819, 3386420557).
        async with agg.t2_locks[tool]:
            return await anyio.to_thread.run_sync(_run_in_worker)

    return server, agg


def main() -> None:
    server, _ = build_server()

    async def _main() -> None:
        async with stdio_server() as (read, write):
            await server.run(read, write, server.create_initialization_options())

    anyio.run(_main)


if __name__ == "__main__":
    main()
