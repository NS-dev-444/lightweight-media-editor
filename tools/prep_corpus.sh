#!/usr/bin/env bash
# Turn the downloaded lossless frame sequences into lossless reference clips.
#
# Why convert at all: reading thousands of PNGs per encode is slow, and every
# encoder must consume IDENTICAL input for the comparison to mean anything.
#
# Why yuv420p: the PNGs are RGB. Converting to 4:2:0 is itself lossy (chroma
# subsampling), so it must happen ONCE, up front, in the reference — not
# separately inside each encoder run, which would let encoders be scored on
# slightly different source pixels.
set -euo pipefail
FFMPEG="${FFMPEG:-/opt/homebrew/bin/ffmpeg}"   # measurement tooling, never shipped
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/media/corpus"

prep() {  # $1 dir  $2 glob  $3 fps  $4 start-number
  local dir="$OUT/$1" pat="$2" fps="$3" start="$4"
  local n; n=$(ls "$dir" 2>/dev/null | wc -l | tr -d ' ')
  [[ "$n" -gt 0 ]] || { echo "  $1: no frames, skipped"; return; }
  "$FFMPEG" -hide_banner -loglevel error -y \
    -framerate "$fps" -start_number "$start" -i "$dir/$pat" \
    -c:v libx264 -qp 0 -preset veryfast -pix_fmt yuv420p \
    "$OUT/$1.mp4"
  # `ffmpeg -i` with no output file exits 1; with pipefail that would abort the
  # script, so the probe is explicitly tolerated.
  local res
  res=$("$FFMPEG" -hide_banner -i "$OUT/$1.mp4" 2>&1 | grep -oE '[0-9]{3,4}x[0-9]{3,4}' | head -1 || true)
  printf "  %-16s %4s frames  %-10s %s\n" "$1" "$n" "$res" \
         "$(du -h "$OUT/$1.mp4" | cut -f1)"
}

echo "Building lossless reference clips:"
prep sintel_4k_a "%08d.png" 24 5000
prep sintel_4k_b "%08d.png" 24 15000
prep tos_1080_a  "%05d.png" 24 1000
prep tos_1080_b  "%05d.png" 24 8000

# A synthetic screen-recording clip: sharp text edges, near-static, occasional
# scroll. Real screen recordings are a distinct encoding regime (high spatial
# frequency, low temporal change) and no open film corpus contains one.
# Synthesised rather than captured — capturing would record the user's screen.
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "color=c=0x1e1e1e:s=1920x1080:r=30" \
  -vf "drawtext=text='fn render(ctx: \&mut Context) -> Result<(), Error> {':fontcolor=0xd4d4d4:fontsize=28:x=60:y=80+mod(t*40\,200), \
drawtext=text='    let frame = decoder.next()?;':fontcolor=0x9cdcfe:fontsize=28:x=60:y=130+mod(t*40\,200), \
drawtext=text='    surface.bind(frame.texture());':fontcolor=0xce9178:fontsize=28:x=60:y=180+mod(t*40\,200), \
drawtext=text='}':fontcolor=0xd4d4d4:fontsize=28:x=60:y=230+mod(t*40\,200)" \
  -t 5 -c:v libx264 -qp 0 -preset veryfast -pix_fmt yuv420p "$OUT/screencast_1080.mp4" 2>/dev/null \
  && printf "  %-16s synthetic  1920x1080  %s\n" "screencast_1080" "$(du -h "$OUT/screencast_1080.mp4" | cut -f1)" \
  || echo "  screencast: skipped (drawtext unavailable)"
# A clip WITH SOUND. Every other fixture is silent, and an audio-free corpus
# cannot catch the failure that matters most in conversion: an output that plays
# but has lost its soundtrack. Deliberately AAC in MP4, so the transcode tests
# exercise both the copy path and the re-encode path.
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=s=1280x720:r=30" \
  -f lavfi -i "sine=frequency=440:sample_rate=48000" \
  -t 5 -c:v libx264 -crf 20 -preset veryfast -pix_fmt yuv420p \
  -c:a aac -b:a 128k -shortest "$ROOT/media/av_sync.mp4" \
  && printf "  %-16s synthetic  1280x720 + AAC  %s\n" "av_sync" "$(du -h "$ROOT/media/av_sync.mp4" | cut -f1)" \
  || echo "  av_sync: skipped"

# The same idea with UNCOMPRESSED audio in Matroska. MP4 will not carry PCM, so
# converting this one forces the audio RE-ENCODE path; av_sync.mp4 (AAC in MP4)
# takes the copy path. One fixture each, or half the code is never run.
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=s=640x480:r=25" \
  -f lavfi -i "sine=frequency=440:sample_rate=48000" \
  -t 3 -c:v libx264 -crf 22 -preset veryfast -pix_fmt yuv420p \
  -c:a pcm_s16le -shortest "$ROOT/media/av_pcm.mkv" \
  && printf "  %-16s synthetic  640x480 + PCM   %s\n" "av_pcm" "$(du -h "$ROOT/media/av_pcm.mkv" | cut -f1)" \
  || echo "  av_pcm: skipped"

# Two analysis fixtures with KNOWN answers, so the loudness and silence code is
# checked against arithmetic rather than against itself.
#   tone_-20dbfs.wav  a 1 kHz sine of amplitude EXACTLY 0.1 in both channels.
#                     R128 then says -0.691 + 20*log10(0.1) = -20.69 LUFS, which
#                     is an arithmetic fact rather than another measurement.
#                     Built with aevalsrc, not sine+volume: the amplitude has to
#                     be stated outright for the expected value to mean anything.
#   gap.wav           tone, two seconds of true silence, tone.
"$FFMPEG" -hide_banner -loglevel error -y \
  -i "aevalsrc=0.1*sin(2*PI*1000*t)|0.1*sin(2*PI*1000*t):s=48000:d=6" \
  -ac 2 -c:a pcm_s16le "$ROOT/media/tone_-20dbfs.wav" \
  && printf "  %-16s 1 kHz at -20 dBFS\n" "tone_-20dbfs" || echo "  tone: skipped"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1" \
  -f lavfi -i "anullsrc=r=48000:cl=stereo:duration=2" \
  -f lavfi -i "sine=frequency=880:sample_rate=48000:duration=1" \
  -filter_complex "[0:a][1:a][2:a]concat=n=3:v=0:a=1[out]" -map "[out]" \
  -ac 2 -c:a pcm_s16le "$ROOT/media/gap.wav" \
  && printf "  %-16s tone / 2 s silence / tone\n" "gap" || echo "  gap: skipped"
# A deliberately EXTREME test LUT. A subtle one cannot be told from "the LUT did
# not load" in a screenshot, which is the failure this fixture exists to catch.
python3 - "$ROOT/media/teal_orange.cube" <<'PYLUT'
import sys
N = 17
out = ['TITLE "Teal and Orange (test)"', f"LUT_3D_SIZE {N}",
       "DOMAIN_MIN 0.0 0.0 0.0", "DOMAIN_MAX 1.0 1.0 1.0", ""]
for b in range(N):
    for g in range(N):
        for r in range(N):
            R, G, B = r/(N-1), g/(N-1), b/(N-1)
            t = 0.2126*R + 0.7152*G + 0.0722*B
            nr = min(0.06 + 0.94*(R*(0.55 + 0.75*t)), 1)
            ng = min(0.05 + 0.95*(G*(0.80 + 0.25*t)), 1)
            nb = min(0.08 + 0.92*(B*(1.25 - 0.65*t)), 1)
            out.append(f"{nr:.6f} {ng:.6f} {nb:.6f}")
open(sys.argv[1], "w").write("\n".join(out) + "\n")
PYLUT
printf "  %-16s 17-cube test LUT\n" "teal_orange"
# Speech with KNOWN words, for the caption tests. Synthesised with macOS `say`
# so the expected transcript is not a judgement call: word accuracy can be
# measured rather than eyeballed. Also entirely local — no recording of anyone.
SPEECH="The quick brown fox jumps over the lazy dog. Video editing should be simple and fast. This sentence is here to test the caption timing."
if command -v say >/dev/null 2>&1; then
  say -o "$ROOT/media/speech.aiff" "$SPEECH" \
    && "$FFMPEG" -hide_banner -loglevel error -y -i "$ROOT/media/speech.aiff" \
         -ac 2 -ar 48000 -c:a pcm_s16le "$ROOT/media/speech.wav" \
    && rm -f "$ROOT/media/speech.aiff" \
    && printf "  %-16s synthesised speech, known words\n" "speech"

  # A LONG one, past the 25-second transcription chunk, so the seam between
  # chunks is exercised. A fixed-overlap implementation lost a whole sentence
  # there, silently; only a multi-chunk fixture catches that.
  python3 -c "
lines = []
for i in range(1, 13):
    lines.append(f'This is sentence number {i}. It exists to make the recording long enough to need more than one chunk.')
print(' '.join(lines))
" > "$ROOT/media/.long.txt"
  say -o "$ROOT/media/.long.aiff" -f "$ROOT/media/.long.txt" \
    && "$FFMPEG" -hide_banner -loglevel error -y -i "$ROOT/media/.long.aiff" \
         -ac 2 -ar 48000 -c:a pcm_s16le "$ROOT/media/speech_long.wav" \
    && rm -f "$ROOT/media/.long.aiff" "$ROOT/media/.long.txt" \
    && printf "  %-16s 60 s, spans three chunks\n" "speech_long"
else
  echo "  speech: skipped (no \`say\`; macOS only)"
fi
echo "done"
