#!/usr/bin/env bash
# Cross-check our EBU R128 implementation against FFmpeg's `ebur128` filter.
#
# Two independent implementations of a precisely specified algorithm should
# agree closely. They will not agree EXACTLY — gating boundaries and block
# alignment differ between implementations — so the comparison is to a
# tolerance, printed rather than asserted so the actual numbers are visible.
#
# Uses the HOMEBREW ffmpeg deliberately: it is a different codebase from the one
# we link, which is the whole point of a cross-check. It is never distributed
# (DEPENDENCY_AND_LICENSE_AUDIT.md §3.1).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FFMPEG="${FFMPEG:-ffmpeg}"

# Build the reader rather than depending on a stale binary being around.
"$ROOT/tools/ct.sh" build -q -p mediacore-media --example loudness
OURS="$ROOT/core/target/debug/examples/loudness"

printf "%-28s %10s %10s %8s\n" FILE OURS FFMPEG DIFF
for f in "$@"; do
  ours=$("$OURS" "$f" 2>/dev/null | head -1)
  # ebur128 prints its summary at INFO level, on stderr.
  theirs=$("$FFMPEG" -hide_banner -nostats -i "$f" -filter_complex ebur128 -f null - 2>&1 \
           | grep -A5 "Integrated loudness" | grep -m1 "I:" | awk '{print $2}')
  if [ -z "$theirs" ]; then theirs="n/a"; diff="n/a"; else
    diff=$(python3 -c "print(f'{abs($ours - ($theirs)):.2f}')" 2>/dev/null || echo "?")
  fi
  printf "%-28s %10s %10s %8s\n" "$(basename "$f")" "$ours" "$theirs" "$diff"
done
