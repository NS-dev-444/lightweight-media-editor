#!/usr/bin/env bash
# Run the app layer's pure-logic tests. See swift_selftest.swift for why this
# is a plain executable rather than an XCTest bundle.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT
swiftc -O \
  "$ROOT/app/macos/Sources/ExportPreset.swift" \
  "$ROOT/app/macos/Sources/CubeLUT.swift" \
  "$ROOT/tools/swift_selftest/main.swift" \
  -o "$OUT/selftest"
"$OUT/selftest"
