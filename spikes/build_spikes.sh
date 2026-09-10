#!/usr/bin/env bash
# Rebuild the Phase 1 Swift harnesses against the promoted crate layout.
#
# These are throwaway (§35) but still useful as integration checks: they are
# the only thing that exercises the C ABI from Swift.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FF="$ROOT/build/ffmpeg-lgpl"
CORE="$ROOT/core/mediacore-media"
HDR="$CORE/include/mediacore.h"
LIB="$ROOT/core/target/release"

export FFMPEG_INCLUDE_DIR="$FF/include" FFMPEG_LIBS_DIR="$FF/lib" FFMPEG_LINK_MODE=dynamic
( cd "$ROOT/core" && cargo build --release -p mediacore-media >/dev/null )

common=(-import-objc-header "$HDR" -L "$LIB" -lmediacore
        -L "$FF/lib" -lavcodec -lavformat -lavutil -lswresample -lswscale
        -Xlinker -rpath -Xlinker "$FF/lib"
        -framework CoreVideo -framework CoreMedia -framework VideoToolbox
        -framework CoreFoundation -framework Metal -framework QuartzCore)

( cd "$ROOT/spikes/s1-zerocopy" && swiftc -O main.swift "${common[@]}" -framework IOSurface -o s1 )
echo "  built s1"
( cd "$ROOT/spikes/s6-isolation" && swiftc -O worker.swift "${common[@]}" -o s6-worker && swiftc -O host.swift -o s6-host )
echo "  built s6"
( cd "$ROOT/spikes/s7-waveform" && swiftc -O main.swift "${common[@]}" -o s7 )
echo "  built s7"
