import AppKit
import Foundation

// S2 — shared scene. Both renderers draw EXACTLY this, with identical culling,
// so the measurement isolates framework overhead rather than drawing strategy.

struct Clip {
    let start: Double          // seconds
    let duration: Double
    let track: Int
    let hue: CGFloat
    let peaks: [Float]         // waveform envelope, one per ~2px at 1x zoom
    let title: String
}

enum Scene {
    static let trackCount = 4
    static let clipCount = 200
    static let trackHeight: CGFloat = 78
    static let headerWidth: CGFloat = 0

    static let clips: [Clip] = {
        var rng = SystemRandomNumberGenerator()
        var out: [Clip] = []
        var cursor = [Double](repeating: 0, count: trackCount)
        for i in 0..<clipCount {
            let track = i % trackCount
            let dur = Double.random(in: 2.0...9.0, using: &rng)
            let gap = Double.random(in: 0.1...0.8, using: &rng)
            let start = cursor[track] + gap
            cursor[track] = start + dur
            // ~200 peaks/sec of audio — realistic envelope density
            let n = max(64, Int(dur * 200))
            var peaks = [Float](repeating: 0, count: n)
            var phase: Float = Float.random(in: 0..<6.28, using: &rng)
            for j in 0..<n {
                phase += 0.05
                let env = abs(sin(phase * 0.7)) * 0.7 + Float.random(in: 0...0.3, using: &rng)
                peaks[j] = min(1.0, env)
            }
            out.append(Clip(start: start, duration: dur, track: track,
                            hue: CGFloat(i % 12) / 12.0, peaks: peaks,
                            title: "clip_\(String(format: "%03d", i))"))
        }
        return out
    }()

    static let totalDuration: Double = clips.map { $0.start + $0.duration }.max() ?? 60

    // A small pool of thumbnails, generated once, drawn many times per frame —
    // exactly how a timeline thumbnail strip behaves.
    static let thumbnails: [CGImage] = {
        (0..<8).map { i in
            let w = 160, h = 90
            let cs = CGColorSpaceCreateDeviceRGB()
            let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8,
                                bytesPerRow: 0, space: cs,
                                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            let g = CGGradient(colorsSpace: cs, colors: [
                NSColor(hue: CGFloat(i)/8.0, saturation: 0.6, brightness: 0.8, alpha: 1).cgColor,
                NSColor(hue: CGFloat(i)/8.0, saturation: 0.9, brightness: 0.25, alpha: 1).cgColor
            ] as CFArray, locations: [0, 1])!
            ctx.drawLinearGradient(g, start: .zero, end: CGPoint(x: w, y: h), options: [])
            return ctx.makeImage()!
        }
    }()
}

/// Viewport state animated by the harness. Identical for both renderers.
struct Viewport {
    var scrollSeconds: Double = 0
    var pixelsPerSecond: Double = 80

    func xFor(_ t: Double) -> CGFloat { CGFloat((t - scrollSeconds) * pixelsPerSecond) }
    func visibleRange(width: CGFloat) -> ClosedRange<Double> {
        scrollSeconds...(scrollSeconds + Double(width) / pixelsPerSecond)
    }
}

/// Frame-time statistics. Reported as percentiles because a mean hides drops.
struct Stats {
    let samples: [Double]
    var p50: Double { pct(0.50) }
    var p95: Double { pct(0.95) }
    var p99: Double { pct(0.99) }
    var max: Double { samples.max() ?? 0 }
    var mean: Double { samples.reduce(0,+) / Double(samples.count) }
    var over60: Int { samples.filter { $0 > 16.67 }.count }   // misses 60 fps
    var over120: Int { samples.filter { $0 > 8.33 }.count }   // misses 120 fps
    private func pct(_ p: Double) -> Double {
        let s = samples.sorted()
        return s[Swift.min(s.count-1, Swift.max(0, Int(Double(s.count-1) * p)))]
    }
    func report(_ label: String) {
        print(String(format: "  %-26s p50 %6.2f  p95 %6.2f  p99 %6.2f  max %7.2f  mean %6.2f ms",
                     (label as NSString).utf8String!, p50, p95, p99, max, mean))
        print(String(format: "  %-26s frames >16.67ms: %4d/%d (%.1f%%)   >8.33ms: %4d/%d (%.1f%%)",
                     ("" as NSString).utf8String!, over60, samples.count,
                     100.0*Double(over60)/Double(samples.count),
                     over120, samples.count, 100.0*Double(over120)/Double(samples.count)))
    }
}
