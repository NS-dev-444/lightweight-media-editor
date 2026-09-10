#!/usr/bin/env python3
"""S6 corpus generator — malformed media, per spec §42's failure matrix.

Everything here is deliberately broken. The point is not that FFmpeg mishandles
it (it is robust); the point is that the ARCHITECTURE survives whatever it does.
"""
import os, random, shutil, subprocess, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "build/s6-corpus"
FFMPEG = "/opt/homebrew/bin/ffmpeg"   # test-media tooling only, never shipped
random.seed(20260909)

shutil.rmtree(OUT, ignore_errors=True)
os.makedirs(OUT, exist_ok=True)

base = os.path.join(OUT, "_base.mp4")
subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc2=size=640x480:rate=30",
                "-f", "lavfi", "-i", "sine=frequency=440",
                "-t", "3", "-c:v", "libx264", "-preset", "ultrafast",
                "-c:a", "aac", base], check=True)
data = open(base, "rb").read()

mkv = os.path.join(OUT, "_base.mkv")
subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-i", base,
                "-c", "copy", mkv], check=True)
mkv_data = open(mkv, "rb").read()

wav = os.path.join(OUT, "_base.wav")
subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440", "-t", "2", wav], check=True)
wav_data = open(wav, "rb").read()

n = 0
def emit(name, blob):
    global n
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(blob)
    n += 1

# 1. Truncation at many points — the "download did not finish" case
for i in range(1, 301):
    cut = int(len(data) * (i / 302.0))
    emit(f"trunc_{i:03d}.mp4", data[:cut])

# 2. Single-byte corruption scattered through the file
for i in range(300):
    b = bytearray(data)
    pos = random.randrange(len(b))
    b[pos] ^= 1 << random.randrange(8)
    emit(f"bitflip_{i:03d}.mp4", bytes(b))

# 3. Burst corruption — a damaged sector
for i in range(150):
    b = bytearray(data)
    pos = random.randrange(max(1, len(b) - 4096))
    for j in range(random.randrange(64, 4096)):
        if pos + j < len(b):
            b[pos + j] = random.randrange(256)
    emit(f"burst_{i:03d}.mp4", bytes(b))

# 4. Header intact, body destroyed
for i in range(80):
    keep = random.randrange(64, 2048)
    emit(f"headonly_{i:03d}.mp4", data[:keep] + os.urandom(len(data) - keep))

# 5. Not media at all
emit("empty.mp4", b"")
emit("onebyte.mp4", b"\x00")
for i in range(60):
    emit(f"garbage_{i:03d}.mp4", os.urandom(random.randrange(1, 200_000)))
emit("textfile.mp4", b"This is plainly not a video file.\n" * 500)
emit("zeros.mp4", b"\x00" * 100_000)
emit("html.mp4", b"<!DOCTYPE html><html><body>404 Not Found</body></html>")

# 6. Truncated MKV and WAV — different demuxers, different failure paths
for i in range(1, 61):
    emit(f"trunc_mkv_{i:02d}.mkv", mkv_data[:int(len(mkv_data) * (i / 62.0))])
for i in range(1, 41):
    emit(f"trunc_wav_{i:02d}.wav", wav_data[:int(len(wav_data) * (i / 42.0))])

# 7. Container/extension mismatch
emit("actually_mkv.mp4", mkv_data)
emit("actually_wav.mp4", wav_data)
emit("actually_mp4.wav", data)
emit("audio_only.mp4", wav_data)          # no video track

# 8. Adversarial sizes claimed in the header
for i in range(20):
    b = bytearray(data)
    b[0:4] = ((0xFFFFFFF0 + i) & 0xFFFFFFFF).to_bytes(4, "big")   # absurd box size
    emit(f"hugebox_{i:02d}.mp4", bytes(b))

# 9. Hostile filenames (§42: unicode, spaces, very long)
emit("with spaces in name.mp4", data[:len(data)//2])
emit("ünïcödé_名前_🎬.mp4", data[:len(data)//2])
emit(("a" * 180) + ".mp4", data[:len(data)//2])
emit("trailing.dots...mp4", data[:len(data)//2])
emit("-leading-dash.mp4", data[:len(data)//2])

for f in ("_base.mp4", "_base.mkv", "_base.wav"):
    os.rename(os.path.join(OUT, f), os.path.join(OUT, f[1:]))  # keep as valid controls

print(f"corpus: {n} malformed files + 3 valid controls in {OUT}")
