#!/usr/bin/env bash
# S4 test corpus — open-licensed footage for encoder quality benchmarking.
#
# Sources are LOSSLESS FRAME SEQUENCES, not compressed video: a compressed
# reference would bake in generation loss and corrupt every VMAF number.
#
# Licences (recorded per DEPENDENCY_AND_LICENSE_AUDIT.md discipline, even though
# this corpus is measurement tooling and is never distributed):
#   Sintel          (c) Blender Foundation — CC BY 3.0
#   Tears of Steel  (c) Blender Foundation — CC BY 3.0
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

grab() {  # $1 base-url  $2 out-dir  $3 start  $4 count  $5 digits  $6 prefix
  local base="$1" out="$2" start="$3" count="$4" digits="$5" prefix="${6:-}"
  mkdir -p "$out"
  seq "$start" $((start + count - 1)) \
    | awk -v d="$digits" -v p="$prefix" '{printf "%s%0*d.png\n", p, d, $1}' \
    | xargs -P 8 -I{} sh -c 'test -s '"$out"'/{} || curl -sfL --max-time 120 -o '"$out"'/{} '"$base"'/{}'
  echo "  $(ls "$out" | wc -l | tr -d ' ') frames -> $out"
}

echo "Sintel 4K scene A (frames 5000-5119)"
grab https://media.xiph.org/sintel/sintel-4k-png "$ROOT/media/corpus/sintel_4k_a" 5000 120 8
echo "Sintel 4K scene B (frames 15000-15119)"
grab https://media.xiph.org/sintel/sintel-4k-png "$ROOT/media/corpus/sintel_4k_b" 15000 120 8
echo "Tears of Steel 1080p scene A (frames 1000-1149)"
grab https://media.xiph.org/tearsofsteel/tearsofsteel-1080bis-png "$ROOT/media/corpus/tos_1080_a" 1000 150 5
echo "Tears of Steel 1080p scene B (frames 8000-8149)"
grab https://media.xiph.org/tearsofsteel/tearsofsteel-1080bis-png "$ROOT/media/corpus/tos_1080_b" 8000 150 5
echo "done"
