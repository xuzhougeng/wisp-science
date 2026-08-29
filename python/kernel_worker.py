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
           "usage": {"wall_s": 0.0, "cpu_s": 0.0, "rss_kb": 0},
           "files_written": ["<path>", ...],  # omitted if unobserved/truncated
           "files_written_base": "project",  # present for configured relative reports
           "plots": ["<base64 png>", ...]}  # omitted when the cell drew nothing

`files_written` lists the files this cell actually changed, not the ones it
opened for writing; see `_WriteObserver`.

This is a Windows-friendly port of the upstream wisp-science
`kernels/kernel_worker.py`: the POSIX-only `resource`, `/proc`, and
delivered-SIGINT discipline are dropped. RSS comes from `psutil` when
installed (else 0). Per-cell interrupt is not supported in this MVP —
long-running cells block until they return.
"""

import base64
import builtins
from collections import deque
import io
import json
import os
import stat
import sys
import time
import traceback
import types

MAX_OUTPUT_SIZE = 1024 * 1024  # 1 MB head cap on stdout/stderr
MAX_OBJECTS = 200
MAX_NAME_SIZE = 256
MAX_META_SIZE = 160
MAX_LINECACHE_CELLS = 64
MAX_LINECACHE_BYTES = 8 * 1024 * 1024
MAX_CODE_SIZE = 1024 * 1024
MAX_CODE_LINES = 20_000
MAX_REQUEST_SIZE = 8 * 1024 * 1024
# Cap on actual, project-eligible changes reported per cell. Exceeding it
# omits the field entirely: a truncated list would look authoritative while
# being wrong. Candidate intent has a separate, larger memory guard so paths
# later proven unchanged do not consume this semantic cap.
MAX_REPORTED_WRITES = 512
MAX_OBSERVED_WRITE_CANDIDATES = 4096
MAX_OBSERVED_WRITE_PATH_BYTES = 4 * 1024 * 1024
MAX_PLOTS_PER_CELL = 8
_WRITE_FLAGS = (
    os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC
)

# Force a non-interactive matplotlib backend before matplotlib is ever imported.
# Without this, generated plotting code (plt.show(), scanpy sc.pl.*) selects the
# platform GUI backend (MacOSX/Tk/Qt) and plt.show() opens a window that BLOCKS
# the kernel until the user closes it, stalling the whole analysis (issue #37).
# Figures are meant to be surfaced via savefig, never a GUI window.
os.environ["MPLBACKEND"] = "Agg"


def _open_has_write_intent(mode, flags) -> bool:
    if isinstance(mode, str) and any(ch in mode for ch in "wax+"):
        return True
    if isinstance(flags, int) and flags & _WRITE_FLAGS:
        return True
    return False


def _file_state(path):
    """`(size, mtime_ns)` of a regular file, or None when it is not one.

    Sampled twice per candidate path — once when the audit event fires and
    once after the cell — so an unchanged pair proves the bytes never moved.
    """
    try:
        info = os.stat(path)
    except Exception:
        return None
    if not stat.S_ISREG(info.st_mode):
        return None
    return (info.st_size, info.st_mtime_ns)


def _is_bytecode_cache(path) -> bool:
    """True for anything under a `__pycache__` directory.

    Importing a project-local module writes bytecode through `os.replace`,
    which the hook sees. The host discards those paths anyway (its workspace
    snapshot skips the directory), but they used to consume the report cap
    first, so a cell that imported enough modules lost its real outputs.
    Separators are normalized only on Windows: on Unix a literal `\\` is a
    legal filename character.
    """
    normalized = path.replace("\\", "/") if os.name == "nt" else path
    return "__pycache__" in normalized.split("/")[:-1]


def _path_components(path):
    """Split an OS-native relative path without rewriting Unix backslashes."""
    normalized = path.replace(os.altsep, os.sep) if os.altsep else path
    return [part for part in normalized.split(os.sep) if part not in ("", ".")]


class _WriteObserver:
    """Collect the paths this interpreter changed during one cell.

    Audit hooks cannot be unregistered, so per-cell collection is toggled
    with begin() / finish(). The hook body is wrapped in a broad
    try/except and returns immediately while collection is off: a raise
    would propagate into arbitrary user code, including C-extension I/O.

    The `open` audit event carries write *intent*, not proof of a write, and
    it fires before the OS call completes. So each candidate's pre-open state
    is sampled when the event arrives and compared again in finish(): a cell
    that opens a file `'r+'` or `'a'` and writes nothing must not be credited
    with it (`h5py.File(p, "a")` is the idiomatic way to open a store you
    then only read).
    """

    def __init__(self):
        self._active = False
        self._paths = []
        self._before = {}
        self._relative = {}
        self._path_bytes = 0
        self._truncated = False
        self._project_root = None
        self._skip_dirs = frozenset()

    def configure(self, root, skip_dirs):
        """Set the host-owned project boundary used by subsequent cells."""
        if self._active:
            raise ValueError("cannot configure write scope during a cell")
        if not isinstance(root, str) or not root:
            raise ValueError("write scope root must be a non-empty string")
        if not isinstance(skip_dirs, list):
            raise ValueError("write scope skip_dirs must be a list")
        cleaned = []
        for name in skip_dirs:
            if (
                not isinstance(name, str)
                or not name
                or name in (".", "..")
                or "/" in name
                or "\\" in name
            ):
                raise ValueError("write scope skip_dirs contains an invalid name")
            cleaned.append(name)
        project_root = os.path.realpath(os.path.abspath(root))
        if not os.path.isdir(project_root):
            raise ValueError("write scope root is not an existing directory")
        self._project_root = project_root
        self._skip_dirs = frozenset(cleaned)

    @property
    def report_base(self):
        return "project" if self._project_root is not None else None

    def begin(self):
        self._active = True
        self._paths = []
        self._before = {}
        self._relative = {}
        self._path_bytes = 0
        self._truncated = False

    def finish(self):
        self._active = False
        if self._truncated:
            self._reset_cell()
            return None
        out = []
        for path in self._paths:
            try:
                after = _file_state(path)
                # Gone or never created (a failed open fires the event too),
                # or byte-for-byte the file we saw before the open.
                if after is None or after == self._before[path]:
                    continue
                out.append(self._relative.get(path, path))
                if len(out) > MAX_REPORTED_WRITES:
                    self._reset_cell()
                    return None
            except Exception:
                pass
        self._reset_cell()
        return out

    def _reset_cell(self):
        self._paths = []
        self._before = {}
        self._relative = {}
        self._path_bytes = 0

    def _project_relative(self, resolved):
        if self._project_root is None:
            return None
        candidate = os.path.realpath(resolved)
        try:
            common = os.path.commonpath([self._project_root, candidate])
        except (OSError, ValueError):
            return False
        if os.path.normcase(common) != os.path.normcase(self._project_root):
            return False
        relative = os.path.relpath(candidate, self._project_root)
        if relative in ("", "."):
            return False
        components = _path_components(relative)
        if any(name in self._skip_dirs for name in components[:-1]):
            return False
        return "/".join(components)

    def _note(self, path):
        if not self._active or self._truncated:
            return
        try:
            resolved = os.path.abspath(os.fspath(path))
            if isinstance(resolved, bytes):
                resolved = os.fsdecode(resolved)
            # The path must survive the JSON protocol: bytes are not
            # serializable at all, and surrogate escapes from undecodable
            # filenames are rejected by the host's strict JSON parser.
            # Skip such paths — the host's snapshot inference still covers
            # them, and skipping can never suppress it (reports only add).
            resolved.encode("utf-8", "strict")
        except Exception:
            return
        if self._project_root is not None:
            resolved = os.path.realpath(resolved)
        relative = self._project_relative(resolved)
        if relative is False:
            return
        if relative is None and _is_bytecode_cache(resolved):
            return
        if resolved in self._before:
            return
        encoded_bytes = len(resolved.encode("utf-8"))
        if (
            len(self._paths) >= MAX_OBSERVED_WRITE_CANDIDATES
            or self._path_bytes + encoded_bytes > MAX_OBSERVED_WRITE_PATH_BYTES
        ):
            self._truncated = True
            return
        self._before[resolved] = _file_state(resolved)
        self._paths.append(resolved)
        self._path_bytes += encoded_bytes
        if relative is not None:
            self._relative[resolved] = relative

    def hook(self, event, args):
        try:
            if not self._active or self._truncated:
                return
            if event == "open":
                path = args[0] if args else None
                mode = args[1] if len(args) > 1 else None
                flags = args[2] if len(args) > 2 else None
                if path is not None and _open_has_write_intent(mode, flags):
                    self._note(path)
            elif event in ("os.rename", "os.replace") and len(args) > 1:
                self._note(args[1])
        except Exception:
            return


_WRITE_OBSERVER = _WriteObserver()


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


def _collect_plots():
    """Base64 PNGs of the matplotlib figures a cell left open, oldest first.

    Notebook semantics: figures are harvested and closed at the end of every
    cell, so the next cell starts fresh and nothing is reported twice. Empty
    figures (a bare `plt.figure()`) are skipped. Never raises — plot capture
    is a courtesy, not a reason to lose the cell's result.
    """
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return []
    out = []
    try:
        for num in plt.get_fignums()[:MAX_PLOTS_PER_CELL]:
            fig = plt.figure(num)
            if not fig.get_axes():
                continue
            buf = io.BytesIO()
            fig.savefig(buf, format="png")
            out.append(base64.b64encode(buf.getvalue()).decode("ascii"))
    except Exception:
        pass
    finally:
        try:
            plt.close("all")
        except Exception:
            pass
    return out


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
    sys.addaudithook(_WRITE_OBSERVER.hook)
    _kernel_init(namespace)
    protocol_out.write(json.dumps({
        "type": "ready",
        "protocol": 1,
        "language": "python",
        "pid": os.getpid(),
        "version": sys.version.split()[0],
    }) + "\n")
    protocol_out.flush()

    linecache_cells = deque()
    linecache_bytes = 0
    while True:
        line = protocol_in.readline(MAX_REQUEST_SIZE + 1)
        if not line:
            break
        if len(line) > MAX_REQUEST_SIZE:
            protocol_out.write(json.dumps({
                "type": "result", "id": "unknown", "stdout": "", "stderr": "",
                "error": f"Request exceeds {MAX_REQUEST_SIZE} character limit",
            }) + "\n")
            protocol_out.flush()
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
        if req.get("type") == "configure":
            try:
                scope = req.get("write_scope")
                if not isinstance(scope, dict):
                    raise ValueError("write_scope must be an object")
                _WRITE_OBSERVER.configure(
                    scope.get("root"),
                    scope.get("skip_dirs"),
                )
                configured = {"type": "configured", "id": rid}
            except Exception as error:
                configured = {
                    "type": "configure_error",
                    "id": rid,
                    "error": str(error),
                }
            protocol_out.write(json.dumps(configured) + "\n")
            protocol_out.flush()
            continue
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
        if not isinstance(code, str):
            code = ""
        code_size = len(code.encode("utf-8"))
        code_lines = code.count("\n") + code.count("\r") - code.count("\r\n") + 1
        if code_size > MAX_CODE_SIZE or code_lines > MAX_CODE_LINES:
            protocol_out.write(json.dumps({
                "type": "result", "id": rid, "stdout": "", "stderr": "",
                "error": (
                    f"Code exceeds {MAX_CODE_SIZE} byte or {MAX_CODE_LINES} line limit"
                ),
            }) + "\n")
            protocol_out.flush()
            continue

        required_objects = req.get("required_objects", [])
        if not isinstance(required_objects, list) or not all(
            isinstance(name, str) and name for name in required_objects
        ):
            protocol_out.write(json.dumps({
                "type": "result", "id": rid, "stdout": "", "stderr": "",
                "error": "required_objects must be an array of non-empty strings",
            }) + "\n")
            protocol_out.flush()
            continue
        missing_objects = [name for name in required_objects if name not in namespace]
        if missing_objects:
            protocol_out.write(json.dumps({
                "type": "result", "id": rid, "stdout": "", "stderr": "",
                "error": (
                    "required runtime objects are missing: "
                    + ", ".join(missing_objects)
                ),
            }) + "\n")
            protocol_out.flush()
            continue

        cell_counter += 1
        source_name = req.get("source_name")
        cell_tag = (
            source_name
            if isinstance(source_name, str) and source_name
            else f"<wisp-kernel:{cell_counter}>"
        )

        import linecache as _lc
        if cell_tag in _lc.cache:
            retained_cells = []
            for old_tag, old_size in linecache_cells:
                if old_tag == cell_tag:
                    linecache_bytes -= old_size
                else:
                    retained_cells.append((old_tag, old_size))
            linecache_cells.clear()
            linecache_cells.extend(retained_cells)
            _lc.cache.pop(cell_tag, None)
        while linecache_cells and (
            len(linecache_cells) >= MAX_LINECACHE_CELLS
            or linecache_bytes + code_size > MAX_LINECACHE_BYTES
        ):
            old_tag, old_size = linecache_cells.popleft()
            _lc.cache.pop(old_tag, None)
            linecache_bytes -= old_size
        _lc.cache[cell_tag] = (len(code), None, code.splitlines(True), cell_tag)
        linecache_cells.append((cell_tag, code_size))
        linecache_bytes += code_size

        stdout_cap = _StreamingStdout(protocol_out, protocol_lock, rid)
        stderr_cap = _CappedStream()
        error = None
        error_lineno = None

        wall0 = time.perf_counter()
        cpu0 = time.process_time()
        old_out, old_err = sys.stdout, sys.stderr
        files_written = None
        _WRITE_OBSERVER.begin()
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
            files_written = _WRITE_OBSERVER.finish()
        plots = _collect_plots()

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
        # Absent ≠ empty: omit the field when the observer could not produce a
        # complete list (cap exceeded). An explicit [] means "wrote nothing".
        if files_written is not None:
            resp["files_written"] = files_written
            if _WRITE_OBSERVER.report_base is not None:
                resp["files_written_base"] = _WRITE_OBSERVER.report_base
        if plots:
            resp["plots"] = plots
        with protocol_lock:
            protocol_out.write(json.dumps(resp) + "\n")
            protocol_out.flush()


if __name__ == "__main__":
    main()
