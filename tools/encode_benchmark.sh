#!/usr/bin/env bash
#
# S4 — encoder quality/speed benchmark.
#
# Answers: how much bitrate does VideoToolbox need to match libx264 at the same
# perceived quality? AD-12 ships hardware-only H.264/HEVC encode, so RISK_REGISTER.md
# R-09 (users notice worse quality) is a real exposure. Export preset bitrates must
# be chosen FROM THIS DATA, not guessed (spec §46 Rule 8).
#
# LICENCE NOTE — read before changing anything here:
#   This script deliberately uses the SYSTEM/Homebrew ffmpeg, which is a GPL build
#   with libx264 and libvmaf. That is correct and permitted: it is MEASUREMENT
#   TOOLING and its output is never distributed (DEPENDENCY_AND_LICENSE_AUDIT.md §3.1).
#   libx264 is the reference we are measuring AGAINST -- it is never shipped.
#   The product links only against build/ffmpeg-lgpl.
#
# Usage:  ./encode_benchmark.sh <source.mp4> [more sources...]
#
set -euo pipefail

FFMPEG="${FFMPEG:-/opt/homebrew/bin/ffmpeg}"
OUT_DIR="${OUT_DIR:-build/benchmark}"
BITRATES_1080P=(4000 6000 8000 12000)
BITRATES_4K=(15000 25000 35000 50000)
DURATION="${DURATION:-20}"   # seconds sampled from each source

command -v "${FFMPEG}" >/dev/null || { echo "no ffmpeg at ${FFMPEG}"; exit 2; }

# Capture, then match. Do NOT pipe into `grep -q` under `set -o pipefail`:
# grep -q exits on the first match, ffmpeg takes SIGPIPE, and the pipeline
# reports failure *intermittently* depending on who finishes first.
FILTER_LIST="$("${FFMPEG}" -hide_banner -filters 2>/dev/null || true)"
case "${FILTER_LIST}" in
  *libvmaf*) ;;
  *) echo "ERROR: ${FFMPEG} lacks libvmaf; cannot score quality"; exit 2 ;;
esac
(( $# )) || { echo "usage: $0 <source.mp4> [...]"; exit 2; }

mkdir -p "${OUT_DIR}"
CSV="${OUT_DIR}/results.csv"
echo "source,resolution,encoder,bitrate_kbps,vmaf,encode_seconds,output_bytes" > "${CSV}"

score_vmaf() {   # $1 = encoded, $2 = reference
  # -loglevel MUST be info: libvmaf prints its score at info level, so
  # -loglevel error silently yields no score at all.
  "${FFMPEG}" -hide_banner -loglevel info \
    -i "$1" -i "$2" -lavfi "[0:v][1:v]libvmaf=n_threads=8" -f null - 2>&1 \
    | grep -oE 'VMAF score: [0-9.]+' | grep -oE '[0-9.]+$' || echo "NA"
}

run_one() {      # $1 encoder  $2 bitrate  $3 ref  $4 label  $5 res
  local enc="$1" br="$2" ref="$3" label="$4" res="$5"
  local out="${OUT_DIR}/${label}_${enc}_${br}k.mp4"

  local t0 t1
  t0=$(python3 -c 'import time;print(time.time())')
  if [[ "${enc}" == "libx264" ]]; then
    "${FFMPEG}" -hide_banner -loglevel error -y -i "${ref}" \
        -c:v "${enc}" -b:v "${br}k" -preset medium -an "${out}" 2>/dev/null || {
          echo "  ${enc} @ ${br}k FAILED"; return; }
  else
    "${FFMPEG}" -hide_banner -loglevel error -y -i "${ref}" \
        -c:v "${enc}" -b:v "${br}k" -an "${out}" 2>/dev/null || {
          echo "  ${enc} @ ${br}k FAILED"; return; }
  fi
  t1=$(python3 -c 'import time;print(time.time())')

  local secs vmaf bytes
  secs=$(python3 -c "print(f'{${t1}-${t0}:.2f}')")
  vmaf=$(score_vmaf "${out}" "${ref}")
  bytes=$(stat -f%z "${out}")
  printf "  %-22s %6sk  VMAF %-7s %6ss  %s\n" "${enc}" "${br}" "${vmaf}" "${secs}" "${bytes}"
  echo "${label},${res},${enc},${br},${vmaf},${secs},${bytes}" >> "${CSV}"
}

for src in "$@"; do
  [[ -f "${src}" ]] || { echo "missing: ${src}"; continue; }
  label="$(basename "${src%.*}")"
  # NOTE: `ffmpeg -i` with no output file exits 1. Under `set -e` + `pipefail`
  # that silently kills the script, so this probe is explicitly tolerated.
  height=$("${FFMPEG}" -hide_banner -i "${src}" 2>&1 | grep -oE '[0-9]{3,4}x[0-9]{3,4}' | head -1 | cut -dx -f2 || true)
  if [[ "${height:-0}" -gt 1500 ]]; then res="4K"; brs=("${BITRATES_4K[@]}")
  else res="1080p"; brs=("${BITRATES_1080P[@]}"); fi

  ref="${OUT_DIR}/${label}_ref.mp4"
  echo "== ${label} (${res}) — extracting ${DURATION}s lossless reference"
  "${FFMPEG}" -hide_banner -loglevel error -y -t "${DURATION}" -i "${src}" \
      -c:v libx264 -qp 0 -preset ultrafast -an "${ref}"

  for br in "${brs[@]}"; do
    for enc in libx264 h264_videotoolbox hevc_videotoolbox; do
      run_one "${enc}" "${br}" "${ref}" "${label}" "${res}"
    done
  done
done

echo
echo "Results: ${CSV}"
cat <<'NOTE'

INTERPRETING THIS — do not skip:
  * VMAF is only meaningful on REAL footage. Synthetic sources (testsrc, gradients)
    are unrepresentative and will flatter or punish encoders arbitrarily.
    A real corpus is required before any preset bitrate is set: talking head,
    high-motion, low light, screen recording, and graded/film content at minimum.
  * The number that matters is not absolute VMAF. It is the BITRATE RATIO:
    how much more bitrate VideoToolbox needs to match libx264's VMAF.
    That ratio sets the preset bitrates and quantifies RISK_REGISTER.md R-09.
  * Encode times here include process startup and are indicative only.
NOTE
