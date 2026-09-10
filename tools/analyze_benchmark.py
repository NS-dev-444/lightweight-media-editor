#!/usr/bin/env python3
"""S4 analysis — turn the VMAF grid into the number that sets export presets.

The question is NOT "what VMAF does each encoder get". It is:
  to match libx264 -preset medium at quality Q, how much MORE bitrate does
  the hardware encoder need?
That ratio sets every preset bitrate and quantifies RISK_REGISTER.md R-09.
"""
import csv, sys
from collections import defaultdict

path = sys.argv[1] if len(sys.argv) > 1 else "build/benchmark/results.csv"
rows = [r for r in csv.DictReader(open(path)) if r["vmaf"] not in ("NA", "")]
data = defaultdict(dict)          # (source, encoder) -> {bitrate: vmaf}
size = defaultdict(dict)
for r in rows:
    k = (r["source"], r["encoder"])
    data[k][int(r["bitrate_kbps"])] = float(r["vmaf"])
    size[k][int(r["bitrate_kbps"])] = int(r["output_bytes"])

sources = sorted({s for s, _ in data})

def bitrate_for_vmaf(curve, target):
    """Interpolate the bitrate needed to reach `target` VMAF. None if off-curve."""
    pts = sorted(curve.items())
    for (b0, v0), (b1, v1) in zip(pts, pts[1:]):
        if v0 <= target <= v1:
            if v1 == v0:
                return b0
            return b0 + (b1 - b0) * (target - v0) / (v1 - v0)
    if target <= pts[0][1]:
        return pts[0][0]
    return None                    # target above the highest measured point

print("=" * 78)
print("VMAF BY BITRATE")
print("=" * 78)
for s in sources:
    encs = [e for (ss, e) in data if ss == s]
    brs = sorted({b for e in encs for b in data[(s, e)]})
    print(f"\n{s}")
    print("  " + "encoder".ljust(22) + "".join(f"{b:>10}k" for b in brs))
    for e in ("libx264", "h264_videotoolbox", "hevc_videotoolbox"):
        if (s, e) not in data:
            continue
        cells = "".join(f"{data[(s,e)].get(b, float('nan')):>11.2f}" for b in brs)
        print("  " + e.ljust(22) + cells)

print("\n" + "=" * 78)
print("BITRATE PENALTY — extra bitrate needed to match libx264 -preset medium")
print("=" * 78)
overall = defaultdict(list)
for s in sources:
    ref = data.get((s, "libx264"))
    if not ref:
        continue
    print(f"\n{s}")
    for target_br, target_v in sorted(ref.items()):
        line = f"  match libx264 @{target_br}k (VMAF {target_v:.2f}):"
        parts = []
        for e in ("h264_videotoolbox", "hevc_videotoolbox"):
            curve = data.get((s, e))
            if not curve:
                continue
            need = bitrate_for_vmaf(curve, target_v)
            if need is None:
                hi = max(curve); parts.append(f"{e.split('_')[0]}: cannot reach (max VMAF {curve[hi]:.2f} @{hi}k)")
            else:
                ratio = need / target_br
                parts.append(f"{e.split('_')[0]}: {need:>7.0f}k ({ratio:.2f}x)")
                overall[e].append(ratio)
        print(line + "  " + " | ".join(parts))

print("\n" + "=" * 78)
print("HEADLINE")
print("=" * 78)
for e, ratios in overall.items():
    if ratios:
        r = sorted(ratios)
        med = r[len(r)//2]
        print(f"  {e:<20} median {med:.2f}x bitrate to match libx264 "
              f"(range {min(r):.2f}-{max(r):.2f}x, n={len(r)})")
print("\n  Encode speed (mean seconds per 5s clip):")
spd = defaultdict(list)
for r in rows:
    spd[r["encoder"]].append(float(r["encode_seconds"]))
for e, v in spd.items():
    print(f"    {e:<20} {sum(v)/len(v):.2f}s")
