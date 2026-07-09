"""Fail-closed serving gate for the standalone per-domain servers.

The aggregate (mcp_bio) applies deferred.json itself; when a domain server
runs STANDALONE (run_server.py mcp_<domain>) it must enforce the exact same
gate, or splitting the aggregate into per-domain connectors would silently
re-enable legally-deferred tools (KEGG/CADD/PanglaoDB license gate, plus any
domain/tool deferrals). Single source of truth stays mcp_bio/domains.json +
mcp_bio/deferred.json — read here as data files (no module import, so a
single domain server never pays for importing the whole 26-server fleet).

Contract (same as mcp_bio.load_deferred): unknown names in the gate fail
CLOSED with a RuntimeError; a server whose every tool is gated refuses to
start rather than serving an empty tool list.
"""

from __future__ import annotations

import importlib.resources
import json
import threading

import anyio


def _load(resource: str) -> dict:
    # encoding=utf-8: Windows ANSI code pages (e.g. GBK) cannot decode these files.
    with importlib.resources.files("mcp_bio").joinpath(resource).open(
        "r", encoding="utf-8"
    ) as f:
        return json.load(f)


def gated_tool_names() -> frozenset[str]:
    """Union of all tool names deferred.json removes from serving."""
    domains = _load("domains.json")
    gate = _load("deferred.json")
    all_tools = {t for roster in domains.values() for t in roster}
    bad_domains = set(gate.get("domains", [])) - set(domains)
    bad_tools = (set(gate.get("tools", []))
                 | set(gate.get("license_tools", []))) - all_tools
    if bad_domains or bad_tools:
        raise RuntimeError(
            "deferred.json names unknown to domains.json — failing closed: "
            f"domains={sorted(bad_domains)} tools={sorted(bad_tools)}")
    names = set(gate.get("tools", [])) | set(gate.get("license_tools", []))
    for d in gate.get("domains", []):
        names |= set(domains[d])
    return frozenset(names)


def apply_gate_fastmcp(fm) -> None:
    """Remove gated tools from a tier-2 FastMCP server before serving.

    Call from the server's main() (NOT at import time — the aggregate
    imports these modules and applies its own gate). Raises if the gate
    empties the server: a fully-deferred domain must refuse to start.

    Also ports the aggregate's CRITICAL dispatch safeguard (finding
    3406443687): FastMCP calls SYNC tool functions inline on the event
    loop, and every tier-2 tool does blocking HTTP — one slow upstream
    would wedge the whole standalone server (listTools health probes,
    concurrent calls, cancellations) exactly like the pride_search
    stress-test incident the aggregate engineered around. call_tool is
    rebound to dispatch into a worker thread (own event loop — sync and
    async tools both work) behind ONE process-wide anyio.Lock: a
    standalone process serves a single domain, so the per-domain
    serialization the aggregate does per-package collapses to one lock
    (shared @lru_cache client wrapping a non-thread-safe
    requests.Session).
    """
    gated = gated_tool_names()
    served = list(fm._tool_manager._tools)
    for name in served:
        if name in gated:
            fm.remove_tool(name)
    if not fm._tool_manager._tools:
        raise RuntimeError(
            f"{fm.name}: every tool is deferred by mcp_bio/deferred.json — "
            "this domain is not cleared to serve standalone")
    if getattr(fm, "_operon_offloop_dispatch", False):
        return  # idempotent — don't double-wrap
    orig_call_tool = fm.call_tool
    lock = anyio.Lock()

    async def _call_tool_offloop(name, arguments):
        def _run_in_worker() -> object:
            return anyio.run(orig_call_tool, name, arguments)

        async with lock:
            return await anyio.to_thread.run_sync(_run_in_worker)

    # Rebinding the instance attribute alone is DEAD on the real serving
    # path (finding 3406680897): FastMCP registers the BOUND self.call_tool
    # into the low-level Server's request-handler table at construction, and
    # that captured reference is never re-resolved. Re-register the handler
    # so mcp.run() actually dispatches through the wrapper — same decorator
    # call _setup_handlers used, which overwrites the handler-table entry.
    fm._mcp_server.call_tool(validate_input=False)(_call_tool_offloop)
    # Keep the attribute rebound too, for in-process callers and tests.
    fm.call_tool = _call_tool_offloop
    fm._operon_offloop_dispatch = True


def apply_gate_tier1(t1) -> None:
    """Remove gated tools from a Tier1Server before serving.

    Serve-time only (call from the server's main()): build_server() must
    stay pristine — the drop-in parity tests and the aggregate consume the
    full schema set. Raises if the gate empties the server.

    Also serializes same-process dispatch (finding 3406443687):
    Tier1Server._call_tool offloads via anyio.to_thread with NO lock, while
    the domain's handlers share one @lru_cache client wrapping a
    requests.Session (not thread-safe, non-atomic stats writes — reviews
    3386234819/3386420557). Each handler is wrapped with a process-wide
    threading.Lock, held inside the worker thread: same one-in-flight
    serialization the aggregate enforces per domain, while the event loop
    stays free.
    """
    gated = gated_tool_names()
    t1.schemas = [s for s in t1.schemas if s["name"] not in gated]
    t1.handlers = {k: v for k, v in t1.handlers.items() if k not in gated}
    if not t1.schemas:
        raise RuntimeError(
            f"{t1.name}: every tool is deferred by mcp_bio/deferred.json — "
            "this domain is not cleared to serve standalone")
    if getattr(t1, "_operon_serialized_dispatch", False):
        return  # idempotent — don't double-wrap
    lock = threading.Lock()

    def _serialized(handler):
        def run(args):
            with lock:
                return handler(args)
        return run

    t1.handlers = {k: _serialized(v) for k, v in t1.handlers.items()}
    t1._operon_serialized_dispatch = True
