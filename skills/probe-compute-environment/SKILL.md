---
name: probe-compute-environment
description: Inspect a registered execution server before compute planning and interpret its persisted capability profile. Use when a server is added, when the user clicks Probe, before enabling an unfamiliar SSH/WSL resource, or when deciding whether GPU, sudo/root, a scheduler, Python, R, conda, mamba, or environment modules are available.
---

# Probe compute environment

Use Wisp's environment Probe action. It performs bounded, read-only checks and
stores the result on the `ExecutionContext`. An SSH probe batches every check
into one authenticated session; do not replace it with an ad-hoc SSH discovery
loop. Batch SSH uses `IdentitiesOnly=yes`; users relying on a non-default
ssh-agent key must configure its `IdentityFile` in Wisp or SSH config.

After probing:

1. Treat persisted capabilities as the server contract until it is probed again.
2. Treat `gpu_summary: null` as **no usable GPU**. Plan CPU work and never add
   CUDA/GPU flags speculatively.
3. Treat `privilege: unprivileged` as **no root or passwordless sudo**. Do not
   use `sudo`, system package managers, or system paths; prefer user-space
   environments, modules, containers already installed by the administrator,
   or ask the user for an administrator-installed dependency.
4. Use a detected scheduler rather than running long work on a login node:
   submit through the scheduler wrapper in `remote-compute-ssh`. A detected
   `qsub` is reported as `pbs` even when it is Grid Engine; let the wrapper
   identify it.
5. Use the recorded interpreter paths and environment managers. Do not assume
   `python`, `Rscript`, conda, mamba, or modulecmd exists when absent.
6. If the probe failed, inspect the saved error and ask the user to check the
   connection before manually probing again. Never loop or automatically retry
   an SSH probe. If it is merely stale relative to a server change, probe again
   before submitting work.
