# Wisp Science Advanced: Batch-Process Papers with map_items

You have downloaded 100 PDFs and need to decide, paper by paper, whether each is a randomized controlled trial, how large the sample is, and whether it meets your inclusion criteria. Ask the agent to read them one by one in a single conversation and the context fills up after twenty or so. Split the job into 100 conversations and there is nothing to summarize.

Work where the same task repeats N times belongs to `map_items`: one instruction, N items, each item handled in its own short-lived context, and the results written to a table. The main conversation receives only counts and short lists, never the full texts.

This tutorial walks through screening 100 papers: how to get the agent to call the tool, what to check at approval, where the results go, and how to resume a run or screen a second time.

> File names, run ids, numbers, and replies in this article are illustrative. They show the interaction and are not the results of a screening that was run.

**First, one thing to know: `map_items` is a tool the agent calls, not a command you type.**

You do not write JSON by hand. Describe the batch task in plain language in a project conversation, and the agent builds the arguments and makes the call. Your part is to state the task clearly and confirm once at approval. The JSON below is what the agent sends. It is shown so that you can read the approval prompt and the tool card, and so that you can say "use map_items" when the agent picks a different tool.

It has a different job from the other ways of handing work off:

| Situation | Use |
| --- | --- |
| The same task repeated many times: screening 100 papers, extracting the same table from 50 PDFs | `map_items` |
| A few different tasks, at most 8 | `delegate_tasks`, or an [Agent Workflow](wisp-science-agent-workflow.md) |
| One open-ended investigation | `explore` |

**Prepare: put the items in the project, and add a TypeSafe key if you need one.**

- Put the files under the project directory, for example in `papers/`. PDFs, Office documents, EPUB, and plain text are converted to text on your machine, which spends no tokens.
- A scanned PDF has no extractable text, and OCR is not supported yet. Such a file fails only its own row.
- One run takes at most 1000 items. More than that is an error, never a silent cut.
- Extraction alone needs no extra setup. To have each row labelled pass, reject, or uncertain, enter a TypeSafe (Jev) API key under Settings → Credentials, or set the `TYPESAFE_API_KEY` environment variable for the CLI. Without a key, the decision step is not offered to the agent at all.

**Step 1: say what to do with one item.**

Send this in a project conversation:

> Use map_items on every PDF in `papers/`. For each paper, extract the study design (rct, cohort, case-control, or other), the sample size, and the primary outcome. Extract only, with no decision yet.

A good batch instruction is about **one item**, is self-contained because the model handling an item cannot see your earlier conversation, and has fixed fields. The agent turns it into a call like this:

```json
{
  "items": { "glob": "papers/*.pdf" },
  "worker": {
    "instruction": "Read this paper. Report its study design, sample size and primary outcome.",
    "output_schema": {
      "type": "object",
      "required": ["design", "sample_size", "primary_outcome"],
      "properties": {
        "design": { "enum": ["rct", "cohort", "case-control", "other"] },
        "sample_size": { "type": "integer" },
        "primary_outcome": { "type": "string" }
      }
    }
  }
}
```

`worker` is one extraction per item, and `output_schema` defines what a result row looks like. If the reply for one paper does not match it, that row is recorded as failed with its reason and never reaches the summary step.

**Step 2: read the approval prompt and confirm once.**

Wisp asks once before any tokens are spent (it does not ask when the project's tool approval mode is "Full bypass"). The prompt looks roughly like this:

```text
map_items will process 100 item(s) — 100 item(s) from glob 'papers/*.pdf'.
Worker: model your-model-id — up to ~3150k input / 100k output tokens; stops and pauses at 3250k total.
Concurrency 4; pauses after 20 min per call (resume continues). Results: .wisp/map-runs/20261006-081530-a1b2c3
```

Check four things: the item count, the model, the token ceiling, and where the results go. The ceiling assumes every paper fills all 60000 characters, so it is a conservative upper bound and not the expected spend. When the ceiling is reached the run pauses. It does not truncate.

The item list is frozen at this point. Files added to `papers/` during the run are not picked up by it.

**Step 3: read the results.**

When the run ends, the agent receives a short report and tells you what it says:

```text
[map_items run 20261006-081530-a1b2c3: complete — 97 ok, 3 failed, 0 not yet processed, of 100]
failed (3): papers/scan_017.pdf (no extractable text (a scanned PDF? OCR is not supported)), ...
tokens (worker/reduce): 812k in / 21k out; 412s elapsed
rows: .wisp/map-runs/20261006-081530-a1b2c3/rows.jsonl
table: .wisp/map-runs/20261006-081530-a1b2c3/results.csv
continue: map_items {"resume": "20261006-081530-a1b2c3"} (retries the failed items)
```

The full results are saved in the project under `.wisp/map-runs/<run id>/`:

| File | Contents |
| --- | --- |
| `manifest.json` | The frozen item list and arguments, and the run status |
| `rows.jsonl` | Records appended one at a time; the last line for an item wins |
| `results.csv` | One row per item: item, status, verdict, extracted fields, the probability for each question, and the error |
| `reduce.md` | The summary, written only when you asked for one |

`results.csv` opens directly in the project file preview.

**Step 4: when you are screening, give the criteria along with the task.**

Extraction answers "what does this paper say". The decision answers "does it meet the requirement". With a TypeSafe key configured, state the criteria in the same request:

> Use map_items to screen every PDF in `papers/`. Inclusion criteria: a randomized controlled trial with a sample size of at least 100. For each paper, extract the study design and sample size, then decide whether to include it.

The call gains a `decide` block:

```json
"decide": {
  "questions": {
    "is_rct": { "type": "noul", "instructions": "Is `design` a randomized controlled trial?" },
    "large_enough": { "type": "noul", "instructions": "Is `sample_size` at least 100?" }
  }
}
```

`noul` is a yes/no question, and backticks in the question refer to fields extracted in the previous step. Phrase every question so that **yes means the item meets the requirement**. Jev returns one probability per question, and the rule that turns them into a verdict is fixed:

| Verdict | Condition |
| --- | --- |
| `pass` | Every yes/no probability is at least `pass_at`, 0.8 by default |
| `reject` | Any yes/no probability is at most `reject_at`, 0.2 by default |
| `uncertain` | Everything else, including a missing probability |

Verdicts are labels. No file is deleted or moved. Two more things to know:

- **Data leaves your machine.** With a decision step, the extracted row for each item is sent to TypeSafe. Without an extraction step, the start of the item's text is sent instead. The approval prompt says so.
- **Extraction plus decision is the recommended setup.** Extraction reads the full text and collects the facts, and the decision judges that one compact row. A decision step on its own suits a cheap first pass over titles and abstracts.

**Step 5: handle uncertain items and items that did not finish.**

Each of these takes one sentence, and the agent adds the matching argument:

| What you want | What to say | Key argument in the call |
| --- | --- | --- |
| The run was stopped, reached its time or token ceiling, or had failed items | "Continue that map_items run" | `{"resume": "<run id>"}` |
| The thresholds feel too strict or too loose | "Change the pass threshold to 0.7 and relabel" | `resume` plus `decide.pass_at` |
| Review only the uncertain ones | "Run the uncertain papers again, and this time allow reading the supplementary files in the project" | `"items": {"run": "<run id>", "verdict": ["uncertain"]}` |

A resume skips finished items and retries failed ones. If only the decision step failed for an item, the extraction you already paid for is reused and only Jev is asked again. Relabelling after a threshold change needs no model call, because each row stores the raw probabilities.

A resume can change only concurrency, limits, and thresholds. To change the items, the extraction instruction, or the questions, start a new run. `items.run` feeds the rows of the earlier run in as the new input.

**Add a summary, or use another item source, when you need to.**

For a written summary on top of the table, ask for it directly: "then summarize which outcomes the included studies used." The call gains a `reduce` block. It runs after every item is done and reads only the structured rows, never the full texts. When there is a decision step, it summarizes pass and uncertain rows by default.

Items do not have to be files in a folder:

| Source | Form | Notes |
| --- | --- | --- |
| Matching files | `{"glob": "papers/*.pdf"}` | Relative to the project root |
| Named files | `{"paths": ["a.pdf", "b.docx"]}` | Listed one by one |
| A JSONL file | `{"jsonl": "records.jsonl", "id_field": "pmid", "where": {"year": 2024}}` | One item per line; `where` filters by field equality |
| An earlier run | `{"run": "<run id>", "verdict": ["uncertain"]}` | A second pass |

A call takes exactly one of the four.

**Using it from the CLI.**

Interactive mode works the same way as the desktop, and the approval appears in the terminal as a `[y/n]` question:

```bash
export TYPESAFE_API_KEY="your-typesafe-key"   # only when you need the decision step
wisp-science
```

`wisp-science run --output jsonl` cannot answer an approval, so the request is refused automatically and a batch run should not be started in that output mode. For the model environment variables, see [Wisp CLI](wisp-science-cli.md).

**When a run does not go as expected, start with this table.**

| Symptom | Check first |
| --- | --- |
| The agent reads the files one by one and never calls the tool | Name it in your message: "use map_items". Confirm the task really is one thing repeated many times |
| The call has no decision step | Whether a TypeSafe key is configured; for the CLI, whether `TYPESAFE_API_KEY` is set |
| The item count is over the limit | One run takes at most 1000. Narrow the pattern or split the work into several runs |
| Some PDFs fail with no extractable text | Usually scanned copies. Run OCR yourself first, into text or a PDF with a text layer |
| Many rows fail because the reply does not match the structure | Whether the instruction defines each field and its allowed values; whether the structure is too strict |
| The run reports paused | It reached the time or token ceiling, or was stopped by hand. Say "continue" to resume |
| The run ends quickly as failed | Five service errors in a row trip a circuit breaker. Check the key, endpoint, and network, then resume |
| The middle of a long paper was not read | One item sends 60000 characters by default, keeping the start and the end. Ask for a higher `max_chars` |

For a first attempt, try 5 to 10 papers: confirm that the fields come out right and that no decision question is phrased the wrong way round, then run it on everything.

> All arguments and implementation details are in the [map_items reference](../../map-items.md). For how it relates to the other ways of delegating, see [Agent delegation](../../agent-delegation.md). This article reflects the project implementation at the time of writing, and prompt wording may differ slightly between versions.
