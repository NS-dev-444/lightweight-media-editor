#!/usr/bin/env bash
# Build whisper.cpp for on-device transcription (§8's captions, O-16/O-18).
#
# LICENSING — the reason this dependency is acceptable at all:
#   whisper.cpp   MIT, "Copyright (c) 2023-2026 The ggml authors"
#   Whisper model MIT, "Copyright (c) 2022 OpenAI" — CODE *and* WEIGHTS
#
# Weights being MIT is rare and is what makes bundling one legal. There is no
# royalty, no account and no service: see DEPENDENCY_AND_LICENSE_AUDIT.md §7.
#
# PINNED to a tag, with the commit recorded, for the same reason FFmpeg is:
# "whatever is on main today" is not a dependency, it is a moving target.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/tools/deployment.sh"
SRC="$ROOT/build/whisper-src"
OUT="$ROOT/build/whisper"

TAG="v1.9.3"
# The DEREFERENCED commit, not the annotated tag object's own SHA.
# `git ls-remote --tags` prints both; the tag object is the one that is NOT
# what a clone checks out, and pinning it makes every build fail the check.
COMMIT="371b5a7561823ab2bb32142d2751e35e7534727b"

echo "==> fetching whisper.cpp $TAG"
if [[ ! -d "$SRC/.git" ]]; then
  rm -rf "$SRC"
  git clone --quiet --depth 1 --branch "$TAG" https://github.com/ggml-org/whisper.cpp "$SRC"
fi
have="$(cd "$SRC" && git rev-parse HEAD)"
if [[ "$have" != "$COMMIT" ]]; then
  echo "FATAL: expected $COMMIT, got $have" >&2
  echo "The tag moved, or the clone is stale. Verify before changing the pin." >&2
  exit 1
fi
echo "    commit $have verified"

# Sanity: the licence must be the one the audit cleared.
if ! grep -q "MIT License" "$SRC/LICENSE"; then
  echo "FATAL: whisper.cpp's LICENSE is not the MIT one the audit cleared." >&2
  exit 1
fi

echo "==> building"
# Metal ON: this is the whole reason transcription is fast enough to be a
# feature rather than a progress bar. No CoreML — it needs a separate converted
# model per size and doubles the download for a modest gain.
#
# Everything else OFF. We link the library; the examples pull in SDL2 and a
# server, neither of which belongs anywhere near a shipped product.
cmake -S "$SRC" -B "$SRC/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET" \
  -DCMAKE_INSTALL_PREFIX="$OUT" \
  -DBUILD_SHARED_LIBS=OFF \
  -DWHISPER_BUILD_EXAMPLES=OFF \
  -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF \
  -DGGML_METAL=ON \
  -DGGML_METAL_EMBED_LIBRARY=ON \
  -DGGML_ACCELERATE=ON \
  >/dev/null

cmake --build "$SRC/build" --config Release -j"$(sysctl -n hw.ncpu)" >/dev/null
cmake --install "$SRC/build" >/dev/null

# The licence travels with the artifact — same reasoning as the FFmpeg build.
mkdir -p "$OUT/share/licences"
cp "$SRC/LICENSE" "$OUT/share/licences/whisper.cpp-MIT.txt"
printf 'whisper.cpp %s (%s)\n' "$TAG" "$COMMIT" > "$OUT/share/licences/VERSIONS.txt"

echo "==> installed to $OUT"
ls "$OUT/lib" | sed 's/^/    /'
