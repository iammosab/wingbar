#!/usr/bin/env bash
# Measure what wingbar costs, so "light" is a number and not a claim.
#
#   scripts/measure.sh [seconds]             your real config (default 180 s)
#   scripts/measure.sh --isolated [seconds]  built-in widgets only, no user scripts
#   scripts/measure.sh --compare PID         also sample another process (e.g. Übersicht)
#
# Options: --interval S (whole seconds between samples, default 2), --warmup S
# (left out of the steady-state numbers, default 10), --bin PATH (measure that
# binary instead of building; handy for before/after runs), --no-build, --keep
# (keep the work dir with the raw samples).
#
# It builds the release binary, runs `wingbar --no-server --quit-after N` (no
# port is bound, so an installed bar can keep running) and reports the bar's
# own cost apart from the widget scripts it starts:
#
# - from inside the bar, just before it quits (WINGBAR_MEASURE): its own CPU
#   time, the CPU time of its reaped child processes (the scripts), wakeups,
#   context switches, instructions, peak memory, and how many widget runs and
#   threads the scheduler started;
# - from outside, every few seconds: CPU time (ps), RSS (ps), threads (ps -M),
#   live child processes (pgrep -P), and now and then idle wakeups and context
#   switches (top) and the phys footprint (footprint). --compare samples the
#   other process the same way, over the same window. It is only observed:
#   nothing here starts, stops or signals it.
#
# Only macOS built-ins are used (ps, top, footprint, pgrep, awk). No sudo.

set -euo pipefail

usage() { sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

DURATION=180
INTERVAL=2
WARMUP=10
ISOLATED=0
COMPARE=""
BUILD=1
KEEP=0
BIN_ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --isolated) ISOLATED=1 ;;
    --compare) COMPARE="${2:?--compare needs a pid}"; shift ;;
    --duration) DURATION="${2:?}"; shift ;;
    --interval) INTERVAL="${2:?}"; shift ;;
    --warmup) WARMUP="${2:?}"; shift ;;
    --no-build) BUILD=0 ;;
    --bin) BIN_ARG="${2:?}"; BUILD=0; shift ;;
    --keep) KEEP=1 ;;
    -h|--help) usage 0 ;;
    *[!0-9]*|'') echo "measure.sh: unknown argument: $1" >&2; usage 2 ;;
    *) DURATION="$1" ;;
  esac
  shift
done
case "$INTERVAL$WARMUP" in *[!0-9]*) echo "measure.sh: --interval and --warmup take whole seconds" >&2; exit 2 ;; esac
case "$COMPARE" in *[!0-9]*) echo "measure.sh: --compare needs a pid" >&2; exit 2 ;; esac
if [ "$DURATION" -le "$((WARMUP + 2 * INTERVAL))" ]; then
  echo "measure.sh: run for longer than the warmup ($WARMUP s) plus two samples" >&2
  exit 2
fi
if [ -n "$COMPARE" ] && ! ps -p "$COMPARE" >/dev/null 2>&1; then
  echo "measure.sh: no process $COMPARE" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN_ARG:-${CARGO_TARGET_DIR:-$ROOT/target}/release/wingbar}"
case "$BIN" in /*) ;; *) BIN="$PWD/$BIN" ;; esac
if [ "$BUILD" = 1 ]; then
  echo "building release..." >&2
  (cd "$ROOT" && cargo build --release --quiet)
fi
[ -x "$BIN" ] || { echo "measure.sh: no binary at $BIN" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/wingbar-measure.XXXXXX")"
BAR=""
cleanup() {
  # only ever the bar this script started
  if [ -n "$BAR" ] && ps -p "$BAR" -o comm= 2>/dev/null | grep -q wingbar; then
    kill "$BAR" 2>/dev/null || true
  fi
  if [ "$KEEP" = 1 ]; then echo "work dir kept: $WORK" >&2; else rm -rf "$WORK"; fi
}
trap cleanup EXIT
trap 'echo "interrupted" >&2; exit 130' INT TERM
trap '' PIPE # a closed pipe (| head) must not skip the cleanup
trap 'echo "measure.sh: failed at line $LINENO" >&2' ERR

BAR_HOME="$HOME"
MODE="normal (your config)"
if [ "$ISOLATED" = 1 ]; then
  MODE="isolated (built-ins only)"
  BAR_HOME="$WORK/home"
  mkdir -p "$BAR_HOME"
  # every built-in on, no user widgets; the window manager is detected as usual
  cat >"$BAR_HOME/.simplebarrc" <<'EOF'
{
  "widgets": {
    "processWidget": true, "cpuWidget": true, "memoryWidget": true,
    "batteryWidget": true, "soundWidget": true, "dateWidget": true,
    "timeWidget": true, "spotifyWidget": true, "musicWidget": true
  },
  "userWidgets": { "userWidgetsList": {} }
}
EOF
fi

now() { perl -MTime::HiRes=time -e 'printf "%.2f\n", time' 2>/dev/null || date +%s; }
# ps prints CPU time as [[dd-]hh:]mm:ss.cc
secs() { awk -v t="$1" 'BEGIN { n = split(t, p, ":"); s = 0; for (i = 1; i <= n; i++) s = s * 60 + p[i]; print s }'; }

# sample PID LABEL T: one line "t cpu rss_kb threads children" into $WORK/LABEL.ps
sample_ps() {
  local pid="$1" label="$2" t="$3" line u s rss th kids
  line="$(ps -o utime=,stime=,rss= -p "$pid" 2>/dev/null)" || return 1
  [ -n "$line" ] || return 1
  set -- $line
  u="$(secs "$1")"; s="$(secs "$2")"; rss="$3"
  th="$(ps -M -p "$pid" 2>/dev/null | awk 'NR > 1' | wc -l | tr -d ' ' || true)"
  kids="$(pgrep -P "$pid" 2>/dev/null | wc -l | tr -d ' ' || true)"
  echo "$t $(awk -v u="$u" -v s="$s" 'BEGIN { print u + s }') $rss $th $kids" >>"$WORK/$label.ps"
}

# sample_slow PID LABEL T: "t idlew csw footprint_bytes" (top and footprint are slower)
sample_slow() {
  local pid="$1" label="$2" t="$3" top fp
  top="$(top -l 1 -stats pid,idlew,csw -pid "$pid" 2>/dev/null | awk -v p="$pid" '$1 == p { print $2, $3 }' | tr -d '+' || true)"
  fp="$(footprint --noCategories -f bytes "$pid" 2>/dev/null | awk '/phys_footprint:/ { print $2; exit }' || true)"
  [ -n "$top" ] || top="- -"
  echo "$t $top ${fp:--}" >>"$WORK/$label.slow"
}

cd "$WORK"
echo "running wingbar for ${DURATION}s, $MODE..." >&2
HOME="$BAR_HOME" WINGBAR_NO_PROMPT=1 WINGBAR_MEASURE="$WORK/usage.txt" WINGBAR_MEASURE_WARMUP="$WARMUP" \
  "$BIN" --no-server --quit-after "$DURATION" >"$WORK/bar.log" 2>&1 &
BAR=$!
T0="$(now)"
SLOW_EVERY=5 # top + footprint every 5th sample
n=0
while ps -p "$BAR" >/dev/null 2>&1; do
  t="$(awk -v a="$(now)" -v b="$T0" 'BEGIN { printf "%.2f", a - b }')"
  sample_ps "$BAR" bar "$t" || break
  if [ -n "$COMPARE" ]; then sample_ps "$COMPARE" cmp "$t" || true; fi
  if [ $((n % SLOW_EVERY)) = 0 ] || awk -v t="$t" -v d="$DURATION" -v i="$INTERVAL" 'BEGIN { exit !(t + 2 * i > d) }'; then
    sample_slow "$BAR" bar "$t"
    if [ -n "$COMPARE" ]; then sample_slow "$COMPARE" cmp "$t"; fi
  fi
  n=$((n + 1))
  sleep "$INTERVAL"
done
STATUS=0
wait "$BAR" || STATUS=$?
BAR=""
[ "$STATUS" = 0 ] || echo "warning: wingbar exited with status $STATUS (log: $WORK/bar.log)" >&2
[ -s "$WORK/bar.ps" ] || { echo "measure.sh: no samples (did it start? see $WORK/bar.log)" >&2; KEEP=1; exit 1; }

# ── report ──
calc() { awk "BEGIN { $1 }"; }
steady() { # FILE -> "seconds cpu_seconds percent", first sample after the warmup to the last
  awk -v w="$WARMUP" '$1 >= w { if (!n++) { t0 = $1; c0 = $2 } t1 = $1; c1 = $2 }
    END { d = t1 - t0; if (d > 0) printf "%.0f %.3f %.3f", d, c1 - c0, (c1 - c0) / d * 100; else print "0 - -" }' "$1"
}
col_stats() { # FILE COL -> "min avg max" over the samples after the warmup
  [ -f "$1" ] || { echo "- - -"; return; }
  awk -v c="$2" -v w="$WARMUP" '$1 >= w { v = $c; if (!n++ || v < mn) mn = v; if (v > mx) mx = v; s += v }
    END { if (n) printf "%s %.1f %s", mn, s / n, mx; else print "- - -" }' "$1"
}
rate() { # FILE COL -> per second, first to last slow sample after the warmup
  [ -f "$1" ] || { echo "-"; return; }
  awk -v c="$2" -v w="$WARMUP" '$1 >= w && $c != "-" { if (!n++) { t0 = $1; v0 = $c } t1 = $1; v1 = $c }
    END { d = t1 - t0; if (n > 1 && d > 0) printf "%.2f", (v1 - v0) / d; else print "-" }' "$1"
}
footprints() { # FILE -> "last / max" in MB
  [ -f "$1" ] || { echo "-"; return; }
  awk '$4 != "-" { l = $4; if ($4 > m) m = $4 } END { if (m) printf "%.1f / %.1f", l / 1048576, m / 1048576; else print "-" }' "$1"
}
mbs() { # "min avg max" in KB -> MB
  awk -v s="$1" 'BEGIN { split(s, a, " "); if (a[1] == "-") print "-"; else printf "%.1f / %.1f / %.1f", a[1] / 1024, a[2] / 1024, a[3] / 1024 }'
}
slashed() { echo "$1" | sed 's# # / #g'; }

# the bar's own report, at the end and (keys prefixed "warm.") after the warmup
U="$WORK/usage.txt"
val() { awk -v k="$1" '$1 == k { print $2 }' "$U" 2>/dev/null; }
sum() { # PREFIX KEY... -> the sum of those values
  awk -v p="$1" -v keys="${*:2}" 'BEGIN { n = split(keys, k, " "); for (i = 1; i <= n; i++) want[p k[i]] = 1 }
    ($1 in want) { s += $2 } END { printf "%.6f", s }' "$U"
}
line() { # UNIT LABEL KEY...: the steady value (since the warmup) and the whole-run value
  local unit="$1" label="$2" end warm a b
  shift 2
  end="$(sum "" "$@")"
  warm="$(sum warm. "$@")"
  case "$unit" in
    s) a="$(calc "printf \"%.3f\", $end - $warm")"; b="$(calc "printf \"%.3f\", $end")" ;;
    pct) a="$(calc "printf \"%.3f\", ($end - $warm) / $D * 100")"; b="$(calc "printf \"%.3f\", $end / $S * 100")" ;;
    mps) a="$(calc "printf \"%.2f\", ($end - $warm) / $D / 1e6")"; b="$(calc "printf \"%.2f\", $end / $S / 1e6")" ;;
    min) a="$(calc "printf \"%.1f\", ($end - $warm) / $D * 60")"; b="$(calc "printf \"%.1f\", $end / $S * 60")" ;;
    *) a="$(calc "printf \"%.2f\", ($end - $warm) / $D")"; b="$(calc "printf \"%.2f\", $end / $S")" ;;
  esac
  printf '  %-42s %12s %12s\n' "$label" "$a" "$b"
}

DIRTY=""
git -C "$ROOT" diff --quiet HEAD 2>/dev/null || DIRTY=" (+ uncommitted changes)"
echo
echo "wingbar $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo '?')$DIRTY, $MODE, ${DURATION}s run"
[ -z "$BIN_ARG" ] || echo "binary: $BIN"
echo "machine: load average $(sysctl -n vm.loadavg | tr -d '{}' | awk '{ print $1, $2, $3 }'), $(sysctl -n hw.ncpu) CPUs"
echo

if [ -s "$U" ] && [ -n "$(val warm.seconds)" ]; then
  S="$(val seconds)"
  D="$(calc "print $S - $(val warm.seconds)")"
  printf '  %-42s %12s %12s\n' "measured inside the bar" "last ${D}s" "whole ${S}s"
  echo "  the bar itself"
  line s "    CPU time, user+sys (s)" user system
  line pct "    average CPU (% of one core)" user system
  line mps "    instructions (millions/s)" instructions
  line ps "    idle wakeups (/s)" idle_wakeups
  line ps "    interrupt wakeups (/s)" interrupt_wakeups
  line ps "    voluntary context switches (/s)" voluntary_switches
  line min "    widget runs (/min)" runs
  line min "    runs skipped, widget still busy (/min)" skipped
  line min "    threads started by the scheduler (/min)" threads_started
  echo "  its child processes (widget scripts), once reaped"
  line s "    CPU time, user+sys (s)" child_user child_system
  line pct "    average CPU (% of one core)" child_user child_system
  echo "  memory (at the end)"
  printf '  %-42s %12s %12s\n' "    peak RSS (MB)" "" "$(calc "printf \"%.1f\", $(val max_rss) / 1048576")"
  printf '  %-42s %12s %12s\n' "    phys footprint (MB)" "" "$(calc "printf \"%.1f\", $(val phys_footprint) / 1048576")"
  printf '  %-42s %12s %12s\n' "    phys footprint, lifetime peak (MB)" "" "$(calc "printf \"%.1f\", $(val max_phys_footprint) / 1048576")"
else
  echo "  (no report from inside the bar: $U is missing; see $WORK/bar.log)"
fi

read -r B_SECS B_CPU B_PCT <<<"$(steady "$WORK/bar.ps")"
C_CPU=""; C_PCT=""; C_NAME=""
if [ -n "$COMPARE" ] && [ -s "$WORK/cmp.ps" ]; then
  read -r _ C_CPU C_PCT <<<"$(steady "$WORK/cmp.ps")"
  C_NAME="$(ps -o comm= -p "$COMPARE" 2>/dev/null | sed 's#.*/##' | cut -c1-12)"
  C_NAME="${C_NAME:-?} (pid $COMPARE)"
fi
row() { # LABEL WINGBAR COMPARE
  if [ -n "$C_NAME" ]; then printf '  %-42s %20s %20s\n' "$1" "$2" "$3"; else printf '  %-42s %20s\n' "$1" "$2"; fi
}
echo
row "sampled from outside, last ${B_SECS}s" "wingbar" "$C_NAME"
row "    CPU time (s)" "$B_CPU" "$C_CPU"
row "    average CPU (% of one core)" "$B_PCT" "$C_PCT"
row "    idle wakeups (/s, top)" "$(rate "$WORK/bar.slow" 2)" "$(rate "$WORK/cmp.slow" 2)"
row "    context switches (/s, top)" "$(rate "$WORK/bar.slow" 3)" "$(rate "$WORK/cmp.slow" 3)"
row "    RSS min / avg / max (MB)" "$(mbs "$(col_stats "$WORK/bar.ps" 3)")" "$(mbs "$(col_stats "$WORK/cmp.ps" 3)")"
row "    phys footprint last / max (MB)" "$(footprints "$WORK/bar.slow")" "$(footprints "$WORK/cmp.slow")"
row "    threads min / avg / max" "$(slashed "$(col_stats "$WORK/bar.ps" 4)")" "$(slashed "$(col_stats "$WORK/cmp.ps" 4)")"
row "    live child processes, max" "$(col_stats "$WORK/bar.ps" 5 | awk '{ print $3 }')" \
  "$(col_stats "$WORK/cmp.ps" 5 | awk '{ print $3 }')"
echo
echo "($(wc -l <"$WORK/bar.ps" | tr -d ' ') samples every ${INTERVAL}s. CPU time and wakeups are noisy on a busy machine;"
echo " instructions are the steadiest number to compare two runs by.)"

