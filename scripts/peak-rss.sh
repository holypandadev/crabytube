#!/usr/bin/env bash
# Usage: scripts/peak-rss.sh ./target/release/crabytube [app args...]
#
# Linux only. Starts the app with an isolated profile (cold caches unless PEAK_PROFILE is set) and
# samples, every 20 ms until it exits, the summed memory of the app and its direct children
# (the helper processes):
#   peak_aggregate_pss_kib  (primary metric: proportional set size, from smaps_rollup)
#   peak_aggregate_rss_kib  (secondary: resident set size, VmRSS)
# The app's stdout and stderr go to a log file, so time-to-audio can be read from it.
set -u
# PEAK_PROFILE=<dir> reuses a profile across runs (warm caches); otherwise the profile is fresh.
T=${PEAK_PROFILE:-$(mktemp -d)}
mkdir -p "$T"
LOG="$T/app.log"
XDG_CONFIG_HOME=$T/c XDG_CACHE_HOME=$T/k XDG_DATA_HOME=$T/d "$@" >"$LOG" 2>&1 &
APP=$!
peak_rss=0
peak_pss=0
while kill -0 $APP 2>/dev/null; do
  rss=0
  pss=0
  for p in $APP $(pgrep -P $APP); do
    r=$(awk '/^VmRSS/{print $2}' /proc/$p/status 2>/dev/null)
    s=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup 2>/dev/null)
    rss=$((rss + ${r:-0}))
    pss=$((pss + ${s:-0}))
  done
  (( rss > peak_rss )) && peak_rss=$rss
  (( pss > peak_pss )) && peak_pss=$pss
  sleep 0.02
done
echo "peak_aggregate_pss_kib=$peak_pss"
echo "peak_aggregate_rss_kib=$peak_rss"
echo "log=$LOG"
