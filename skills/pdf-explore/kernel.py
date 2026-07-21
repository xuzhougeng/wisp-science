"""
Kernel sidecar for the pdf-explore skill (wisp edition).

NOT auto-injected: `use_skill` appends a "Python Kernel Sidecar" section
with a one-time ``exec(compile(open(...).read(), "kernel.py", "exec"))``
loading instruction; definitions then persist in the project's python
kernel across cells. Top level is definition-only and all non-stdlib
imports live inside function bodies, so the load itself never fails on a
missing package. All names are ``pdf_``-prefixed since the sidecar shares
the kernel's ``__main__``.

Surface:
    pdf_pages(path, ...)   — parse → [{page, text, n_chars, image_path?}]
    pdf_outline(path)      — embedded-bookmark TOC (no LLM fallback here)
    pdf_resolve(path)      — ~ expansion + existence-friendly resolution

The upstream skill also shipped LLM fan-out helpers (pdf_scan /
pdf_extract / pdf_map and an LLM outline fallback). They need an
in-kernel model-call bridge the wisp host does not provide; they are
removed rather than left to fail at call time.
"""

import hashlib
import os
import re

PDF_PAGE_CACHE = {}
"""(abs_path, mtime, mode, dpi) → [{page, text, n_chars, image_path?}].

Module-level so repeat reads of the same file skip re-parsing and
re-rendering. Cleared only on kernel restart."""

PDF_AUTO_IMAGE_CHARS_THRESHOLD = 80
"""Mean chars/page below which mode="auto" treats the PDF as scanned."""

PDF_INSTALL_HINT = (
    "Install pypdfium2 (and pillow for mode='image') into the project's "
    "python environment — e.g. `pixi add pypdfium2 pillow` from the "
    "project directory, or pip in the active venv — then re-run."
)


def pdf_resolve(path_or_id):
    """Expand ``~`` and return a local filesystem path.

    Artifact/version ids from the reference host are not supported in
    wisp — a UUID-shaped string that is not an existing path raises with
    a clear message instead of resolving.
    """
    if not isinstance(path_or_id, str) or not path_or_id:
        raise TypeError("pdf_resolve: path_or_id must be a non-empty str")
    p = os.path.expanduser(path_or_id)
    if os.path.exists(p):
        return p
    if re.fullmatch(r"[0-9a-fA-F-]{32,36}", path_or_id.strip()):
        raise FileNotFoundError(
            f"pdf_resolve: {path_or_id!r} looks like an artifact id; "
            f"artifact resolution is not available here — pass a file path."
        )
    return p


def pdf_pages(path, mode="auto", pages=None, dpi=100, cache=True):
    """Parse a PDF into a per-page list. Cached on (path, mtime, mode, dpi).

    Returns ``[{"page": 1-indexed int, "text": str, "n_chars": int,
    "image_path": str|None}, ...]``.

    ``mode``:
        "auto"  — (default) try text extraction first; if the mean page
                  has fewer than :data:`PDF_AUTO_IMAGE_CHARS_THRESHOLD`
                  characters (i.e. a scanned/image-only PDF), switch to
                  image mode. No extra cost on text-layer PDFs.
        "text"  — text extraction only (cheap; misses figures/scans)
        "image" — render each page to
                  ``./.cache/pdf-explore/{sha8}-{mtime}/dpi{N}/p{NNN}.png``
                  at ``dpi`` (default 100; ~1200×1600 for letter-size)
        "both"  — text + image

    ``pages``: optional 1-indexed list/range to restrict to (e.g. ``[3,4,5]``
    or ``range(1,11)``). With ``cache=True`` only a FULL read populates the
    in-memory cache; a later subset read is served from it for free, but a
    cold subset read re-parses each time (page renders are still reused on
    disk via the ``.cache/pdf-explore`` dir).

    Requires ``pypdfium2`` (permissively licensed). Falls back to
    ``pymupdf`` if the user installed it, then to ``pypdf`` for text-only
    mode. Raises ``ImportError`` with an install hint if none is available.
    """
    path = pdf_resolve(path)
    if not os.path.exists(path):
        raise FileNotFoundError(f"pdf_pages: {path!r} not found")
    if mode not in ("text", "image", "both", "auto"):
        raise ValueError(
            f"pdf_pages: mode must be 'text'|'image'|'both'|'auto', got {mode!r}"
        )
    # mode="auto" passes `pages` to two recursive calls — materialize a
    # one-shot iterable (generator/filter/iter) so the second call doesn't
    # see an exhausted object and silently return [].
    if pages is not None and not hasattr(pages, "__len__"):
        pages = list(pages)
    if mode == "auto":
        # Auto-detect scanned/image-only PDFs: parse text first, and if the
        # mean page has almost no extractable text (<80 chars — threshold
        # catches rasterized scans and slide-deck exports while leaving
        # sparse figure-pages alone), re-parse with rendering. Both parses
        # are cached independently so a re-scan is free.
        txt = pdf_pages(path, mode="text", pages=pages, dpi=dpi, cache=cache)
        if not txt:
            return txt
        mean_chars = sum(p["n_chars"] for p in txt) / len(txt)
        if mean_chars < PDF_AUTO_IMAGE_CHARS_THRESHOLD:
            return pdf_pages(path, mode="image", pages=pages, dpi=dpi,
                             cache=cache)
        return txt

    abspath = os.path.abspath(path)
    mtime = os.stat(abspath).st_mtime_ns
    key = (abspath, mtime, mode, int(dpi))
    want = None if pages is None else set(int(p) for p in pages)
    if cache and key in PDF_PAGE_CACHE:
        cached = PDF_PAGE_CACHE[key]
        if want is None:
            return [dict(p) for p in cached]
        hit = [dict(p) for p in cached if p["page"] in want]
        if len(hit) == len(want):
            return hit

    render = mode in ("image", "both")
    need_text = mode in ("text", "both")
    out = []
    img_dir = None
    if render:
        sha8 = hashlib.sha1(abspath.encode()).hexdigest()[:8]
        # Renders live under .cache/ so nothing scans them into context by
        # accident. View a chosen page explicitly with the view_image/read
        # tool on its image_path — each viewed image persists in context
        # (and only ages out at /compact), so view the few pages that
        # matter, not the whole render set. Keyed on mtime + dpi so a
        # re-render at a different dpi, or after the PDF is modified in
        # place, doesn't silently reuse stale PNGs.
        img_dir = os.path.join(
            os.getcwd(), ".cache", "pdf-explore",
            f"{sha8}-{mtime}", f"dpi{int(dpi)}",
        )
        os.makedirs(img_dir, exist_ok=True)

    try:
        import pypdfium2 as pdfium
    except ImportError:
        pdfium = None
    # pypdfium2's to_pil() lazy-imports PIL.Image; without pillow the render
    # path dies with a bare ModuleNotFoundError instead of the install hint
    # below. When rendering is requested and pillow is absent, demote pdfium
    # so fitz (pix.save() writes PNG natively, no PIL dep) or the install
    # hint gets a chance. Text-only pdfium needs no pillow — keep it for
    # mode="text".
    if pdfium is not None and render:
        try:
            import PIL.Image  # noqa: F401
        except ImportError:
            pdfium = None
    fitz = None
    if pdfium is None:
        try:
            import fitz  # pymupdf — user-installed fallback (AGPL-3.0)
        except ImportError:
            pass

    if pdfium is not None:
        try:
            doc = pdfium.PdfDocument(abspath)
        except Exception as e:
            if "password" in str(e).lower():
                raise ValueError(
                    f"pdf_pages: {path!r} is password-protected. Decrypt "
                    f"it first (e.g. `qpdf --decrypt --password=... in out` "
                    f"or pypdfium2.PdfDocument(path, password=pw))."
                ) from e
            raise
        try:
            total = len(doc)
            idxs = (
                range(total) if want is None
                else sorted(i - 1 for i in want if 1 <= i <= total)
            )
            for i in idxs:
                pg = doc[i]
                txt = ""
                if need_text:
                    tp = pg.get_textpage()
                    # pdfium emits \r\n line endings — normalize so char
                    # counts/thresholds match the historical extractor.
                    txt = tp.get_text_bounded().replace("\r\n", "\n")
                    tp.close()
                ip = None
                if render:
                    ip = os.path.join(img_dir, f"p{i + 1:03d}.png")
                    if not (cache and os.path.exists(ip)):
                        # dpi→scale: PDF native is 72dpi.
                        bmp = pg.render(scale=float(dpi) / 72.0)
                        bmp.to_pil().save(ip)
                out.append({
                    "page": i + 1,
                    "text": txt,
                    "n_chars": len(txt),
                    "image_path": ip,
                })
        finally:
            doc.close()
    elif fitz is not None:
        doc = fitz.open(abspath)
        try:
            if doc.needs_pass:
                raise ValueError(
                    f"pdf_pages: {path!r} is password-protected. Decrypt "
                    f"it first (e.g. `qpdf --decrypt --password=... in out` "
                    f"or `fitz.open(path).authenticate(pw)`)."
                )
            total = doc.page_count
            idxs = (
                range(total) if want is None
                else sorted(i - 1 for i in want if 1 <= i <= total)
            )
            for i in idxs:
                pg = doc.load_page(i)
                txt = pg.get_text("text") if need_text else ""
                ip = None
                if render:
                    ip = os.path.join(img_dir, f"p{i + 1:03d}.png")
                    if not (cache and os.path.exists(ip)):
                        # dpi→zoom: PDF native is 72dpi.
                        zoom = float(dpi) / 72.0
                        pix = pg.get_pixmap(matrix=fitz.Matrix(zoom, zoom))
                        pix.save(ip)
                out.append({
                    "page": i + 1,
                    "text": txt,
                    "n_chars": len(txt),
                    "image_path": ip,
                })
        finally:
            doc.close()
    else:
        if render:
            raise ImportError(
                "pdf_pages(mode='image'|'both') requires pypdfium2 and "
                "pillow (PNG encoding). " + PDF_INSTALL_HINT
            )
        try:
            from pypdf import PdfReader
        except ImportError as e:
            raise ImportError(
                "pdf_pages requires pypdfium2 or pypdf. " + PDF_INSTALL_HINT
            ) from e
        reader = PdfReader(abspath)
        total = len(reader.pages)
        idxs = (
            range(total) if want is None
            else sorted(i - 1 for i in want if 1 <= i <= total)
        )
        for i in idxs:
            txt = reader.pages[i].extract_text() or ""
            out.append({
                "page": i + 1,
                "text": txt,
                "n_chars": len(txt),
                "image_path": None,
            })

    if cache and want is None:
        PDF_PAGE_CACHE[key] = [dict(p) for p in out]
    return out


def pdf_outline(path):
    """Table of contents from the PDF's embedded outline: ``[{"page": int,
    "heading": str, "level": int}, ...]`` in page order.

    Free and instant — most LaTeX-sourced arXiv PDFs have embedded
    bookmarks. Returns ``[]`` (with a printed hint) when the PDF has none;
    there is no LLM fallback in this host — skim
    ``pdf_pages(path, mode="text")`` headings instead.

    Use this as the first step for navigating any structured document::

        toc = pdf_outline("paper.pdf")
        for e in toc:
            print(f"p{e['page']:>3} {'  ' * (e['level'] - 1)}{e['heading']}")
    """
    abspath = os.path.abspath(pdf_resolve(path))
    toc = None
    try:
        import pypdfium2 as pdfium
        doc = pdfium.PdfDocument(abspath)
        try:
            toc = []
            for bm in doc.get_toc():
                dest = bm.get_dest()
                idx = dest.get_index() if dest else None
                # [level, title, 1-indexed page] — same shape as the
                # historical fitz get_toc(simple=True); unresolvable
                # destinations map to page 0 and are dropped below.
                toc.append([bm.level + 1, bm.get_title(),
                            (idx + 1) if idx is not None else 0])
        finally:
            doc.close()
    except Exception:  # noqa: BLE001
        try:
            import fitz  # pymupdf — user-installed fallback
            with fitz.open(abspath) as doc:
                toc = doc.get_toc(simple=True)  # [[lv, title, page], ...]
        except Exception:  # noqa: BLE001
            toc = None
    if toc:
        fast = [{"page": int(p), "heading": str(t), "level": int(lv)}
                for lv, t, p in toc if p > 0]
        if fast:
            # Sanity check: embedded bookmarks sometimes point to
            # document-logical pages (e.g. a LaTeX thesis whose hyperref
            # anchors were generated before 20 pages of front-matter were
            # prepended), so page N in the TOC is really PDF page N+offset.
            # Verify 2-3 level-1 entries against the actual page text; warn
            # if none match.
            try:
                import unicodedata as _ud

                def _norm(s):
                    return "".join(c for c in _ud.normalize("NFKD", s)
                                   if c.isalnum()).lower()
                probes = [e for e in fast if e["level"] == 1][:3] or fast[:3]
                probe_pages = pdf_pages(
                    abspath, pages=[e["page"] for e in probes], mode="text")
                by_pg = {p["page"]: p["text"] for p in probe_pages}
                hits = 0
                for e in probes:
                    h = _norm(e["heading"])[:40]
                    t = _norm(by_pg.get(e["page"], "")[:1200])
                    if h and h in t:
                        hits += 1
                # A scanned PDF (no text layer) yields empty probe text —
                # that's "can't verify", not "offset bookmarks"; stay quiet
                # rather than mis-diagnose.
                has_text_layer = any(
                    len(by_pg.get(e["page"], "").strip())
                    >= PDF_AUTO_IMAGE_CHARS_THRESHOLD
                    for e in probes
                )
                if probes and hits == 0 and has_text_layer:
                    print(
                        "[pdf_outline] ⚠ embedded TOC page numbers don't "
                        "match page text for any of "
                        f"{len(probes)} sampled entries — the PDF's "
                        "bookmarks likely use logical page numbers, not "
                        "file page numbers (front-matter offset). Verify "
                        "one entry against "
                        "pdf_pages(path, pages=[N])[0]['text'] before "
                        "navigating."
                    )
            except Exception:  # noqa: BLE001
                pass  # best-effort sanity check only
            return fast
    print(
        "[pdf_outline] no embedded outline in this PDF — skim headings via "
        "pdf_pages(path, mode='text') (e.g. print the first lines of each "
        "page) to build your own map."
    )
    return []
