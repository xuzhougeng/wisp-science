---
name: pdf-explore
description: "Use this skill when the user has attached a PDF, paper, report, or other document and the answer needs its content: summarize a section, compare sections, read specific pages, check the table of contents, or read a value off a figure. The `read` tool cannot parse PDF binary — python is the extraction path. Provides `pdf_pages` (pages as text or rendered PNGs, cached) and `pdf_outline` (embedded-bookmark TOC) in the persistent python kernel; load them once via the Kernel Sidecar exec line that `use_skill` appends. For PDF creation/manipulation, use reportlab/pypdf directly."
fold_cue: "instead_of=read use=pdf_pages/pdf_outline for PDFs — read cannot parse PDF binary; print ≤5 pages, else write to a file and read that"
license: Apache-2.0
---

# PDF Explore — navigate a PDF without flooding your context

The `read` tool cannot parse PDFs (binary), and a 50-page PDF pasted
wholesale is ~40K+ tokens. This skill parses the PDF **once** in the
persistent python kernel (disk + memory cached) so you load only the
pages that matter.

**Load first (once per session):** run the `exec(...)` line from the
"Python Kernel Sidecar" section this skill's `use_skill` output ends
with. Definitions persist across cells; re-run only after a kernel
restart. Requires `pypdfium2` (plus `pillow` for image mode) — if the
first call raises ImportError, install per its hint and re-run.

## Which helper

| | when | returns |
|---|---|---|
| **`pdf_outline(path)`** | structured doc (paper, report, book) — try this first | `[{page, heading, level}, ...]` from embedded bookmarks; `[]` + hint if none |
| **`pdf_pages(path, pages=[...], mode="text")`** | the pages/sections you actually need | `[{page, text, n_chars}, ...]` |
| **`pdf_pages(path, mode="image", dpi=200, pages=[N])`** | figures, scanned pages | PNG per page under `.cache/pdf-explore/`; view via `view_image` |
| `mode="auto"` (default) | unknown PDF | text; flips to image when pages have no text layer (scans) |

## Recipe — navigate by outline (try this first)

```python
for e in pdf_outline("paper.pdf"):
    print(f"p{e['page']:>3} {'  ' * (e['level'] - 1)}{e['heading']}")
```

Free and instant when the PDF has embedded bookmarks (most
LaTeX-compiled papers do). No LLM fallback in this host: if it returns
`[]`, skim `pdf_pages(path, mode="text")` first lines per page to build
your own map.

## Recipe — read a few pages (≤ ~5)

```python
for p in pdf_pages("paper.pdf", pages=[3, 4, 5], mode="text"):
    print(f"\n── page {p['page']} ──\n{p['text']}")
```

Printing is fine at this scale (~2–4KB/page). Python output beyond the
context budget (~16KB) gets head/tail-truncated at ingestion — so for
anything bigger, use the next recipe instead of printing.

## Recipe — pull whole sections for synthesis

For "summarize the methods" / "compare section 3 and 5" / anything
drawing on several page ranges, write the pages to a file in **one**
call, then `read` that file — `read` results enter context whole:

```python
wanted = [5, 21, 22, 23, 24, 25, 62, 63, 64]   # from pdf_outline
with open("sections.txt", "w") as f:
    for p in pdf_pages("paper.pdf", pages=wanted, mode="text"):
        f.write(f"\n── page {p['page']} ──\n{p['text']}")
import os; print(f"wrote {os.path.getsize('sections.txt'):,} bytes")
```

Then `read` `sections.txt` (with `offset`/`limit` if it is large).
~800 tokens/page as text vs ~8K tokens as an attached image — and you
pay it once.

## Recipe — read a figure in detail

A full page render is too low-res to read axis labels off a dense
figure. Render high-DPI, crop the figure region with PIL, then view the
crop:

```python
p = pdf_pages("paper.pdf", mode="image", pages=[5], dpi=200)[0]
from PIL import Image
Image.open(p["image_path"]).crop((x0, y0, x1, y1)).save("fig_p5.png")
```

Then call `view_image` on `fig_p5.png` (or the full `image_path` once to
locate the figure). Viewed images persist in context until `/compact`
ages them — view the few crops that matter, not every page.

## Not available in this host

The upstream skill's LLM fan-out helpers (`pdf_scan` semantic page
ranking, `pdf_extract` structured sweeps, `pdf_map` per-page summaries)
need an in-kernel model-call bridge wisp doesn't provide; they were
removed rather than left to NameError. For an exhaustive sweep, dump all
pages to files (recipe above, chunked) and work through them — or
delegate the reading to the `explore` subagent once the text is on disk.
