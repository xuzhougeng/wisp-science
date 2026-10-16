---
name: remote-compute-ssh
description: Submit recoverable SSH-direct research Runs with live progress cards and model-free monitoring, including Slurm, PBS, LSF, and Grid Engine batch jobs from a cluster login node.
license: Apache-2.0
---

# Remote compute over SSH

Use this skill after choosing an `ssh:<alias>` execution context. Wisp owns the
job lifecycle locally: `run_in_context` creates the Run record, stages explicit
inputs with persisted byte progress, and starts a detached supervisor on the server.
The Runs panel and SQLite record remain authoritative if the conversation ends
or Wisp restarts.

When the context reports a `scheduler` or the server is a cluster login node,
submit heavy work through the scheduler wrapper described in
[Cluster scheduler jobs](#cluster-scheduler-jobs), not directly on the login
node.

## Dispatch workflow

1. Use short `run_in_context` calls for read-only discovery such as
   `nvidia-smi -L`, `which python3`, or `module avail`. Free-form shell SSH is
   disabled. Use the interpreter path reported by the context probe; do not
   assume a `python` alias exists when the probe found `python3`.
2. Put the real command in one `run_in_context` call. Include environment
   activation in the command so the Run is reproducible.
3. To watch the Run or wait for later work, call `monitor_run` with the
   returned Run id. Wisp inserts a live card in the conversation, suspends
   the tool without additional model calls, and resumes the same agent turn
   with the terminal result. If the result has `wait_interrupted: true`, the
   remote process is still running: answer the user from the snapshot, then
   call `monitor_run` again with the same id. Do not resubmit. Use `cancel_run`
   only when the user asked to stop. For fire-and-forget work, report the Run id
   and end the turn instead.
4. Use `get_run` only for one explicit status snapshot; never call it repeatedly
   to wait. Use `cancel_run` when the user asks to stop.

Never monitor a Run with `Start-Sleep`, `sleep`, `ssh ... ps`, `kill -0`, a
shell polling loop, `nohup`, background `&`, or hand-written PID files. Those
duplicate the control plane and can strand the agent turn. A transient SSH
error is stored as `last_poll_error`; do not resubmit, because Wisp retries the
same idempotent remote handle.

```json
{
  "context_id": "ssh:gpu-box",
  "title": "Motif enrichment across 2,000 backgrounds",
  "command": "source ~/miniforge3/etc/profile.d/conda.sh && conda activate genomics && python motif_enrichment_analysis.py",
  "timeout_secs": 14400,
  "input_paths": ["scripts/motif_enrichment_analysis.py"]
}
```

Then, when live monitoring is needed:

```json
{ "run_id": "<id returned by run_in_context>" }
```

Pass that object to `monitor_run`. The call may remain suspended for hours;
it does not consume model tokens while the Run Manager watches the job.
If `wait_interrupted` is true, respond from the snapshot and call `monitor_run`
again with the same id; do not resubmit.

`input_paths` are project-relative local files. Wisp validates them, copies
them into an isolated `inputs/` directory, and flattens them to their basenames.
The command starts in that directory, so the example above can use the staged
script by basename. Upload progress, throughput, and ETA appear in the Run card. For a large dataset already on
the server, reference its absolute remote path in `command`; do not copy it
back to the laptop just to send it out again.

A remote command or application exiting non-zero is normal exploration after a
successful login. Read stderr, correct the command from the probed
capabilities, and continue. Stop only when SSH rejects authentication or host
trust; do not repeat a rejected login with guessed credentials or SSH options.

The control directory is `~/.wisp-science/runs/<run-id>` and the command starts
in its `inputs/` subdirectory. stdout and stderr are
tailed into the Run record. The SSH supervisor requires `setsid`, GNU-compatible
`timeout`, `bash`, and `/proc`; a missing prerequisite fails the Run instead of
running without a wall-time limit. Wisp maps the supervisor timeout marker to
`timed_out`.

## Results

Declare `output_specs` with workdir-relative globs for the final products.
After the Run succeeds, Wisp collects the matches on the server, checksums
them, pulls them back through a persisted transfer Run, places them under the
project's configured results directory, and registers each as an
ArtifactVersion. The Run records `harvested_at` once registration completes;
`harvest_run({"run_id":"..."})` retries a failed or interrupted harvest.

Selection is the database boundary: only spec-matched outputs are transferred
and recorded. Point globs at final products (for example `Trinity.fasta`),
never at intermediate trees. A non-bundle glob may match at most 500 files.
For a many-file output that must be kept, set `bundle: true` so the matches
(or a whole directory) arrive as one tar.gz archive registered as a single
artifact:

```json
{
  "output_specs": [
    { "glob": "results/*.tsv", "kind": "table", "residency": "auto" },
    { "glob": "assembly_out", "kind": "archive", "residency": "local", "bundle": true }
  ]
}
```

Files over the size caps (or `residency: "remote"`) are moved out of the run
workdir into the project's persistent remote data area, registered as
`ssh://` references with checksum and size, and ledgered so they stay visible
in `list_remote_files`. Workspace cleanup never orphans them. Delete a
ledgered persist file with `remove_remote_files` only after the user confirms
they no longer need it — that marks the artifact's source discarded. Explicit
`ssh://…` URIs in `output_specs` still register a remote reference without
any download.

## Cluster scheduler jobs

On a cluster, the Run's command is the bundled wrapper
`scripts/scheduler_job.sh` (in this skill's directory). It submits one batch
job, blocks until the job ends, and exits with the job's exit code. So the Run
stays `running` while the job is queued or running, and it finishes with the
job's real result. `cancel_run` and the Run timeout both send TERM, and the
wrapper then cancels the job with `scancel`, `qdel`, or `bkill`.

1. Write the job script in the project, for example `jobs/align.sbatch`.
   - Put scheduler directives at the top: `#SBATCH`, `#PBS`, `#BSUB`, or `#$`.
   - Write the job log to a known file: `#SBATCH -o job.out`,
     `#PBS -j oe -o job.out`, `#BSUB -o job.out`, or `#$ -j y -o job.out`.
   - Put environment setup (`module load`, conda activation) inside the job
     script.
   - Take the partition or queue, account, and resources from the user or the
     server notes. Never guess an account or allocation.
2. Copy `scripts/scheduler_job.sh` from this skill's directory into the
   project, for example as `scripts/scheduler_job.sh`. The Run's staged inputs
   then record exactly what was submitted.
3. Submit both files in one `run_in_context` call and watch it with
   `monitor_run`, the same as any other Run:

```json
{
  "context_id": "ssh:hpc-login",
  "title": "STAR alignment for 24 samples",
  "command": "bash scheduler_job.sh --follow job.out align.sbatch",
  "timeout_secs": 259200,
  "input_paths": ["scripts/scheduler_job.sh", "jobs/align.sbatch"],
  "output_specs": [
    { "glob": "job.out", "kind": "log", "residency": "local" },
    { "glob": "counts/*.tsv", "kind": "table", "residency": "auto" }
  ]
}
```

- **Time limit.** `timeout_secs` covers queue wait plus run time, so set it
  well above the job's own wall-time directive. When the Run times out, the
  wrapper cancels the job.
- **Working directory.** The job runs in the Run's `inputs/` directory on every
  scheduler, because the wrapper changes to it. Relative paths and
  `output_specs` globs therefore behave as in any SSH-direct Run. This assumes
  the home directory is shared with the compute nodes, which is normal on
  clusters. If outputs never appear, ask the user.
- **Scheduler detection.** The wrapper detects the scheduler itself:
  - `sbatch` means Slurm.
  - `bsub` means LSF.
  - `qsub` means PBS Professional/OpenPBS or Grid Engine.

  The probe reports every `qsub` as `pbs`, so do not pass `--scheduler` from
  the probe. Pass it only when the user names the scheduler. Torque's `qsub`
  cannot wait for a job to end, so the wrapper refuses it without submitting.
  Tell the user instead of submitting by hand.
- **Exit codes.** The wrapper exits with the job's exit code, with these
  exceptions:
  - 64: bad usage or a missing job script.
  - 69: no supported scheduler.
  - 143: the job was cancelled.
  - 1: a PBS job was killed by the scheduler (wall time, memory) before its
    script finished.

  The wrapper's stderr names the job id and the scheduler's own messages.
- **One job per Run.** Array jobs are fine: the Run waits for every task and
  reports the highest task exit code. Jobs submitted from inside the job script
  (pipelines, dependencies) are neither tracked nor cancelled by the Run.
- **No direct submission.** Never put a bare `sbatch`, `qsub`, or `bsub` in
  `run_in_context`, and never write your own submit-and-poll loop. Either way
  the Run would track only the submit command. The wrapper is the one
  exception to the no-polling rule above, because it runs inside the Run and
  ends with it.
- **Multiple login nodes.** Some clusters put several login nodes behind one
  SSH name. Wisp checks the wrapper process on whichever node a poll reaches,
  so a poll that lands on another login node marks the Run `lost` while the job
  keeps running. Sites that kill long-lived login-node processes cause the same
  result. When a scheduler Run is `lost`, tell the user the job may still be
  queued or running and give them the job id from the Run's stderr. If it
  recurs, ask them to point the SSH alias at one specific login node.

## Cleanup: servers are disposable

Tasks and artifacts belong to the project; the server only computes. After the
results are harvested (or knowingly abandoned), reclaim the workspace:

- `cleanup_run_workspace({"run_id":"..."})` deletes the Run's
  `~/.wisp-science/runs/<run-id>` directory (inputs, logs, intermediates). A
  succeeded Run with declared `output_specs` must be harvested first; the tool
  refuses otherwise so results are never lost. Registered artifacts stay in the
  project. Before deletion Wisp pulls a trailing slice of stdout/stderr
  (at most 4 MiB per stream) into `runs/<id>/` — not the complete remote logs.
- `list_remote_files({"context_id":"ssh:<alias>"})` shows every file this
  project placed on the server (staged inputs, uploads, and harvest-persisted
  outputs) classified as active, replaced, or orphan; `remove_remote_files`
  deletes retracted ones. Current successful uploads stay active — they are
  the user's dataset, not sweep fodder. Replaced rows are closed in the ledger
  only (they share a path with the current file). Harvest-persisted outputs
  stay active while a live External artifact still points at them. Uploads are
  ledgered when the transfer attempt starts, so a failed or cancelled partial
  is visible and can be removed.
- Removing the SSH host from Settings audits remaining references and
  ledgered files, then marks those External artifacts as source-discarded.
  Later download, preview, or transfer of those URIs is refused even if the
  same alias is re-registered.
- Project settings can enable retention windows that automatically clean
  succeeded+harvested and failed run workspaces after N days.

Intermediate files (for example Trinity's hundreds of thousands of read
partitions) should never be enumerated, downloaded, or registered — leave them
in the workdir and let cleanup reclaim them in one deletion.

## Transfers between local and SSH contexts

Use `transfer_between_contexts` for one exact remote file or directory. The
destination may be another selected SSH context or `local`. Never compose
nested `ssh`, `scp`, or `rsync -e ssh` inside `run_in_context`.

Users can also upload from the Files panel: select the SSH context, open the
destination folder, then use **Upload** or drop local files. That UI path
submits the same `file_transfer` Run and does not require this tool.

For a local upload via the agent, set `source_context_id` to `local`, provide
the exact existing absolute local file or directory, and select an SSH
destination.
Omit `destination_path` to place the file under the project's configured
remote data directory for that server. Wisp rejects globs, symlinks, special
files, and existing remote destinations, and ledgers every successful upload
so retracted files can be found and removed later. Call `monitor_run`
with the returned Run id; call it again after `wait_interrupted`.

For a local download, set `destination_context_id` to `local` and provide the
exact new absolute local path. Ask the user when that path is unspecified.
Wisp stages the item beside the destination, never overwrites an existing
path, and removes partial staging data after failure or cancellation. Call
`monitor_run` with the returned Run id; call it again after `wait_interrupted`.

When the user approves persistent A→B trust, call `configure_ssh_trust` first.
It creates a dedicated key on A, carries only the public key through Wisp,
installs it on B, and verifies the directed edge. The transfer then prefers
rsync when both servers provide it and falls back to scp. If the user does not
want server SSH configuration changed, select the relay route; Wisp downloads
to a private local temporary directory and uploads with B's separately stored
credentials.

## Cancellation and recovery

`cancel_run({"run_id":"..."})` changes an SSH Run to `cancelling`. Wisp
verifies the persisted token, PGID, and Linux process start time before sending
TERM to the remote process group; it records `cancelled` only after remote
confirmation. If the server is temporarily unreachable, the Run stays
`cancelling` and retry continues after reconnection or app restart.

Active statuses are `submitted`, `running`, and `cancelling`. Terminal statuses
are `succeeded`, `failed`, `timed_out`, `cancelled`, and `lost`. `lost` means
the remote token/control directory/process identity was definitively missing,
not merely that one SSH poll failed.

## Current boundary

This implementation is SSH-direct and assumes a Linux-like server with `sh`,
`bash`, `nohup`, `setsid`, and `/proc`. Do not daemonize or create a new session
inside the job, because that escapes process-group cancellation.

Scheduler jobs go through `scripts/scheduler_job.sh` only. It supports Slurm,
PBS Professional/OpenPBS, LSF, and Grid Engine. Torque and queue-template
management are not supported.
