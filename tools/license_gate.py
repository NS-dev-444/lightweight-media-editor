#!/usr/bin/env python3
"""
FFmpeg licence gate.

Implements the build-blocking gates in DEPENDENCY_AND_LICENSE_AUDIT.md §7 and
mitigates RISK_REGISTER.md R-05 (shipping a GPL or nonfree FFmpeg by accident).

The gate inspects the BUILT ARTIFACT, not the build script, because a build
script can be edited and a developer's PATH can point at a GPL Homebrew build.
The release checklist requires verification "from the shipped binary, not the
build script" -- this is that check.

Usage:
    license_gate.py --prefix build/ffmpeg-lgpl      # check an install prefix
    license_gate.py --binary /opt/homebrew/bin/ffmpeg   # check an executable

Exit codes:  0 = pass, 1 = licence violation, 2 = could not inspect
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

# --- policy ---------------------------------------------------------------

DENIED_FLAGS = {
    "--enable-gpl": "makes all of FFmpeg GPL; incompatible with a proprietary product",
    "--enable-nonfree": "makes the binary NON-REDISTRIBUTABLE",
    "--enable-version3": "pulls in LGPLv3 terms; not cleared for this product",
}

DENIED_COMPONENTS = {
    "libx264": "GPL",
    "libx265": "GPL",
    "libfdk-aac": "nonfree",
    "libxvid": "GPL",
    "librubberband": "GPL",
    "libvidstab": "GPL",
    "libzvbi": "GPL",
    "libsmbclient": "GPL",
    "libcdio": "GPL",
    "libcaca": "GPL",
    "frei0r": "GPL",
    "openssl": "GPLv2-incompatible when linked with GPL builds; use platform TLS",
}

# Libraries the product actually loads. Anything else in the prefix is noise.
EXPECTED_LIBS = ["avcodec", "avformat", "avfilter", "avutil", "swscale", "swresample"]

CONFIG_RE = re.compile(rb"--prefix=[ -~]{0,4000}")


# --- extraction -----------------------------------------------------------

def config_from_binary(path: Path) -> str:
    """Read the configuration string by running `<ffmpeg> -version`."""
    try:
        out = subprocess.run(
            [str(path), "-version"], capture_output=True, text=True, timeout=30
        ).stdout
    except (OSError, subprocess.SubprocessError) as exc:
        sys.exit(f"GATE ERROR: could not run {path}: {exc}")
    for line in out.splitlines():
        if line.strip().startswith("configuration:"):
            return line.split("configuration:", 1)[1].strip()
    sys.exit(f"GATE ERROR: no configuration string in `{path} -version` output")


def config_from_prefix(prefix: Path) -> str:
    """Recover the configuration string embedded in libavutil."""
    # bin/ as well as lib/: a MinGW build puts the real DLL in bin/
    # (avutil-59.dll) and only an import stub in lib/ (libavutil.dll.a). The
    # stub carries no configuration string, so searching lib/ alone finds
    # nothing on Windows and falls through to a slow walk of the whole prefix.
    candidates = sorted(
        p for pat in ("libavutil*.dylib", "libavutil*.so*", "avutil*.dll", "libavutil*.dll")
        for d in (prefix / "lib", prefix / "bin")
        for p in d.glob(pat)
    )
    if not candidates:
        candidates = sorted(
            p for pat in ("libavutil*.dylib", "libavutil*.so*", "avutil*.dll", "libavutil*.dll")
            for p in prefix.rglob(pat)
        )
    if not candidates:
        sys.exit(f"GATE ERROR: no libavutil found under {prefix}")

    blob = candidates[0].read_bytes()
    matches = CONFIG_RE.findall(blob)
    if not matches:
        sys.exit(f"GATE ERROR: no configuration string embedded in {candidates[0].name}")
    return max(matches, key=len).decode("ascii", errors="replace")


# --- checks ---------------------------------------------------------------

def check_linkage(prefix: Path) -> list[str]:
    """LGPL requires dynamic linking and unrenamed libraries."""
    problems = []
    libdir = prefix / "lib"
    if not libdir.is_dir():
        return [f"no lib/ directory under {prefix}"]

    for name in EXPECTED_LIBS:
        shared = list(libdir.glob(f"lib{name}*.dylib")) + \
                 list(libdir.glob(f"lib{name}*.so*")) + \
                 list(libdir.glob(f"{name}*.dll"))
        static = list(libdir.glob(f"lib{name}*.a"))
        if not shared:
            problems.append(f"lib{name}: no shared library found (LGPL requires dynamic linking)")
        if static and not shared:
            problems.append(f"lib{name}: static-only build violates the LGPL dynamic-linking strategy")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description="FFmpeg licence gate")
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--prefix", type=Path, help="FFmpeg install prefix to inspect")
    src.add_argument("--binary", type=Path, help="ffmpeg executable to inspect")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    if args.binary:
        target, config = str(args.binary), config_from_binary(args.binary)
    else:
        target, config = str(args.prefix), config_from_prefix(args.prefix)

    violations: list[str] = []

    for flag, why in DENIED_FLAGS.items():
        if flag in config:
            violations.append(f"DENIED FLAG  {flag}  -- {why}")

    for comp, lic in DENIED_COMPONENTS.items():
        if f"--enable-{comp}" in config:
            violations.append(f"DENIED COMPONENT  {comp}  -- {lic}")

    if args.prefix:
        violations.extend(f"LINKAGE  {p}" for p in check_linkage(args.prefix))

    if not args.quiet:
        print(f"target:  {target}")
        shown = config if len(config) <= 600 else config[:600] + " ..."
        print(f"config:  {shown}\n")

    if violations:
        print("\033[1;31mLICENCE GATE: FAIL\033[0m")
        for v in violations:
            print(f"  ✗ {v}")
        print(
            "\nThis artifact must not be linked into the product or shipped.\n"
            "See docs/DEPENDENCY_AND_LICENSE_AUDIT.md §3 and RISK_REGISTER.md R-05.\n"
            "Build the approved one with: third_party/ffmpeg/build.sh"
        )
        return 1

    print("\033[1;32mLICENCE GATE: PASS\033[0m")
    print("  ✓ no GPL flags        (--enable-gpl, --enable-version3)")
    print("  ✓ no nonfree flags    (--enable-nonfree)")
    print(f"  ✓ no denied components ({len(DENIED_COMPONENTS)} checked)")
    if args.prefix:
        print("  ✓ dynamically linked, libraries unrenamed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
