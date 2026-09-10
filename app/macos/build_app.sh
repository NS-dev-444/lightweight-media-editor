#!/usr/bin/env bash
# Build Editor.app.
#
# A direct swiftc build rather than SwiftPM: the app links the Rust staticlib
# and our own LGPL FFmpeg dylibs, and needs precise control over rpaths,
# entitlements and bundle layout. That is awkward in SwiftPM and simple here.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/tools/deployment.sh"
FF="$ROOT/build/ffmpeg-lgpl"
CORE="$ROOT/core"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$HERE/Editor.app"
# The swiftc invocation names Sources/ relatively, so run from the script's
# own directory rather than from wherever the caller happened to be.
cd "$HERE"

export FFMPEG_INCLUDE_DIR="$FF/include" FFMPEG_LIBS_DIR="$FF/lib" FFMPEG_LINK_MODE=dynamic
# On-device transcription (§8). Optional: without it the core still builds and
# the app reports transcription unavailable rather than offering a dead button.
if [[ -f "$ROOT/build/whisper/include/whisper.h" ]]; then
  export WHISPER_DIR="$ROOT/build/whisper"
fi
echo "==> building core"
( cd "$CORE" && cargo build --release -p mediacore-media >/dev/null )

echo "==> assembling bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

# The plist floor is substituted from the same variable the compiler gets,
# so the two cannot disagree.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>Editor</string>
  <key>CFBundleIdentifier</key><string>dev.mediacore.editor</string>
  <key>CFBundleName</key><string>Editor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1</string>
  <key>LSMinimumSystemVersion</key><string>$MACOS_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- §3's voiceover recording. macOS refuses microphone access outright when
       this string is missing, and the refusal looks like a bug in the app. -->
  <key>NSMicrophoneUsageDescription</key>
  <string>Editor uses the microphone to record voiceovers onto your timeline.</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>

  <!-- §37: Finder has to know what this app opens, or "Open With" and
       drag-onto-the-icon do nothing. The project type is declared as ours;
       media types are system types we open as a Viewer, so we never claim to
       be the default handler for every video on the machine. -->
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Editor Project</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Owner</string>
      <key>LSItemContentTypes</key><array><string>dev.mediacore.project</string></array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Media</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.movie</string>
        <string>public.audio</string>
        <string>public.image</string>
      </array>
    </dict>
  </array>
  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>dev.mediacore.project</string>
      <key>UTTypeDescription</key><string>Editor Project</string>
      <key>UTTypeConformsTo</key><array><string>public.data</string></array>
      <key>UTTypeTagSpecification</key>
      <dict><key>public.filename-extension</key><array><string>mcproj</string></array></dict>
    </dict>
  </array>
</dict></plist>
PLIST

# Ship the FFmpeg dylibs inside the bundle. They carry @rpath install names
# (set by third_party/ffmpeg/build.sh) precisely so this works, and LGPL
# requires dynamic linking with the libraries unrenamed.
cp "$FF"/lib/*.dylib "$APP/Contents/Frameworks/" 2>/dev/null || true

# The bundled Whisper model (O-18). 57 MB, multilingual, MIT-licensed weights —
# see tools/fetch_models.sh for why this size and why bundled rather than
# downloaded on first run.
if [[ -f "$ROOT/build/models/ggml-base-q5_1.bin" ]]; then
  cp "$ROOT/build/models/ggml-base-q5_1.bin" "$APP/Contents/Resources/"
  echo "    bundled transcription model ($(du -h "$ROOT/build/models/ggml-base-q5_1.bin" | cut -f1))"
else
  echo "    NO transcription model — run tools/fetch_models.sh"
fi

echo "==> compiling swift"
swiftc -O -target "arm64-apple-macos$MACOS_DEPLOYMENT_TARGET" \
  -import-objc-header "$CORE/mediacore-media/include/mediacore.h" \
  -parse-as-library Sources/*.swift \
  -L "$CORE/target/release" -lmediacore \
  ${WHISPER_DIR:+-L "$WHISPER_DIR/lib" -lwhisper -lggml -lggml-base -lggml-cpu -lggml-metal -lggml-blas -lc++} \
  ${WHISPER_DIR:+-framework Accelerate} \
  -L "$FF/lib" -lavcodec -lavformat -lavutil -lswresample -lswscale \
  -Xlinker -rpath -Xlinker @executable_path/../Frameworks \
  -framework SwiftUI -framework AppKit -framework Metal -framework MetalKit \
  -framework AVFoundation -framework CoreAudio \
  -framework CoreVideo -framework CoreMedia -framework VideoToolbox \
  -framework CoreFoundation -framework QuartzCore -framework IOSurface \
  -o "$APP/Contents/MacOS/Editor"

# Acknowledgements. Generated from what is actually in the bundle, and the
# build FAILS if anything ships without its licence text — MIT and LGPL both
# require these notices to accompany the software.
echo "==> acknowledgements"
python3 "$ROOT/tools/gather_licences.py" "$APP/Contents/Resources/Acknowledgements.txt" \
  || { echo "    FAILED: something ships unattributed"; exit 1; }

echo "==> signing"
# Library Validation requires every loaded dylib to share the host's TEAM ID.
# AD-11's plan holds for release: we build the FFmpeg dylibs ourselves, so a
# Developer ID signature on all of them satisfies Library Validation and
# `disable-library-validation` is never needed.
#
# But AD-HOC signatures carry NO Team ID, so under the hardened runtime they can
# never match and the app dies at launch with "different Team IDs". Local dev
# therefore signs ad-hoc WITHOUT the hardened runtime; release signs everything
# with the real Developer ID and turns it back on.
for f in "$APP"/Contents/Frameworks/*.dylib; do codesign --force --sign - "$f" >/dev/null 2>&1 || true; done
if [[ -n "${DEVELOPER_ID:-}" ]]; then
  for f in "$APP"/Contents/Frameworks/*.dylib; do
    codesign --force --sign "$DEVELOPER_ID" --options runtime --timestamp "$f"
  done
  codesign --force --sign "$DEVELOPER_ID" --options runtime --timestamp "$APP"
  echo "    signed with Developer ID, hardened runtime on"
else
  codesign --force --sign - "$APP" >/dev/null 2>&1
  echo "    ad-hoc, hardened runtime OFF (set DEVELOPER_ID for a release build)"
fi

echo "built: $APP"
