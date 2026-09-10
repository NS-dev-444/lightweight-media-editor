#!/usr/bin/env bash
# S4b — CONSTANT QUALITY comparison.
#
# S4 used single-pass ABR (-b:v) for every encoder. That is the mode a naive
# "export at 8 Mbps" preset uses, but it may handicap VideoToolbox, which has a
# native quality-targeted mode. If the quality CEILING seen in S4 is an ABR
# artefact, AD-12 would be decided on a wrong number -- so this re-tests each
# encoder in ITS OWN best mode and compares the resulting rate-quality curves.
#
#   libx264            -crf   (its native quality mode)
#   *_videotoolbox     -q:v   (kVTCompressionPropertyKey_Quality)
#
# Bitrate is then MEASURED from the output, not requested.
set -euo pipefail
FFMPEG="${FFMPEG:-/opt/homebrew/bin/ffmpeg}"     # measurement tooling, never shipped
OUT_DIR="${OUT_DIR:-build/benchmark_cq}"
X264_CRF=(18 21 24 27 30)
VT_Q=(88 78 68 58 48)

FILTER_LIST="$("${FFMPEG}" -hide_banner -filters 2>/dev/null || true)"
case "${FILTER_LIST}" in *libvmaf*) ;; *) echo "no libvmaf"; exit 2 ;; esac
(( $# )) || { echo "usage: $0 <ref.mp4> [...]"; exit 2; }

mkdir -p "${OUT_DIR}"
CSV="${OUT_DIR}/results_cq.csv"
echo "source,encoder,setting,vmaf,measured_kbps,output_bytes" > "${CSV}"

vmaf_of() {
  "${FFMPEG}" -hide_banner -loglevel info -i "$1" -i "$2" \
    -lavfi "[0:v][1:v]libvmaf=n_threads=8" -f null - 2>&1 \
    | grep -oE 'VMAF score: [0-9.]+' | grep -oE '[0-9.]+$' || echo "NA"
}

for ref in "$@"; do
  [[ -f "${ref}" ]] || { echo "missing ${ref}"; continue; }
  label="$(basename "${ref%.*}")"; label="${label%_ref}"
  dur=$("${FFMPEG}" -hide_banner -i "${ref}" 2>&1 \
        | grep -oE 'Duration: [0-9:.]+' | head -1 | cut -d' ' -f2 \
        | awk -F: '{print ($1*3600)+($2*60)+$3}' || true)
  dur="${dur:-5}"
  echo "== ${label} (${dur}s)"

  for crf in "${X264_CRF[@]}"; do
    out="${OUT_DIR}/${label}_x264_crf${crf}.mp4"
    "${FFMPEG}" -hide_banner -loglevel error -y -i "${ref}" \
        -c:v libx264 -crf "${crf}" -preset medium -an "${out}" 2>/dev/null || continue
    b=$(stat -f%z "${out}"); kbps=$(python3 -c "print(f'{$b*8/$dur/1000:.0f}')")
    v=$(vmaf_of "${out}" "${ref}")
    printf "  %-22s crf %-3s  %8s kbps  VMAF %s\n" libx264 "${crf}" "${kbps}" "${v}"
    echo "${label},libx264,crf${crf},${v},${kbps},${b}" >> "${CSV}"
  done

  for enc in h264_videotoolbox hevc_videotoolbox; do
    for q in "${VT_Q[@]}"; do
      out="${OUT_DIR}/${label}_${enc}_q${q}.mp4"
      "${FFMPEG}" -hide_banner -loglevel error -y -i "${ref}" \
          -c:v "${enc}" -q:v "${q}" -an "${out}" 2>/dev/null || continue
      b=$(stat -f%z "${out}"); kbps=$(python3 -c "print(f'{$b*8/$dur/1000:.0f}')")
      v=$(vmaf_of "${out}" "${ref}")
      printf "  %-22s q   %-3s  %8s kbps  VMAF %s\n" "${enc}" "${q}" "${kbps}" "${v}"
      echo "${label},${enc},q${q},${v},${kbps},${b}" >> "${CSV}"
    done
  done
done
echo; echo "Results: ${CSV}"
