---
name: remote-compute-ssh
description: Submit recoverable SSH-direct research Runs with live progress cards and model-free monitoring.
license: Apache-2.0
---

# Remote compute over SSH

Use this skill after choosing an `ssh:<alias>` execution context. Wisp owns the
job lifecycle locally: `run_in_context` creates the Run record, stages explicit
inputs with persisted byte progress, and starts a detached supervisor on the server.
The Runs panel and SQLite record remain authoritative if the conversation ends
or Wisp restarts.

## Dispatch workflow

1. Use short `run_in_context` calls for read-only discovery such as
   `nvidia-smi -L`, `which python3`, or `module avail`. Free-form shell SSH is
   disabled.
2. Put the real command in one `run_in_context` call. Include environment
   activation in the command so the Run is reproducible.
3. To watch the Run or wait for later work, call `monitor_run` exactly once with
   the returned Run id. Wisp inserts a live card in the conversation, suspends
   the tool without additional model calls, and resumes the same agent turn
   with the terminal result. For fire-and-forget work, report the Run id and end
   the turn instead.
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

Pass that object to `monitor_run` once. The call may remain suspended for hours;
it does not consume model tokens while the Run Manager watches the job.

`input_paths` are project-relative local files. Wisp validates them, copies
them into an isolated `inputs/` directory, and flattens them to their basenames.
The command starts in that directory, so the example above can use the staged
script by basename. Upload progress, throughput, and ETA appear in the Run card. For a large dataset already on
the server, reference its absolute remote path in `command`; do not copy it
back to the laptop just to send it out again.

The control directory is `~/.wisp-science/runs/<run-id>` and the command starts
in its `inputs/` subdirectory. stdout and stderr are
tailed into the Run record. The SSH supervisor requires `setsid`, GNU-compatible
`timeout`, `bash`, and `/proc`; a missing prerequisite fails the Run instead of
running without a wall-time limit. Wisp maps the supervisor timeout marker to
`timed_out`.

## Results

SSH-direct v1 does not expand remote output globs or automatically download a
remote directory. Do not promise that relative `output_specs` will be
harvested. When the command writes a result to a known durable server path, it
may register that exact path as a remote Artifact reference:

```json
{
  "output_specs": [
    {
      "glob": "ssh://gpu-box/home/me/project/results/motif_enrichment_all.tsv",
      "kind": "table",
      "residency": "remote"
    }
  ]
}
```

For a small result that must become local, wait until the Run is terminal,
then transfer it as a separate quick operation and register the local file.
Large outputs should remain remote references.

## Transfers between SSH contexts

Use `transfer_between_contexts` for one exact remote file or directory. Never
compose nested `ssh`, `scp`, or `rsync -e ssh` inside `run_in_context`.

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

Scheduler lifecycle is not implemented yet. Do not submit `sbatch`, `qsub`, or
`bsub` through this direct runner: the Run would only track the short submit
command, not the scheduler job. On a shared login node, ask the user for a
dedicated compute host or explain that scheduler-aware submit/poll/cancel is a
separate capability still needed.
