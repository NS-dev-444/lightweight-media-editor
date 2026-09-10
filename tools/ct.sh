#!/usr/bin/env bash
# cargo, with the FFmpeg environment our own LGPL build needs.
#
# The paths are ABSOLUTE. Relative ones are resolved against the working
# directory, so the same command worked from core/ and failed from core/src —
# a confusing "include dir doesn't exist" for a directory that plainly does.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export FFMPEG_INCLUDE_DIR="$ROOT/build/ffmpeg-lgpl/include"
export FFMPEG_LIBS_DIR="$ROOT/build/ffmpeg-lgpl/lib"
export FFMPEG_LINK_MODE=dynamic
cd "$ROOT/core"
exec cargo "$@"
