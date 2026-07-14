#!/usr/bin/env python3
"""Wisp kernel worker — persistent Python execution over a JSON-per-line
stdin/stdout protocol.

Ready:    {"type": "ready", "protocol": 1, "language": "python", ...}
Request:  {"type": "execute", "id": "<uuid>", "code": "<python source>"}
Inspect:  {"type": "inspect", "id": "<uuid>"}
Streamed: {"type": "stdout_chunk", "id": "<uuid>", "data": "<text>"}
Response: {"type": "result", "id": "<uuid>", "stdout": "...", "stderr": "...",
           "error": null|"<traceback>", "interrupted": false,
           "trace": {"error_lineno": null, "error_call": null},
           "usage": {"wall_s": 0.0, "cpu_s": 0.0, "rss_kb": 0}}

This is a Windows-friendly port of the upstream wisp-science
`kernels/kernel_worker.py`: the POSIX-only `resource`, `/proc`, and
delivered-SIGINT discipline are dropped. RSS comes from `psutil` when
installed (else 0). Per-cell interrupt is not supported in this MVP —
long-running cells block until they return.
"""

import builtins
import io
import json
import os
import sys
import time
import traceback
import types

MAX_OUTPUT_SIZE = 1024 * 1024  # 1 MB head cap on stdout/stderr
MAX_OBJECTS = 200
MAX_NAME_SIZE = 256
MAX_META_SIZE = 160

# Force a non-interactive matplotlib backend before matplotlib is ever imported.
# Without this, generated plotting code (plt.show(), scanpy sc.pl.*) selects the
# platform GUI backend (MacOSX/Tk/Qt) and plt.show() opens a window that BLOCKS
# the kernel until the user closes it, stalling the whole analysis (issue #37).
# Figures are meant to be surfaced via savefig, never a GUI window.
os.environ["MPLBACKEND"] = "Agg"


def _neutralize_pyplot_show() -> None:
    """Belt-and-suspenders: make plt.show() a no-op so code that explicitly
    forces a GUI backend (matplotlib.use("MacOSX")) still can't block the kernel."""
    plt = sys.modules.get("matplotlib.pyplot")
    show = getattr(plt, "show", None) if plt is not None else None
    if show is None or getattr(show, "_wisp_noop", False):
        return

    def _noop_show(*_a, **_k):  # ponytail: figures go to savefig, not a GUI
        return None

    _noop_show._wisp_noop = True
    plt.show = _noop_show


def _try_psutil_rss_kb() -> int:
    try:
        import psutil  # type: ignore

        return int(psutil.Process().memory_info().rss // 1024)
    except Exception:
        return 0


class _CappedStream(io.StringIO):
    """StringIO with a hard byte cap; reports dropped bytes on read-out."""

    CAP = MAX_OUTPUT_SIZE - 256

    def __init__(self):
        super().__init__()
        self._buffered = 0
        self._dropped = 0

    def write(self, s):
        n = len(s.encode("utf-8", "surrogatepass"))
        if self._buffered >= self.CAP:
            self._dropped += n
            return len(s)
        remaining = self.CAP - self._buffered
        if n <= remaining:
            self._buffered += n
            return super().write(s)
        head = s.encode("utf-8", "surrogatepass")[:remaining].decode("utf-8", "ignore")
        self._buffered = self.CAP
        self._dropped = n - remaining
        super().write(head)
        return len(s)

    def getvalue(self):
        v = super().getvalue()
        if self._dropped:
            return v + f"\n...(buffer capped at {self.CAP // 1024} KB; {self._dropped} further bytes dropped)\n"
        return v


class _StreamingStdout(_CappedStream):
    """Write-through stdout: captures to a buffer AND streams each write as a
    `stdout_chunk` JSON line on the protocol-out pipe."""

    STREAM_CAP = 10 * 1024 * 1024

    def __init__(self, protocol_out, lock, request_id):
        super().__init__()
        self._streamed = 0
        self._protocol_out = protocol_out
        self._lock = lock
        self._request_id = request_id
        self._active = True

    def write(self, s):
        if s and self._active and self._streamed < self.STREAM_CAP:
            try:
                n = len(s.encode("utf-8", "surrogatepass"))
                remaining = self.STREAM_CAP - self._streamed
                payload = s if n <= remaining else s.encode("utf-8", "surrogatepass")[:remaining].decode("utf-8", "ignore")
                self._streamed += min(n, remaining)
                line = json.dumps({"type": "stdout_chunk", "id": self._request_id, "data": payload}) + "\n"
                with self._lock:
                    self._protocol_out.write(line)
                    self._protocol_out.flush()
            except Exception:
                pass
        return super().write(s)


def _truncate(text, max_size=MAX_OUTPUT_SIZE):
    if len(text) > max_size:
        return text[:max_size] + f"\n... (truncated, {len(text) - max_size} bytes omitted)"
    return text


def _object_summary(value):
    value_type = type(value)
    if value is None or value_type in (bool, int, float, complex):
        return repr(value)
    if value_type is str:
        return repr(value) if len(value) <= 80 else f"{len(value)} chars"
    if value_type in (bytes, bytearray):
        return f"{len(value)} bytes"
    if value_type is dict:
        return f"{len(value)} keys"
    if value_type in (list, tuple, set, frozenset):
        return f"{len(value)} items"

    module = value_type.__module__.split(".", 1)[0]
    if module in {"anndata", "numpy", "pandas", "polars", "pyarrow", "scipy", "torch", "xarray"}:
        try:
            shape = value.shape
            if (
                isinstance(shape, (list, tuple))
                and len(shape) <= 8
                and all(item is None or type(item) is int for item in shape)
            ):
                return " × ".join("?" if item is None else str(item) for item in shape)
        except Exception:
            pass
    return ""


def _object_size(value):
    value_type = type(value)
    module = value_type.__module__.split(".", 1)[0]
    try:
        if module == "numpy":
            return int(value.nbytes)
        if module == "pandas":
            usage = value.memory_usage(index=True, deep=False)
            return int(usage.sum() if hasattr(usage, "sum") else usage)
        if value_type.__module__ == "builtins":
            return int(sys.getsizeof(value))
    except Exception:
        pass
    return None


def _inspect_objects(namespace):
    values = [
        (name, value)
        for name, value in namespace.items()
        if isinstance(name, str)
        and not name.startswith("_")
        and not isinstance(value, types.ModuleType)
    ]
    values.sort(key=lambda item: item[0].casefold())
    objects = [
        {
            "name": name[:MAX_NAME_SIZE],
            "typeName": type(value).__name__[:MAX_META_SIZE],
            "summary": _object_summary(value)[:MAX_META_SIZE],
            "sizeBytes": _object_size(value),
        }
        for name, value in values[:MAX_OBJECTS]
    ]
    return {"objects": objects, "totalCount": len(values)}


def _error_lineno(exc, cell_tag):
    tb = getattr(exc, "__traceback__", None)
    lineno = None
    while tb is not None:
        if tb.tb_frame.f_code.co_filename == cell_tag:
            lineno = tb.tb_lineno
        tb = tb.tb_next
    return lineno


def _configure_pandas():
    try:
        import pandas as pd  # type: ignore

        pd.set_option("display.max_columns", None)
        pd.set_option("display.max_rows", 500)
        pd.set_option("display.max_colwidth", None)
        pd.set_option("display.width", None)
        pd.set_option("display.expand_frame_repr", False)
    except Exception:
        pass


_EXEC_PREFIXES = (
    "import ", "from ", "def ", "class ", "if ", "for ", "while ",
    "with ", "try:", "try ", "except ", "finally:", "elif ", "else:",
    "raise ", "return ", "del ", "global ", "nonlocal ", "assert ",
    "async ", "match ", "case ", "yield ", "@",
)


def _looks_like_exec(code: str) -> bool:
    """Heuristic: multi-line or statement-leading cells should skip eval."""
    stripped = code.strip()
    if not stripped:
        return True
    if "\n" in stripped:
        return True
    head = stripped.lstrip()
    return any(head.startswith(p) for p in _EXEC_PREFIXES)


def _kernel_init(namespace: dict) -> None:
    """Pre-import common stdlib and optional deps into the persistent namespace."""
    exec(compile(
        "import json, math, os, re, sys, urllib.parse, urllib.request",
        "<wisp-kernel:init>",
        "exec",
    ), namespace)
    for mod in ("requests", "numpy", "pandas"):
        try:
            namespace[mod] = __import__(mod)
        # These are conveniences, not runtime dependencies. A missing package
        # or broken optional native wheel must not prevent the ready handshake.
        except Exception:
            pass
    _configure_pandas()


def _execute_cell(code: str, cell_tag: str, namespace: dict) -> None:
    """Run one cell as eval (expression) or exec (statements)."""
    if _looks_like_exec(code):
        exec(compile(code, cell_tag, "exec"), namespace)
        return
    try:
        compiled = compile(code, cell_tag, "eval")
    except SyntaxError:
        try:
            exec(compile(code, cell_tag, "exec"), namespace)
        except SyntaxError as e:
            raise e from None
        return
    result = eval(compiled, namespace)
    if result is not None:
        print(repr(result))


def main():
    import threading

    # Move the protocol pipes off fd 0/1 so user subprocesses inheriting the
    # handles don't corrupt the stream. On Windows we dup to new handles.
    protocol_in = os.fdopen(os.dup(0), "r", encoding="utf-8", errors="replace")
    protocol_out = os.fdopen(os.dup(1), "w", encoding="utf-8", errors="replace", buffering=1)
    devnull = os.open(os.devnull, os.O_RDONLY)
    os.dup2(devnull, 0)
    os.dup2(os.open(os.devnull, os.O_WRONLY), 1)
    protocol_lock = threading.Lock()

    namespace = {"__name__": "__main__", "__builtins__": __builtins__}
    cell_counter = 0

    # Configure pandas on first import.
    _orig_import = builtins.__import__

    def import_wrapper(name, *a, **k):
        mod = _orig_import(name, *a, **k)
        if name == "pandas":
            _configure_pandas()
        elif name.startswith("matplotlib"):
            _neutralize_pyplot_show()
        return mod

    builtins.__import__ = import_wrapper
    _kernel_init(namespace)
    protocol_out.write(json.dumps({
        "type": "ready",
        "protocol": 1,
        "language": "python",
        "pid": os.getpid(),
        "version": sys.version.split()[0],
    }) + "\n")
    protocol_out.flush()

    while True:
        line = protocol_in.readline()
        if not line:
            break
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            protocol_out.write(json.dumps({"type": "result", "id": "unknown", "stdout": "", "stderr": "", "error": f"Invalid JSON: {e}"}) + "\n")
            protocol_out.flush()
            continue
        if not isinstance(req, dict):
            continue

        rid = req.get("id", "unknown")
        if req.get("type") == "inspect":
            inspection = _inspect_objects(namespace)
            with protocol_lock:
                protocol_out.write(json.dumps({
                    "type": "objects",
                    "id": rid,
                    **inspection,
                }) + "\n")
                protocol_out.flush()
            continue
        if req.get("type") != "execute":
            continue

        code = req.get("code", "")
        cell_counter += 1
        cell_tag = f"<wisp-kernel:{cell_counter}>"

        import linecache as _lc
        _lc.cache[cell_tag] = (len(code), None, code.splitlines(True), cell_tag)

        stdout_cap = _StreamingStdout(protocol_out, protocol_lock, rid)
        stderr_cap = _CappedStream()
        error = None
        error_lineno = None

        wall0 = time.perf_counter()
        cpu0 = time.process_time()
        old_out, old_err = sys.stdout, sys.stderr
        try:
            sys.stdout = stdout_cap
            sys.stderr = stderr_cap
            try:
                _execute_cell(code, cell_tag, namespace)
            except BaseException as e:  # noqa: BLE001 — survive hostile exceptions
                error = traceback.format_exc()
                error_lineno = _error_lineno(e, cell_tag)
        finally:
            stdout_cap._active = False
            sys.stdout = old_out
            sys.stderr = old_err

        usage = {
            "wall_s": round(time.perf_counter() - wall0, 3),
            "cpu_s": round(time.process_time() - cpu0, 3),
            "rss_kb": _try_psutil_rss_kb(),
        }
        resp = {
            "type": "result",
            "id": rid,
            "stdout": _truncate(stdout_cap.getvalue()),
            "stderr": _truncate(stderr_cap.getvalue()),
            "error": error,
            "interrupted": False,
            "trace": {"error_lineno": error_lineno, "error_call": None},
            "usage": usage,
        }
        with protocol_lock:
            protocol_out.write(json.dumps(resp) + "\n")
            protocol_out.flush()


if __name__ == "__main__":
    main()
