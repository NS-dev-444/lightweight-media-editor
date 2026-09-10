#!/usr/bin/env bash
#
# Reproducible LGPL-only FFmpeg build.
#
# Implements ARCHITECTURE_DECISION.md AD-11 and DEPENDENCY_AND_LICENSE_AUDIT.md §3.
# The product links ONLY against the output of this script. Never against a
# system, Homebrew, or developer-configured FFmpeg -- see RISK_REGISTER.md R-05.
#
# Usage:  ./build.sh [output-prefix]
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Pinned source. Changing any of these three lines is a licensing-relevant
# change and requires re-running the audit in DEPENDENCY_AND_LICENSE_AUDIT.md.
# ---------------------------------------------------------------------------
FFMPEG_VERSION="8.1.2"
FFMPEG_SHA256="464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c"
FFMPEG_GPG_FINGERPRINT="FCF986EA15E6E293A5644F10B4322F04D67658D8"

# LAME -- required for MP3 export (spec §4). LGPL v2 "or any later version"
# (verified from libmp3lame/lame.c and COPYING), so it is compatible with
# FFmpeg's LGPL v2.1+ and does NOT make this build GPL. MP3 patents expired.
LAME_VERSION="3.100"
LAME_SHA256="ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e"

# Version rationale (RISK_REGISTER.md R-11): 8.1.x is the newest series with a
# published Rust binding (rusty_ffmpeg 0.17.0+ffmpeg.8.1). ffmpeg-next targets
# 3.4-8.0 and is in maintenance mode, so this pin deliberately favours
# rusty_ffmpeg plus our own thin safe wrapper.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The supported macOS floor, in one place — see tools/deployment.sh for why.
# shellcheck source=/dev/null
source "$REPO_ROOT/tools/deployment.sh"
WORK_DIR="${REPO_ROOT}/build"

# A stamp of what the prefix was last built for. `make` relinks only the
# libraries whose objects changed, so changing the deployment target and
# re-running produced a prefix with libavcodec at 14.0 and libavutil at 26.0 —
# individually valid, collectively unshippable. Changing the floor now wipes
# the build rather than half-updating it.
DEPLOYMENT_STAMP="${WORK_DIR}/.deployment-target"
PREFIX="${1:-${WORK_DIR}/ffmpeg-lgpl}"
TARBALL="ffmpeg-${FFMPEG_VERSION}.tar.xz"
SRC_DIR="${WORK_DIR}/ffmpeg-${FFMPEG_VERSION}"

# ---------------------------------------------------------------------------
# Flags that must NEVER appear. Defence in depth: tools/license_gate.py checks
# the built artifact independently, because a build script can be edited.
# ---------------------------------------------------------------------------
DENIED_FLAGS=(
  "--enable-gpl"        # makes all of FFmpeg GPL
  "--enable-nonfree"    # makes the result NON-REDISTRIBUTABLE
  "--enable-version3"   # pulls in LGPLv3 terms
  "--enable-libx264"    # GPL
  "--enable-libx265"    # GPL
  "--enable-libfdk-aac" # nonfree
)

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

mkdir -p "${WORK_DIR}"

# Wipe a prefix built for a different macOS floor — see DEPLOYMENT_STAMP above.
if [[ -f "$DEPLOYMENT_STAMP" ]] && [[ "$(cat "$DEPLOYMENT_STAMP")" != "$MACOS_DEPLOYMENT_TARGET" ]]; then
  log "Deployment target changed ($(cat "$DEPLOYMENT_STAMP") -> $MACOS_DEPLOYMENT_TARGET); rebuilding from scratch"
  rm -rf "$PREFIX" "${WORK_DIR}/ffmpeg-${FFMPEG_VERSION}" "${WORK_DIR}/lame-${LAME_VERSION}"
fi
echo "$MACOS_DEPLOYMENT_TARGET" > "$DEPLOYMENT_STAMP"
cd "${WORK_DIR}"

# ---------------------------------------------------------------------------
# 1. Fetch and verify source
# ---------------------------------------------------------------------------
if [[ ! -f "${TARBALL}" ]]; then
  log "Downloading ${TARBALL}"
  curl -fsSL -O "https://ffmpeg.org/releases/${TARBALL}"
  curl -fsSL -O "https://ffmpeg.org/releases/${TARBALL}.asc"
fi

log "Verifying SHA256"
actual="$(shasum -a 256 "${TARBALL}" | awk '{print $1}')"
[[ "${actual}" == "${FFMPEG_SHA256}" ]] \
  || die "SHA256 mismatch. expected ${FFMPEG_SHA256}, got ${actual}"

if command -v gpg >/dev/null 2>&1 && [[ -f "${TARBALL}.asc" ]]; then
  # Import the release key if this machine has never seen it — a fresh CI
  # runner never has, and the first CI run failed here.
  #
  # This does NOT weaken the check. The fingerprint above is the trust anchor
  # and it is hardcoded; a key that does not carry it is refused regardless of
  # where the bytes came from. Fetching a key and then verifying its
  # fingerprint is exactly as strong as importing it by hand.
  if ! gpg --list-keys "${FFMPEG_GPG_FINGERPRINT}" >/dev/null 2>&1; then
    log "Importing the FFmpeg release key"
    if curl -fsSL --max-time 30 "https://ffmpeg.org/ffmpeg-devel.asc" -o "${WORK_DIR}/ffmpeg-key.asc"; then
      gpg --import "${WORK_DIR}/ffmpeg-key.asc" >/dev/null 2>&1 || true
    fi
    # Keyservers are flaky; the project's own site is the primary source and
    # this is the fallback rather than the other way round.
    if ! gpg --list-keys "${FFMPEG_GPG_FINGERPRINT}" >/dev/null 2>&1; then
      gpg --keyserver hkps://keyserver.ubuntu.com \
          --recv-keys "${FFMPEG_GPG_FINGERPRINT}" >/dev/null 2>&1 || true
    fi
    gpg --list-keys "${FFMPEG_GPG_FINGERPRINT}" >/dev/null 2>&1 \
      || die "Could not obtain the FFmpeg release key ${FFMPEG_GPG_FINGERPRINT}. \
Import it manually and re-run; do NOT remove this check."
  fi

  log "Verifying GPG signature against ${FFMPEG_GPG_FINGERPRINT}"
  if gpg --verify "${TARBALL}.asc" "${TARBALL}" 2>&1 | grep -q "Good signature"; then
    gpg --verify "${TARBALL}.asc" "${TARBALL}" 2>&1 \
      | grep -qi "${FFMPEG_GPG_FINGERPRINT// /}" \
      || die "Signature is good but the fingerprint does not match the pin"
    log "GPG signature verified"
  else
    die "GPG signature verification FAILED (import the key: gpg --recv-keys ${FFMPEG_GPG_FINGERPRINT})"
  fi
else
  log "WARNING: gpg unavailable; relying on SHA256 pin only"
fi

# ---------------------------------------------------------------------------
# 2. Unpack and apply patches
# ---------------------------------------------------------------------------
if [[ ! -d "${SRC_DIR}" ]]; then
  log "Unpacking"
  tar xf "${TARBALL}"
fi

shopt -s nullglob
patches=("${REPO_ROOT}/third_party/ffmpeg/patches/"*.patch)
shopt -u nullglob
if (( ${#patches[@]} )); then
  log "Applying ${#patches[@]} patch(es) -- these MUST be published with the source (LGPL)"
  for p in "${patches[@]}"; do
    ( cd "${SRC_DIR}" && patch -p1 --forward < "${p}" ) || die "patch failed: ${p}"
  done
fi

# ---------------------------------------------------------------------------
# 2b. Build LAME (LGPL) -- FFmpeg has no native MP3 encoder, and macOS no
#     longer exposes an AudioToolbox one. Without this, spec §4's MP3 export
#     targets are impossible. See docs/PHASE_1_RESULTS.md.
# ---------------------------------------------------------------------------
# The prefix is shared by LAME and FFmpeg, so it is cleaned exactly once, here,
# before either installs into it. (Cleaning it before `make install` instead
# would delete the LAME that FFmpeg has just linked against.)
if [[ "${CLEAN_PREFIX:-1}" == "1" && -d "${PREFIX}" ]]; then
  log "Cleaning ${PREFIX}"
  rm -rf "${PREFIX}"
fi

LAME_TARBALL="lame-${LAME_VERSION}.tar.gz"
LAME_SRC="${WORK_DIR}/lame-${LAME_VERSION}"

if [[ ! -f "${PREFIX}/lib/libmp3lame.dylib" && ! -f "${PREFIX}/lib/libmp3lame.so" ]]; then
  cd "${WORK_DIR}"
  if [[ ! -f "${LAME_TARBALL}" ]]; then
    log "Downloading ${LAME_TARBALL}"
    curl -fsSL -o "${LAME_TARBALL}" \
      "https://downloads.sourceforge.net/project/lame/lame/${LAME_VERSION}/${LAME_TARBALL}"
  fi
  log "Verifying LAME SHA256"
  lame_actual="$(shasum -a 256 "${LAME_TARBALL}" | awk '{print $1}')"
  [[ "${lame_actual}" == "${LAME_SHA256}" ]] \
    || die "LAME SHA256 mismatch. expected ${LAME_SHA256}, got ${lame_actual}"

  [[ -d "${LAME_SRC}" ]] || tar xzf "${LAME_TARBALL}"

  # LAME 3.100 exports lame_init_old in its symbol file but never defines it,
  # which breaks the shared-library link on modern clang. Removing the stale
  # export is a build fix, not a functional change.
  if grep -q "^lame_init_old$" "${LAME_SRC}/include/libmp3lame.sym" 2>/dev/null; then
    log "Removing stale lame_init_old export (LAME 3.100 build fix)"
    sed -i '' '/^lame_init_old$/d' "${LAME_SRC}/include/libmp3lame.sym"
  fi

  log "Building LAME"
  cd "${LAME_SRC}"
  ./configure --prefix="${PREFIX}" --enable-shared --disable-static \
      --disable-frontend \
      > "${WORK_DIR}/lame-configure.log" 2>&1 \
    || { tail -20 "${WORK_DIR}/lame-configure.log"; die "LAME configure failed"; }
  make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)" > "${WORK_DIR}/lame-make.log" 2>&1 \
    || { tail -20 "${WORK_DIR}/lame-make.log"; die "LAME build failed"; }
  make install > "${WORK_DIR}/lame-install.log" 2>&1 || die "LAME install failed"

  # LAME has no --install-name-dir; rewrite for .app bundling.
  if [[ "$(uname -s)" == "Darwin" ]]; then
    for dylib in "${PREFIX}"/lib/libmp3lame*.dylib; do
      [[ -f "${dylib}" && ! -L "${dylib}" ]] || continue
      install_name_tool -id "@rpath/$(basename "${dylib}")" "${dylib}"
    done
    log "Rewrote libmp3lame install name to @rpath"
  fi
fi

# ---------------------------------------------------------------------------
# 3. Configure -- LGPL only
# ---------------------------------------------------------------------------
CONFIGURE_FLAGS=(
  --prefix="${PREFIX}"
  --enable-shared             # LGPL: dynamic linking (ffmpeg.org/legal.html)
  --disable-static
  --disable-programs          # we ship libraries, not the ffmpeg CLI
  --disable-doc
  --disable-autodetect        # reproducibility: never silently absorb system libs
  --install-name-dir=@rpath   # required for bundling into a .app
  --enable-libmp3lame         # LGPL v2+. Required for spec §4 MP3 export.
  --extra-cflags="-I${PREFIX}/include -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
  --extra-ldflags="-L${PREFIX}/lib -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
)

case "$(uname -s)" in
  Darwin)
    CONFIGURE_FLAGS+=(--enable-videotoolbox --enable-audiotoolbox)
    [[ "$(uname -m)" == "arm64" ]] && CONFIGURE_FLAGS+=(--enable-neon)
    ;;
  MINGW*|MSYS*|CYGWIN*)
    CONFIGURE_FLAGS+=(--enable-d3d11va --enable-dxva2)
    # --enable-nvenc/--enable-qsv/--enable-amf are pending the VERIFY items in
    # DEPENDENCY_AND_LICENSE_AUDIT.md §3.3. Do not add them until cleared.
    ;;
esac

log "Self-checking configure flags against the deny list"
for denied in "${DENIED_FLAGS[@]}"; do
  for flag in "${CONFIGURE_FLAGS[@]}"; do
    [[ "${flag}" == "${denied}"* ]] && die "DENIED FLAG in build script: ${denied}"
  done
done

cd "${SRC_DIR}"
log "Configuring"
./configure "${CONFIGURE_FLAGS[@]}" > "${WORK_DIR}/configure.log" 2>&1 \
  || { tail -30 "${WORK_DIR}/configure.log"; die "configure failed (see ${WORK_DIR}/configure.log)"; }

# ---------------------------------------------------------------------------
# 4. Build and install
# ---------------------------------------------------------------------------
log "Building with $(sysctl -n hw.ncpu 2>/dev/null || echo 4) jobs"
make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)" > "${WORK_DIR}/make.log" 2>&1 \
  || { tail -30 "${WORK_DIR}/make.log"; die "build failed (see ${WORK_DIR}/make.log)"; }

log "Installing to ${PREFIX}"
make install > "${WORK_DIR}/install.log" 2>&1 || die "install failed"

# ---------------------------------------------------------------------------
# 5. Manifest -- the record the licence audit and release checklist consume
# ---------------------------------------------------------------------------
cat > "${PREFIX}/BUILD_MANIFEST.txt" <<MANIFEST
FFmpeg build manifest
=====================
version          ${FFMPEG_VERSION}
source sha256    ${FFMPEG_SHA256}
lame version     ${LAME_VERSION}
lame sha256      ${LAME_SHA256}
gpg fingerprint  ${FFMPEG_GPG_FINGERPRINT}
built            $(date -u +%Y-%m-%dT%H:%M:%SZ)
host             $(uname -srm)
patches applied  ${#patches[@]}
licence          LGPL v2.1+  (no GPL, no nonfree, no version3)
lame licence     LGPL v2 or later (verified from source headers + COPYING)

LGPL obligations (DEPENDENCY_AND_LICENSE_AUDIT.md §3.2):
  - ship ${TARBALL} and this build script alongside the binaries
  - link dynamically; do not rename the libraries
  - attribute FFmpeg/LGPLv2.1 in the about box and EULA
  - publish any patches in third_party/ffmpeg/patches/ as diffs

configure:
$(sed -n 's/^  configuration: *//p' "${WORK_DIR}/configure.log" 2>/dev/null || echo "  (see configure.log)")
MANIFEST

# The licence texts travel WITH the artifact, into the prefix.
#
# Not left in the unpacked source tree: CI caches the prefix and not the source,
# so on a cache hit the source tree does not exist and the acknowledgements
# generator would have had nothing to read. LGPL obliges us to ship these texts,
# and an obligation that depends on a build directory surviving is one that will
# eventually not be met.
log "Installing licence texts"
mkdir -p "${PREFIX}/share/licences"
cp "${SRC_DIR}/COPYING.LGPLv2.1" "${PREFIX}/share/licences/FFmpeg-LGPL-2.1.txt"
cp "${SRC_DIR}/LICENSE.md"       "${PREFIX}/share/licences/FFmpeg-LICENSE.md"
if [[ -f "${WORK_DIR}/lame-${LAME_VERSION}/COPYING" ]]; then
  cp "${WORK_DIR}/lame-${LAME_VERSION}/COPYING" "${PREFIX}/share/licences/LAME-LGPL-2.1.txt"
fi
# The version is recorded here too: an acknowledgement has to say WHICH version
# it covers, and LGPL §6 requires pointing at the corresponding source.
cat > "${PREFIX}/share/licences/VERSIONS.txt" <<VERS
FFmpeg ${FFMPEG_VERSION}
LAME ${LAME_VERSION}
VERS

log "Done. Prefix: ${PREFIX}"
log "Now run: tools/license_gate.py --prefix ${PREFIX}"
