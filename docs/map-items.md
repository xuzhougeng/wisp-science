# Batch map: `map_items`

`map_items` applies **one instruction to many items** — for example "does this
paper meet our inclusion criteria?" over 100 downloaded PDFs — and returns a
table instead of 100 conversations. It is the batch sibling of `explore`
(one open-ended investigation) and `delegate_tasks` (a few *different* tasks,
at most 8); use it when the *same* task repeats over N items.

Each item is handled in its own short-lived model context, so one unreadable PDF
or one bad answer fails only that row and the main conversation receives counts
and short lists, never the full texts. It is a map-reduce: the per-item *map*
step writes a structured row, an optional *reduce* step folds the rows.

## What happens per item

```text
items (frozen when the run starts)
  per item, up to `concurrency` at a time:
    1. extract text on the host (PDF / Office / EPUB / text — no tokens spent)
    2. worker (optional): one model call → one JSON row, checked against output_schema
    3. decide (optional): TypeSafe Jev answers yes/no questions about the row
       → pass / reject / uncertain
    4. append one line to .wisp/map-runs/<run_id>/rows.jsonl
reduce (optional): one model call over the structured rows only
```

Give `worker`, `decide`, or both. The recommended screening setup is both: the
worker reads the full text and extracts facts, Jev judges the compact row.
`decide` alone is a cheap first pass over the start of each item (title and
abstract); `worker` alone is plain extraction and works without a TypeSafe key.

## Arguments

| Argument | Meaning |
| --- | --- |
| `items` | Exactly one of `glob` (`"papers/*.pdf"`), `paths` (list), `jsonl` (a file of objects; optional `id_field`, `where`), or `run` (rows of an earlier run; optional `verdict` filter). At most **1000** items; more is an error, never a silent cut. |
| `worker.instruction` | What to do with *one* item. |
| `worker.output_schema` | JSON Schema of the row. A reply that does not match fails that item and never reaches the reduce. |
| `worker.tools` | Optional read-only subset of `read`, `grep`, `search` for items that need more than their own text. Reads stay inside the project root. Costs extra rounds per item (`worker.max_iterations`, default 8, max 15). |
| `worker.model` | A cheaper model id on the same endpoint for bulk extraction. |
| `decide.questions` | `id → {type: noul \| choice \| score, instructions, criteria?}` (the Jev question format). |
| `decide.pass_at` / `reject_at` | Default `0.8` / `0.2`. |
| `decide.text_chars` | Without a worker, how much of the item's start Jev reads (default 8000). |
| `reduce.instruction` | One model call over the rows (chunked and merged when large). `reduce.verdicts` picks which rows feed it (default pass and uncertain). |
| `concurrency` | Parallel items, default 4, max 16. |
| `limits` | `max_chars` per item sent to the model (default 60000, head and tail kept), `max_minutes` per call (default 20, max 120), `max_tokens` worker ceiling per call (default: the estimated upper bound). |
| `resume`, `retry` | Continue a run (below). |

## The decision step (Jev)

`decide` is offered only when a TypeSafe API key is configured (Settings →
Credentials, or `TYPESAFE_API_KEY` in the CLI). The rule is fixed and
conservative about *removing* anything:

- Phrase every `noul` question so that **yes means the item meets the requirement**.
- **pass** — every noul answer is at least `pass_at`.
- **reject** — any noul answer is at most `reject_at`.
- **uncertain** — everything else, including a missing or NaN probability. These
  are handed back: look at them yourself, or run a second pass with
  `items: {run: "<id>", verdict: ["uncertain"]}` (for example a worker with
  tools). Choice and score answers are recorded but never decide the verdict.

Verdicts are labels. Nothing is deleted or moved. Rows store the raw
probabilities and the Jev version that answered (pinned in the manifest after the
first response, so one run never mixes versions), and the verdict is recomputed
from them: `resume` with new `decide.pass_at` / `reject_at` relabels every row
with no model call.

**Data egress:** with `decide`, each item's extracted row (or, without a worker,
the start of its text) is sent to TypeSafe. The approval prompt says so.

## Control

- **One approval** before anything is spent, showing the item count, the model,
  an upper bound on tokens, the time limit and where results go.
- **Frozen items.** The list is resolved once into `manifest.json`; nothing is
  appended mid-run.
- **Stop pauses.** In-flight items are dropped without being recorded as
  failures; they are simply pending.
- **Ceilings pause, never truncate.** Hitting `max_minutes` or the token ceiling
  stops starting new items and reports the run as paused.
- **Circuit breaker.** Five consecutive service failures (bad key, endpoint
  down) stop the run instead of burning through the list. A bad item — an
  unreadable or scanned PDF, a reply that violates the schema — fails only itself
  and keeps its reason.
- **Retries.** 429/5xx/transport errors are retried with backoff inside the item.
- **Resume.** `{"resume": "<run_id>"}` skips finished items and retries failed
  ones. If only the decision step failed, the paid worker output is reused and
  only Jev is asked again. A resume takes only `retry`, `concurrency`, `limits`
  and `decide.pass_at` / `reject_at`; to change items, worker or questions, start
  a new run (`items.run` takes the rows of the old one).
- **Read-only workers.** Workers have no write, shell or network tools; an
  instruction hidden in a paper can at worst spoil that paper's own row.

## Files

```text
.wisp/map-runs/<run_id>/
  manifest.json   frozen items, frozen spec, status, pinned Jev version
  rows.jsonl      append-only attempts; the last line per item wins
  results.csv     one row per item: item, status, verdict, worker fields, p_<question>, error
  reduce.md       the reduce output, when requested
```

`results.csv` opens in the project file preview. Cells that start with a formula
character are prefixed with `'` so a spreadsheet does not run text taken from a
paper.

## Not in scope

A Runs-panel entry with background execution, a workflow node that wraps a
map-run, item sources backed by Paper or DataAsset records, token metering of the
Jev step, and OCR for scanned PDFs. A single call is bounded by `max_minutes`; a
bigger list simply takes several `resume` calls.

## Tests

`cargo test -p wisp-core map_items` runs everything offline against a scripted
worker provider and a local fake of the Jev endpoint; the `map-items-batch` case
in `crates/wisp-cli/eval-suites/offline-v1.yaml` drives the real tool through the
agent loop.
