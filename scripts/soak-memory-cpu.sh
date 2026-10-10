#!/usr/bin/env bash
# Usage: scripts/soak-memory-cpu.sh <crabytube-binary> [video-id] [sample-seconds]
#
# Linux only; the binary must be built with `--features diagnostics`.
# Runs one `--soak` session in a throwaway profile and samples the processes every
# <sample-seconds> (default 30) until the app exits:
#   pss_kib    summed PSS of the app and its direct children (smaps_rollup), the memory metric
#   rss_kib    summed VmRSS of the same processes
#   processes  how many of them answered
#   cpu_pct    CPU of the app process alone over the interval (one core = 100)
# The phase comes from the app log ("soak phase: <name>"). After the app exits, summary.txt
# has the figures per phase and the goals.
# SOAK_OUT=<dir> keeps the output there (default: a new temporary directory).
# SOAK_SMAPS=1 also writes smaps.csv: the app's PSS by kind of mapping (heap, small and large
# anonymous mappings, files, stacks), to see which memory grows.
# CRABYTUBE_SOAK_PHASE_SECS shortens the phases for a dry run (default 600 s; the pause lasts half).
set -u
BIN=${1:?usage: $0 <crabytube-binary> [video-id] [sample-seconds]}
VIDEO=${2:-dQw4w9WgXcQ}
SAMPLE=${3:-30}
OUT=${SOAK_OUT:-$(mktemp -d)}
mkdir -p "$OUT"
export XDG_CONFIG_HOME=$OUT/config XDG_CACHE_HOME=$OUT/cache XDG_DATA_HOME=$OUT/data XDG_STATE_HOME=$OUT/state
LOG=$OUT/app.log
CSV=$OUT/samples.csv
THREADS=$OUT/threads.csv
SMAPS=$OUT/smaps.csv
HZ=$(getconf CLK_TCK)

# PSS (KiB) of the app's mappings by kind: heap, small and large anonymous, file, stack, other.
smaps_row() {
  awk '
    /^[0-9a-f]+-[0-9a-f]+ / {
      path = ""
      for (k = 6; k <= NF; k++) path = path (k > 6 ? " " : "") $k
      if (path == "") kind = "anon"
      else if (path == "[heap]") kind = "heap"
      else if (path ~ /^\[stack/) kind = "stack"
      else if (path ~ /^\//) kind = "file"
      else kind = "other"
    }
    /^Size:/ { size = $2 }
    /^Pss:/ {
      if (kind == "anon") bucket = (size >= 256 ? "anon_large" : "anon_small"); else bucket = kind
      sum[bucket] += $2
    }
    END {
      printf "%d,%d,%d,%d,%d,%d\n", sum["heap"], sum["anon_small"], sum["anon_large"], sum["file"], sum["stack"], sum["other"]
    }' "/proc/$APP/smaps" 2>/dev/null
}

# Summed PSS and RSS (KiB) of the app and its direct children, and how many answered.
sample_memory() {
  local pid pss=0 rss=0 n=0 s r
  for pid in "$APP" $(pgrep -P "$APP"); do
    s=$(awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null)
    r=$(awk '/^VmRSS:/{print $2}' "/proc/$pid/status" 2>/dev/null)
    [ -n "$s" ] || continue
    pss=$((pss + s))
    rss=$((rss + ${r:-0}))
    n=$((n + 1))
  done
  echo "$pss $rss $n"
}

# utime + stime of the app process, in clock ticks (fields 14 and 15 of /proc/<pid>/stat).
app_ticks() {
  local fields
  fields=$(sed 's/.*) //' "/proc/$APP/stat" 2>/dev/null) || { echo 0; return; }
  set -- $fields
  echo $((${12} + ${13}))
}

# One line per thread of the app: thread id, thread name and cumulative ticks.
thread_rows() {
  local dir fields
  for dir in "/proc/$APP/task/"*; do
    fields=$(sed 's/.*) //' "$dir/stat" 2>/dev/null) || continue
    set -- $fields
    echo "${dir##*/},$(cat "$dir/comm" 2>/dev/null),$((${12} + ${13}))"
  done
}

"$BIN" --soak --autoplay "$VIDEO" >"$LOG" 2>&1 &
APP=$!
start=$(date +%s.%N)
echo "elapsed_s,phase,pss_kib,rss_kib,processes,cpu_pct" >"$CSV"
echo "elapsed_s,phase,tid,comm,ticks" >"$THREADS"
if [ "${SOAK_SMAPS:-0}" = 1 ]; then
  echo "elapsed_s,phase,heap_kib,anon_small_kib,anon_large_kib,file_kib,stack_kib,other_kib" >"$SMAPS"
fi
prev_ticks=0
prev_time=$start
while kill -0 "$APP" 2>/dev/null; do
  sleep "$SAMPLE"
  kill -0 "$APP" 2>/dev/null || break
  now=$(date +%s.%N)
  ticks=$(app_ticks)
  read -r pss rss n <<<"$(sample_memory)"
  phase=$(grep -o 'soak phase: [a-z-]*' "$LOG" | tail -n1 | sed 's/soak phase: //')
  phase=${phase:-none}
  elapsed=$(awk -v now="$now" -v start="$start" 'BEGIN { printf "%.0f", now - start }')
  thread_rows | sed "s/^/$elapsed,$phase,/" >>"$THREADS"
  if [ "${SOAK_SMAPS:-0}" = 1 ]; then
    echo "$elapsed,$phase,$(smaps_row)" >>"$SMAPS"
  fi
  awk -v elapsed="$elapsed" -v ticks="$ticks" -v prev_ticks="$prev_ticks" \
    -v prev_time="$prev_time" -v now="$now" -v hz="$HZ" -v phase="$phase" \
    -v pss="$pss" -v rss="$rss" -v n="$n" \
    'BEGIN { printf "%s,%s,%s,%s,%s,%.2f\n", elapsed, phase, pss, rss, n, (ticks - prev_ticks) / hz / (now - prev_time) * 100 }' >>"$CSV"
  prev_ticks=$ticks
  prev_time=$now
done
wait "$APP"
code=$?
python3 -I - "$CSV" "$code" "$THREADS" "$HZ" >"$OUT/summary.txt" <<'PY'
import csv
import sys

rows = list(csv.DictReader(open(sys.argv[1])))
code = sys.argv[2]
for row in rows:
    row["t"] = float(row["elapsed_s"])
    row["pss"] = int(row["pss_kib"])
    row["cpu"] = float(row["cpu_pct"])


def phase_rows(name):
    # The first sample of a phase covers the time before the switch, and the first sample of
    # the run covers the start-up, so both are left out.
    return [r for i, r in enumerate(rows) if i > 0 and r["phase"] == name and rows[i - 1]["phase"] == name]


def nearest(seconds):
    return min(rows, key=lambda r: abs(r["t"] - seconds)) if rows else None


# Goals on the mean CPU of each phase. Home has no goal of its own.
GOALS = {
    "now-playing-lyrics": ("< 8 %", lambda v: v < 8.0),
    "minimized": ("< 3.5 %", lambda v: v < 3.5),
    "paused": ("< 0.5 %", lambda v: v < 0.5),
}
last = rows[-1]["t"] if rows else 0.0
print(f"app_exit_code={code} samples={len(rows)} last_elapsed_s={last:.0f}")
print("phase                 samples  cpu_mean  cpu_max  pss_first_mib  pss_last_mib  goal    result")
verdicts = []
for name in ["home", "now-playing-lyrics", "minimized", "paused"]:
    selected = phase_rows(name)
    if not selected:
        print(f"{name:<21} 0        not reached")
        if name in GOALS:
            verdicts.append(False)
        continue
    mean = sum(r["cpu"] for r in selected) / len(selected)
    peak = max(r["cpu"] for r in selected)
    first = selected[0]["pss"] / 1024
    final = selected[-1]["pss"] / 1024
    if name in GOALS:
        text, check = GOALS[name]
        ok = check(mean)
        verdicts.append(ok)
        result = "pass" if ok else "FAIL"
    else:
        text, result = "-", "no goal"
    print(f"{name:<21} {len(selected):<8} {mean:<9.2f} {peak:<8.2f} {first:<14.1f} {final:<13.1f} {text:<7} {result}")
start_row = nearest(300)
end_row = nearest(1800)
if start_row and end_row and end_row["t"] >= 1750:
    growth = (end_row["pss"] - start_row["pss"]) / 1024
    window = [r["pss"] for r in rows if 300 <= r["t"] <= 1800]
    spread = (max(window) - min(window)) / 1024
    print(f"pss_growth_min5_to_min30_mib={growth:+.2f} (samples at t={start_row['t']:.0f} s and t={end_row['t']:.0f} s; spread inside the window {spread:.2f} MiB; goal <= 5)")
    verdicts.append(growth <= 5.0)
else:
    print("pss_growth_min5_to_min30_mib=not measured (the run did not reach minute 30)")
    verdicts.append(False)
print("all goals met" if all(verdicts) else "some goals not met")

# CPU by thread name: ticks gained by each thread between two samples, over the wall time of the phase.
hz = float(sys.argv[4])
by_thread = {}
for row in csv.DictReader(open(sys.argv[3])):
    by_thread.setdefault(row["tid"], []).append((float(row["elapsed_s"]), row["phase"], row["comm"], int(row["ticks"])))
share = {}
for samples in by_thread.values():
    samples.sort()
    for (t0, _, _, k0), (t1, phase, comm, k1) in zip(samples, samples[1:]):
        if t1 > t0 and phase != "none":
            cell = share.setdefault(phase, {}).setdefault(comm, 0)
            share[phase][comm] = cell + (k1 - k0)
wall = {}
for previous, row in zip(rows, rows[1:]):
    wall[row["phase"]] = wall.get(row["phase"], 0.0) + (row["t"] - previous["t"])
print("cpu by thread name, percent of one core, top 5 per phase:")
for name in ["home", "now-playing-lyrics", "minimized", "paused"]:
    if name in share and wall.get(name):
        ranked = sorted(share[name].items(), key=lambda item: -item[1])[:5]
        print(f"  {name:<20} " + "  ".join(f"{comm}={ticks / hz / wall[name] * 100:.2f}" for comm, ticks in ranked))
PY
cat "$OUT/summary.txt"
echo "output_dir=$OUT"
