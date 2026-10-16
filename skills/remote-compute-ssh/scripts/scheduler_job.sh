#!/usr/bin/env bash
# Submit one batch job to the cluster scheduler and block until it ends, so an
# SSH-direct Wisp Run tracks the scheduler job rather than the short submit
# command.
#
# Usage: scheduler_job.sh [--scheduler slurm|pbs|lsf|sge] [--follow FILE] JOB_SCRIPT
#
# - Directive lines (#SBATCH, #PBS, #BSUB, #$) at the top of JOB_SCRIPT are
#   kept. The job runs JOB_SCRIPT from the current directory on every scheduler.
# - The job writes its own exit code to a marker file next to the submission, so
#   this script exits with the job's code even where the blocking client
#   (notably PBS `qsub -W block=true`) does not report it.
# - TERM, INT, or HUP (Wisp cancel_run or the Run timeout) cancels the
#   scheduler job before exiting.
# - --follow streams FILE (the job's -o log) to stdout while the job runs.
#
# Exit status: the job's exit code; 64 usage; 69 no supported scheduler;
# 73 cannot write the submission directory; 143 cancelled before the job ended.

set -u

note() { printf 'wisp-scheduler: %s\n' "$*" >&2; }
usage() {
  printf 'usage: %s [--scheduler slurm|pbs|lsf|sge] [--follow FILE] JOB_SCRIPT\n' "${0##*/}" >&2
  exit 64
}
quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
is_pbs_qsub() { qsub --version 2>&1 | grep -qi 'pbs_version'; }
is_sge() { [ -n "${SGE_ROOT:-}" ] || command -v qconf >/dev/null 2>&1; }

scheduler=""
follow=""
while [ $# -gt 0 ]; do
  case $1 in
    --scheduler) [ $# -ge 2 ] || usage; scheduler=$2; shift 2 ;;
    --follow) [ $# -ge 2 ] || usage; follow=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ $# -eq 1 ] || usage
[ -f "$1" ] || { note "job script not found: $1"; exit 64; }

if [ -z "$scheduler" ]; then
  if command -v sbatch >/dev/null 2>&1; then
    scheduler=slurm
  elif command -v bsub >/dev/null 2>&1; then
    scheduler=lsf
  elif command -v qsub >/dev/null 2>&1; then
    if is_pbs_qsub; then
      scheduler=pbs
    elif is_sge; then
      scheduler=sge
    else
      note 'qsub is neither PBS Professional/OpenPBS nor Grid Engine; Torque cannot wait for a job to end, so nothing was submitted'
      exit 69
    fi
  else
    note 'no supported scheduler found (sbatch, PBS Professional/OpenPBS qsub, bsub, or Grid Engine qsub)'
    exit 69
  fi
fi
case $scheduler in
  slurm) client=sbatch ;;
  lsf) client=bsub ;;
  pbs | sge) client=qsub ;;
  *) usage ;;
esac
command -v "$client" >/dev/null 2>&1 || { note "$client is not on PATH for scheduler $scheduler"; exit 69; }
if [ "$scheduler" = pbs ] && ! is_pbs_qsub; then
  note 'qsub is not PBS Professional/OpenPBS; Torque cannot wait for a job to end, so nothing was submitted'
  exit 69
fi

workdir=$(pwd -P)
job_dir=$(cd "$(dirname "$1")" && pwd -P) || exit 64
job="$job_dir/${1##*/}"
chmod u+x "$job" 2>/dev/null || true
state="$workdir/.wisp-scheduler/$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$state" || { note "cannot create $state"; exit 73; }
submit="$state/job.sh"
client_log="$state/client.log"

# The submitted file: the job's own directives, then a body that runs the job
# from the submission directory and records its exit code. Arrays write one
# marker per task.
directives=$(awk 'NR == 1 && /^#!/ { next }
  /^#(SBATCH|PBS|BSUB|\$)/ { print; next }
  /^[ \t]*(#|$)/ { next }
  { exit }' "$job")
{
  printf '#!/bin/bash\n'
  if [ "$scheduler" = sge ]; then
    printf '#$ -S /bin/bash\n#$ -cwd\n'
  fi
  if [ "$scheduler" = lsf ] && ! printf '%s\n' "$directives" | grep -Eq '^#BSUB[[:space:]]+-oo?([[:space:]]|$)'; then
    # Without -o, LSF mails the job output instead of writing a file.
    printf '#BSUB -o lsf-%%J.out\n'
  fi
  [ -n "$directives" ] && printf '%s\n' "$directives"
  printf 'cd %s || exit 125\n' "$(quote "$workdir")"
  printf 'wisp_state=%s\n' "$(quote "$state")"
  printf '%s\n' "$(quote "$job")"
  cat <<'EOF'
rc=$?
task=${SLURM_ARRAY_TASK_ID:-${PBS_ARRAY_INDEX:-${LSB_JOBINDEX:-${SGE_TASK_ID:-}}}}
case $task in '' | 0 | undefined) marker=exit ;; *) marker=exit.$task ;; esac
tmp="$wisp_state/$marker.tmp.$$"
printf '%s\n' "$rc" > "$tmp" && mv -f "$tmp" "$wisp_state/$marker"
exit "$rc"
EOF
} > "$submit" || { note "cannot write $submit"; exit 73; }

case $scheduler in
  slurm) set -- sbatch --wait --parsable "$submit" ;;
  pbs) set -- qsub -W block=true "$submit" ;;
  lsf) set -- bsub -K ;;
  sge) set -- qsub -terse -sync y "$submit" ;;
esac
# Line-buffer the client so its job id reaches the log before it blocks.
if command -v stdbuf >/dev/null 2>&1; then
  set -- stdbuf -oL -eL "$@"
fi

job_id() {
  case $scheduler in
    slurm) sed -nE '/^[0-9]+(;[^[:space:]]+)?$/{p;q;}' "$client_log" ;;
    pbs) sed -nE '/^[0-9]+(\[[0-9]*\])?(\.[^[:space:]]+)?$/{p;q;}' "$client_log" ;;
    lsf) sed -nE '/^Job <[0-9]+>/{s/^Job <([0-9]+)>.*/\1/;p;q;}' "$client_log" ;;
    sge) sed -nE '/^[0-9]+(\.[0-9]+-[0-9]+:[0-9]+)?$/{s/\..*//;p;q;}' "$client_log" ;;
  esac
}

cancel_job() {
  local id cluster
  id=$(job_id)
  if [ -z "$id" ]; then
    sleep 1
    id=$(job_id)
  fi
  if [ -z "$id" ]; then
    note "cancel requested before the $scheduler job id was known; check the queue for a job submitted from $workdir"
    return
  fi
  note "cancelling $scheduler job $id"
  case $scheduler in
    slurm)
      cluster=${id#*;}
      id=${id%%;*}
      if [ "$cluster" != "$id" ] && [ -n "$cluster" ]; then
        scancel -M "$cluster" "$id" >&2
      else
        scancel "$id" >&2
      fi
      ;;
    pbs | sge) qdel "$id" >&2 ;;
    lsf) bkill "$id" >&2 ;;
  esac
}

follow_pid=""
stop_follow() {
  [ -n "$follow_pid" ] || return 0
  # tail polls about once a second; give it time to print the last lines.
  [ "${1:-}" = drain ] && sleep 2
  kill "$follow_pid" 2>/dev/null
  wait "$follow_pid" 2>/dev/null
  follow_pid=""
}

client_pid=""
client_done=""
stopping=""
on_signal() {
  [ -n "$stopping" ] && return
  stopping=1
  if [ -n "$client_pid" ] && [ -z "$client_done" ]; then
    cancel_job
  fi
  stop_follow
  exit 143
}
trap on_signal TERM INT HUP

if [ -n "$follow" ]; then
  : >> "$follow" 2>/dev/null || true
  tail -n +1 -F "$follow" 2>/dev/null &
  follow_pid=$!
fi

if [ "$scheduler" = lsf ]; then
  "$@" < "$submit" > "$client_log" 2>&1 &
else
  "$@" > "$client_log" 2>&1 &
fi
client_pid=$!

id=""
while [ -z "$id" ] && kill -0 "$client_pid" 2>/dev/null; do
  id=$(job_id)
  [ -n "$id" ] || sleep 1
done
[ -n "$id" ] && note "submitted $scheduler job $id; waiting for it to end"
wait "$client_pid"
client_rc=$?
client_done=1

if [ -z "$id" ]; then
  id=$(job_id)
  if [ -n "$id" ]; then
    note "submitted $scheduler job $id"
  elif [ "$client_rc" -ne 0 ]; then
    stop_follow
    note "$client failed before reporting a job id"
    cat "$client_log" >&2
    exit "$client_rc"
  else
    id=unknown
    note "could not read a job id from the $client output below"
  fi
fi

# Only PBS needs the marker to learn the exit code, so only PBS waits for it.
# It is written on a compute node, and a shared filesystem can take a moment
# to show it here.
settle=0
[ "$scheduler" = pbs ] && settle=${WISP_SCHEDULER_SETTLE_SECS:-60}
found=""
worst=0
waited=0
while :; do
  for marker in "$state"/exit "$state"/exit.*; do
    [ -f "$marker" ] || continue
    case $marker in *.tmp.*) continue ;; esac
    code=$(head -n 1 "$marker" | tr -cd '0-9')
    [ -n "$code" ] || continue
    found=1
    [ "$code" -gt "$worst" ] && worst=$code
  done
  [ -n "$found" ] && break
  [ "$waited" -ge "$settle" ] && break
  sleep 1
  waited=$((waited + 1))
done

stop_follow drain
cat "$client_log" >&2
if [ -n "$found" ]; then
  rc=$worst
  [ "$client_rc" -gt "$rc" ] && rc=$client_rc
elif [ "$client_rc" -ne 0 ]; then
  rc=$client_rc
elif [ "$scheduler" = pbs ]; then
  # qsub -W block=true can return 0 for a job the scheduler killed.
  note "$scheduler job $id ended without finishing its script (wall time, memory, or node failure?)"
  rc=1
else
  rc=0
fi
note "$scheduler job $id ended with exit code $rc"
exit "$rc"
