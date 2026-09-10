#!/usr/bin/env bash
# Everything that must be true before a change is good. ONE script, run by
# developers and by CI, so the two cannot disagree about what "passing" means.
#
# Ordered cheapest-first: a formatting slip should not wait behind a
# transcription run to be reported.
#
# Exits non-zero on the first failure unless --all is given, which runs
# everything and reports at the end — better when you want the full picture.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
source "$ROOT/tools/deployment.sh"

ALL=0
[[ "${1:-}" == "--all" ]] && ALL=1

FAILED=()
PASSED=0
SKIPPED=0

bold() { printf "\033[1m%s\033[0m\n" "$1"; }
pass() { printf "  \033[32m✓\033[0m %s\n" "$1"; PASSED=$((PASSED+1)); }
skip() { printf "  \033[33m–\033[0m %s \033[2m(%s)\033[0m\n" "$1" "$2"; SKIPPED=$((SKIPPED+1)); }
fail() {
  printf "  \033[31m✗\033[0m %s\n" "$1"
  [[ -n "${2:-}" ]] && printf "%s\n" "$2" | sed 's/^/      /' | head -25
  FAILED+=("$1")
  [[ $ALL -eq 0 ]] && finish
}

finish() {
  echo
  if [[ ${#FAILED[@]} -eq 0 ]]; then
    printf "\033[32m%s checks passed\033[0m" "$PASSED"
    [[ $SKIPPED -gt 0 ]] && printf ", \033[33m%s skipped\033[0m" "$SKIPPED"
    echo
    exit 0
  fi
  printf "\033[31m%s FAILED\033[0m (%s passed)\n" "${#FAILED[@]}" "$PASSED"
  for f in "${FAILED[@]}"; do echo "  - $f"; done
  exit 1
}

have_ffmpeg_build() { [[ -f "$ROOT/build/ffmpeg-lgpl/include/libavcodec/avcodec.h" ]]; }
have_whisper()      { [[ -f "$ROOT/build/whisper/include/whisper.h" ]]; }

export FFMPEG_INCLUDE_DIR="$ROOT/build/ffmpeg-lgpl/include"
export FFMPEG_LIBS_DIR="$ROOT/build/ffmpeg-lgpl/lib"
export FFMPEG_LINK_MODE=dynamic
have_whisper && export WHISPER_DIR="$ROOT/build/whisper"

# ---------------------------------------------------------------- 1. the core

bold "Core"

out=$(cd core && cargo build --all-targets 2>&1)
if echo "$out" | grep -q "^error"; then
  fail "mediacore-model builds" "$(echo "$out" | grep -A3 '^error' | head -20)"
elif echo "$out" | grep -q "^warning"; then
  # §46 Rule 4: zero warnings. A warning today is a bug next month.
  fail "core builds without warnings" "$(echo "$out" | grep -A3 '^warning' | head -20)"
else
  pass "core builds, no warnings"
fi

out=$(cd core && cargo test -p mediacore-model 2>&1)
if echo "$out" | grep -qE "FAILED|^error"; then
  fail "model tests" "$(echo "$out" | grep -B2 -A6 'FAILED\|panicked' | head -25)"
else
  n=$(echo "$out" | grep -oE 'ok\. [0-9]+' | grep -oE '[0-9]+' | awk '{s+=$1} END {print s}')
  pass "model tests (${n:-0})"
fi

if have_ffmpeg_build; then
  out=$(cd core && cargo test -p mediacore-media 2>&1)
  if echo "$out" | grep -qE "FAILED|^error\[|^error:"; then
    fail "media tests" "$(echo "$out" | grep -B2 -A6 'FAILED\|panicked' | head -25)"
  else
    n=$(echo "$out" | grep -oE 'ok\. [0-9]+' | grep -oE '[0-9]+' | awk '{s+=$1} END {print s}')
    pass "media tests (${n:-0})"
  fi
else
  skip "media tests" "no FFmpeg build — run third_party/ffmpeg/build.sh"
fi

# The media crate must ALSO build without whisper: transcription is optional,
# and the Windows bring-up depends on being able to add one dependency at a time.
#
# Guarded on FFmpeg too. Without it this check PASSED from a cached artifact
# while the headers it needs were absent — a check that reports success when its
# inputs are missing is worse than no check.
if have_whisper && have_ffmpeg_build; then
  out=$(cd core && env -u WHISPER_DIR cargo build -p mediacore-media 2>&1)
  if echo "$out" | grep -qE "^error|^warning"; then
    fail "media crate builds WITHOUT whisper" "$(echo "$out" | grep -A3 '^error\|^warning' | head -20)"
  else
    pass "media crate builds without whisper too"
  fi
fi

# ---------------------------------------------------------------- 2. §22

bold "Rules"

# §22: no panicking in library code. Test modules are exempt — they are
# supposed to fail loudly.
#
# The exemption is "everything after the first #[cfg(test)] in a file", which is
# a heuristic: it assumes test modules sit at the end, as they do here. It can
# therefore miss a panic in real code placed BELOW a test module. Tightening it
# means parsing Rust, which is not worth it for a lint — but the limitation is
# stated rather than left to be discovered.
offenders=$(cd core && for f in */src/**/*.rs */src/*.rs; do
  [[ -f "$f" ]] || continue
  awk -v file="$f" '
    /^#\[cfg\(test\)\]/ { intest=1 }
    intest && /^mod |^pub mod / { inmod=1 }
    { if (!intest && (/\.unwrap\(\)/ || /\.expect\(/ || /panic!/) && !/unwrap_or/ && !/\/\// ) print file ":" NR ": " $0 }
  ' "$f"
done | head -10)
if [[ -n "$offenders" ]]; then
  fail "§22: no unwrap/expect/panic in library code" "$offenders"
else
  pass "§22: no unwrap/expect/panic in library code"
fi

# R-21: the model crate must keep cross-compiling for Windows, from any machine.
if rustup target list --installed 2>/dev/null | grep -q x86_64-pc-windows-msvc; then
  out=$(cd core && cargo check -p mediacore-model --target x86_64-pc-windows-msvc 2>&1)
  if echo "$out" | grep -q "^error"; then
    fail "R-21: model cross-compiles for Windows" "$(echo "$out" | grep -A3 '^error' | head -15)"
  else
    pass "R-21: model cross-compiles for Windows"
  fi
else
  skip "R-21: Windows cross-check" "rustup target add x86_64-pc-windows-msvc"
fi

# ---------------------------------------------------------------- 3. licence

bold "Licensing"

# R-05 is the likeliest licensing failure on this project: the GPL Homebrew
# FFmpeg is on the default PATH, and linking it would relicense the product.
if have_ffmpeg_build; then
  out=$(python3 tools/license_gate.py --prefix build/ffmpeg-lgpl 2>&1)
  if echo "$out" | grep -q "PASS"; then
    pass "R-05: shipped FFmpeg is LGPL-clean"
  else
    fail "R-05: shipped FFmpeg is LGPL-clean" "$out"
  fi
else
  skip "R-05: licence gate" "no FFmpeg build"
fi

# whisper.cpp is MIT and the build script checks that. Verify the checker is
# actually looking at something (DEPENDENCY_AND_LICENSE_AUDIT.md §6b).
if [[ -f build/whisper-src/LICENSE ]]; then
  if grep -q "MIT License" build/whisper-src/LICENSE; then
    pass "whisper.cpp is still MIT"
  else
    fail "whisper.cpp is still MIT" "LICENSE no longer says MIT — see audit §6b"
  fi
fi

# ---------------------------------------------------------------- 4. the app

bold "App"

if ! command -v swiftc >/dev/null 2>&1; then
  skip "app builds" "no swiftc"
elif ! have_ffmpeg_build; then
  skip "app builds" "no FFmpeg build"
else
  out=$(app/macos/build_app.sh 2>&1)
  if echo "$out" | grep -q "error:"; then
    fail "app builds" "$(echo "$out" | grep -B1 -A3 'error:' | head -20)"
  elif echo "$out" | grep -q "warning:"; then
    fail "app builds without warnings" "$(echo "$out" | grep -B1 -A3 'warning:' | head -20)"
  else
    pass "app builds, no warnings"
  fi

  out=$(tools/swift_selftest.sh 2>&1)
  if echo "$out" | grep -q "FAILED"; then
    fail "app self-tests" "$(echo "$out" | grep -B1 -A2 FAIL | head -20)"
  else
    pass "app self-tests ($(echo "$out" | grep -oE '[0-9]+ checks passed' || echo '?'))"
  fi

  # The bug this exists for: Info.plist claimed macOS 14 while every binary was
  # built for 26. Each piece was individually correct; only the combination was
  # wrong, and nothing warned. See tools/deployment.sh.
  APP="$ROOT/app/macos/Editor.app"
  declared=$(plutil -extract LSMinimumSystemVersion raw "$APP/Contents/Info.plist" 2>/dev/null)
  problems=""
  [[ "$declared" != "$MACOS_DEPLOYMENT_TARGET" ]] &&
    problems+="Info.plist says $declared, expected $MACOS_DEPLOYMENT_TARGET"$'\n'
  for bin in "$APP/Contents/MacOS/Editor" "$APP"/Contents/Frameworks/*.dylib; do
    [[ -f "$bin" ]] || continue
    minos=$(otool -l "$bin" 2>/dev/null | awk '/LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; exit}')
    [[ -z "$minos" ]] && continue
    if [[ "$minos" != "$MACOS_DEPLOYMENT_TARGET" ]]; then
      problems+="$(basename "$bin") is built for $minos, expected $MACOS_DEPLOYMENT_TARGET"$'\n'
    fi
  done
  if [[ -n "$problems" ]]; then
    fail "everything targets macOS $MACOS_DEPLOYMENT_TARGET" "$problems"
  else
    pass "everything targets macOS $MACOS_DEPLOYMENT_TARGET"
  fi

  # Licences are a legal obligation, and one that quietly stops being met the
  # moment somebody adds a dependency. Assert on the SHIPPED file, and that it
  # actually names each thing in the bundle.
  ACK="$APP/Contents/Resources/Acknowledgements.txt"
  if [[ ! -f "$ACK" ]]; then
    fail "acknowledgements ship with the app" "$ACK is missing"
  else
    absent=""
    grep -q "GNU LESSER GENERAL PUBLIC LICENSE" "$ACK" || absent+="the LGPL text"$'\n'
    ls "$APP"/Contents/Frameworks/libmp3lame*.dylib >/dev/null 2>&1 &&
      ! grep -q "^LAME " "$ACK" && absent+="LAME"$'\n'
    if [[ -f "$APP/Contents/Resources/ggml-base-q5_1.bin" ]]; then
      grep -q "whisper.cpp" "$ACK" || absent+="whisper.cpp"$'\n'
      grep -q "Copyright (c) 2022 OpenAI" "$ACK" || absent+="the Whisper model weights"$'\n'
    fi
    # Every crate cargo says ships must be named.
    while read -r crate; do
      [[ -z "$crate" ]] && continue
      grep -q "^$crate " "$ACK" || absent+="crate $crate"$'\n'
    done < <(cd core && cargo metadata --format-version 1 \
               --filter-platform aarch64-apple-darwin 2>/dev/null \
             | python3 -c "
import json,sys
m=json.load(sys.stdin)
pkgs={p['id']:p for p in m['packages']}
nodes={n['id']:n for n in m['resolve']['nodes']}
seen=set()
def walk(i):
    if i in seen: return
    seen.add(i)
    for d in nodes.get(i,{}).get('deps',[]):
        if None in {k.get('kind') for k in d.get('dep_kinds',[])}: walk(d['pkg'])
for p in m['packages']:
    if p['name'].startswith('mediacore'): walk(p['id'])
for i in seen:
    n=pkgs[i]['name']
    if not n.startswith('mediacore'): print(n)
" 2>/dev/null)
    if [[ -n "$absent" ]]; then
      fail "acknowledgements name everything that ships" "$absent"
    else
      pass "acknowledgements name everything that ships"
    fi
  fi

  # A shipped bundle must carry its own libraries, or it only runs here.
  if [[ -d "$APP/Contents/Frameworks" ]] && ls "$APP"/Contents/Frameworks/*.dylib >/dev/null 2>&1; then
    strays=$(otool -L "$APP/Contents/MacOS/Editor" | grep -E "^\s+/(opt|usr/local)/" | head -5)
    if [[ -n "$strays" ]]; then
      fail "app links nothing from Homebrew" "$strays"
    else
      pass "app links nothing from Homebrew (R-05)"
    fi
  fi
fi

finish
