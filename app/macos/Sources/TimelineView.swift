import SwiftUI

/// The timeline.
///
/// SwiftUI `Canvas`, chosen on evidence: spike S2 measured 3.29 ms p99 with the
/// entire 200-clip timeline on screen, and S2b measured 0.74 ms from model
/// mutation to pixels while dragging a 20-clip selection — both far inside a
/// 120 Hz budget. The Phase 0 plan assumed AppKit would be needed; it is not.
/// `Canvas` is immediate-mode and allocates nothing per clip, so the per-row
/// view cost that makes SwiftUI *lists* stutter does not apply here.
struct TimelineView: View {
    @ObservedObject var doc: EditorDocument

    private let trackHeight: CGFloat = 64
    private let rulerHeight: CGFloat = 24

    var body: some View {
        GeometryReader { geo in
            Canvas(rendersAsynchronously: false) { ctx, size in
                draw(ctx: &ctx, size: size)
            }
            .background(Color(white: 0.10))
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { v in handleDrag(v, size: geo.size) }
                    .onEnded { _ in doc.endDrag() }
            )
            // §22: drag media straight from Finder onto the timeline.
            .onDrop(of: ["public.file-url"], isTargeted: nil) { providers in
                doc.handleDrop(providers)
            }
        }
    }

    // ---- drawing -----------------------------------------------------------

    private func draw(ctx: inout GraphicsContext, size: CGSize) {
        drawRuler(&ctx, size)
        drawTracks(&ctx, size)
        drawClips(&ctx, size)
        drawPlayhead(&ctx, size)
    }

    private func x(for t: Int64) -> CGFloat {
        CGFloat((t - doc.scrollTicks).asSeconds * doc.pixelsPerSecond)
    }
    private func time(atX px: CGFloat) -> Int64 {
        doc.scrollTicks + ticks(seconds: Double(px) / doc.pixelsPerSecond)
    }

    private func drawRuler(_ ctx: inout GraphicsContext, _ size: CGSize) {
        ctx.fill(Path(CGRect(x: 0, y: 0, width: size.width, height: rulerHeight)),
                 with: .color(Color(white: 0.15)))
        // Pick a tick spacing that stays readable at any zoom.
        let candidates: [Double] = [0.04, 0.1, 0.5, 1, 2, 5, 10, 30, 60, 300, 600]
        let step = candidates.first { $0 * doc.pixelsPerSecond >= 60 } ?? 600
        var s = (doc.scrollTicks.asSeconds / step).rounded(.down) * step
        while true {
            let px = x(for: ticks(seconds: s))
            if px > size.width { break }
            if px >= 0 {
                ctx.stroke(Path { p in
                    p.move(to: CGPoint(x: px, y: rulerHeight - 6))
                    p.addLine(to: CGPoint(x: px, y: rulerHeight))
                }, with: .color(Color(white: 0.45)), lineWidth: 1)
                ctx.draw(Text(label(seconds: s)).font(.system(size: 9, design: .monospaced))
                            .foregroundColor(Color(white: 0.6)),
                         at: CGPoint(x: px + 3, y: 4), anchor: .topLeading)
            }
            s += step
        }
    }

    private func label(seconds: Double) -> String {
        let t = max(0, seconds)
        let m = Int(t) / 60, sec = Int(t) % 60
        return t < 60 ? String(format: "%.2gs", t) : String(format: "%d:%02d", m, sec)
    }

    private func drawTracks(_ ctx: inout GraphicsContext, _ size: CGSize) {
        for i in 0..<doc.trackCount {
            let y = rulerHeight + CGFloat(i) * trackHeight
            ctx.fill(Path(CGRect(x: 0, y: y, width: size.width, height: trackHeight - 1)),
                     with: .color(Color(white: i % 2 == 0 ? 0.13 : 0.115)))
        }
    }

    private func drawClips(_ ctx: inout GraphicsContext, _ size: CGSize) {
        for c in doc.clips {
            let cx = x(for: c.start_ticks)
            let cw = CGFloat(c.duration_ticks.asSeconds * doc.pixelsPerSecond)
            // Cull off-screen clips, exactly as S2 did — the measurement only
            // holds because the draw is proportional to what is visible.
            if cx + cw < 0 || cx > size.width { continue }

            let y = rulerHeight + CGFloat(c.track_index) * trackHeight + 3
            let rect = CGRect(x: cx, y: y, width: max(cw, 2), height: trackHeight - 8)
            let path = Path(roundedRect: rect, cornerRadius: 4)

            let base: Color = switch c.track_kind {
                case 0: Color(red: 0.18, green: 0.32, blue: 0.50)
                case 1: Color(red: 0.40, green: 0.28, blue: 0.48)
                default: Color(red: 0.18, green: 0.42, blue: 0.34)
            }
            ctx.fill(path, with: .color(c.muted != 0 ? base.opacity(0.35) : base))

            if c.selected != 0 {
                ctx.stroke(path, with: .color(.yellow), lineWidth: 2)
            } else {
                ctx.stroke(path, with: .color(.white.opacity(0.25)), lineWidth: 1)
            }
            if c.locked != 0 {
                ctx.fill(Path(roundedRect: rect.insetBy(dx: 2, dy: 2), cornerRadius: 3),
                         with: .color(.white.opacity(0.06)))
            }
            // Audio clips show their waveform (§15). Peaks are decimated to
            // roughly one per pixel — drawing every peak at a zoomed-out view
            // would cost far more than it shows.
            if c.track_kind == 2, let peaks = doc.waveforms[c.asset_id], peaks.count > 3 {
                let mid = rect.midY
                let amp = rect.height * 0.45
                let pairs = peaks.count / 2
                let step = max(1, Int(Double(pairs) / max(Double(cw), 1)))
                var wave = Path()
                var i = 0
                while i < pairs {
                    let px = rect.minX + cw * CGFloat(i) / CGFloat(pairs)
                    if px > rect.maxX { break }
                    let lo = CGFloat(peaks[i * 2]), hi = CGFloat(peaks[i * 2 + 1])
                    wave.move(to: CGPoint(x: px, y: mid - hi * amp))
                    wave.addLine(to: CGPoint(x: px, y: mid - lo * amp))
                    i += step
                }
                ctx.stroke(wave, with: .color(.white.opacity(0.55)), lineWidth: 1)
            }

            if cw > 46 {
                // Names are as long as the user's filenames, so the label is
                // drawn in a clipped layer: a long name stops at the clip's
                // edge instead of running across its neighbours.
                ctx.drawLayer { layer in
                    layer.clip(to: Path(rect.insetBy(dx: 3, dy: 1)))
                    layer.draw(Text(verbatim: doc.clipLabels[c.clip_id] ?? "clip \(c.clip_id)")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(.white.opacity(0.9)),
                               at: CGPoint(x: rect.minX + 5, y: rect.minY + 3), anchor: .topLeading)
                }
            }
        }
    }

    private func drawPlayhead(_ ctx: inout GraphicsContext, _ size: CGSize) {
        let px = x(for: doc.playhead)
        guard px >= 0, px <= size.width else { return }
        ctx.stroke(Path { p in
            p.move(to: CGPoint(x: px, y: 0))
            p.addLine(to: CGPoint(x: px, y: size.height))
        }, with: .color(.red), lineWidth: 1)
        ctx.fill(Path { p in
            p.move(to: CGPoint(x: px - 5, y: 0))
            p.addLine(to: CGPoint(x: px + 5, y: 0))
            p.addLine(to: CGPoint(x: px, y: 9))
            p.closeSubpath()
        }, with: .color(.red))
    }

    // ---- interaction -------------------------------------------------------

    private func handleDrag(_ v: DragGesture.Value, size: CGSize) {
        let t = max(0, time(atX: v.location.x))
        if v.location.y < rulerHeight {
            doc.scrub(to: t)                       // dragging the ruler scrubs
            return
        }
        let trackIndex = Int((v.location.y - rulerHeight) / trackHeight)
        doc.drag(to: t, trackIndex: trackIndex, isStart: v.translation == .zero)
    }
}
