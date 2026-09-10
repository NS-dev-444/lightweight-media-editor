#!/usr/bin/env bash
# Fetch Whisper models for on-device transcription (O-18).
#
# SHA256-pinned, like every other third-party artefact here. A model is code as
# far as trust goes: it is downloaded over the network and then executed against
# the user's private media, and "whatever the CDN served today" is not a
# dependency.
#
# The models are MIT-licensed (OpenAI, weights included), so they may be
# redistributed inside the product — see DEPENDENCY_AND_LICENSE_AUDIT.md §7.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/build/models"
BASE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
mkdir -p "$OUT"

# The bundled model, and why this one (O-18, decided 2026-09-10):
#
#   ggml-base-q5_1.bin   57 MB   multilingual   ← BUNDLED
#
# The size is the argument. The app plus its FFmpeg libraries is around 60 MB,
# so 57 MB roughly doubles it and anything larger would contradict the word
# "lightweight" in the product's own name. base-q5_1 is the largest model that
# stays on the right side of that line.
#
# **Multilingual, not base.en.** English-only is slightly more accurate on
# English and completely useless on everything else, and a caption feature that
# only works in one language is not a caption feature.
#
# **Bundled rather than downloaded**, which is a departure from
# PRODUCT_DIRECTION §8's "preferred shape". Shipping it means captions work on
# first launch with no network at all, so the first-run network requirement that
# §8 says "must be stated, not discovered" simply does not exist. Larger models
# stay an explicit opt-in download for people who want the accuracy and have
# said so.
#
# Optional, offered in the app rather than fetched here:
#   ggml-small-q5_1.bin           181 MB   noticeably better
#   ggml-large-v3-turbo-q5_0.bin  547 MB   best available, still fast on Metal

fetch() {  # $1 file  $2 expected-sha  $3 label
  local file="$1" want="$2" label="$3" path="$OUT/$1"
  if [[ -f "$path" ]]; then
    local have; have="$(shasum -a 256 "$path" | awk '{print $1}')"
    if [[ "$have" == "$want" ]]; then
      printf "  %-28s ✓ present\n" "$file"; return 0
    fi
    echo "  $file: checksum mismatch, refetching" >&2
    rm -f "$path"
  fi
  printf "  %-28s downloading (%s)…\n" "$file" "$label"
  curl -sSfL --retry 3 -o "$path.part" "$BASE/$file"
  local have; have="$(shasum -a 256 "$path.part" | awk '{print $1}')"
  if [[ -n "$want" && "$have" != "$want" ]]; then
    rm -f "$path.part"
    echo "FATAL: $file hashed $have, expected $want" >&2
    exit 1
  fi
  mv "$path.part" "$path"
  printf "  %-28s ✓ %s  sha256 %s\n" "$file" "$(du -h "$path" | cut -f1)" "$have"
}

echo "Whisper models -> $OUT"
fetch "ggml-base-q5_1.bin" \
  "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898" \
  "bundled default"
echo "done"
