import AppKit
import SwiftUI

// Shared drawing description so both renderers draw pixel-identical content.
struct ClipDraw {
    let rect: CGRect
    let hue: CGFloat
    let peaks: ArraySlice<Float>
    let title: String
    let thumbIndex: Int
    let thumbCount: Int
}

enum Layout {
    /// Culls to the viewport and produces draw commands. Shared by both renderers,
    /// so neither gets an unfair advantage from a different culling strategy.
    static func visibleClips(_ vp: Viewport, size: CGSize) -> [ClipDraw] {
        let range = vp.visibleRange(width: size.width)
        var out: [ClipDraw] = []
        out.reserveCapacity(64)
        for (i, c) in Scene.clips.enumerated() {
            let end = c.start + c.duration
            guard end >= range.lowerBound, c.start <= range.upperBound else { continue }
            let x = vp.xFor(c.start)
            let w = CGFloat(c.duration * vp.pixelsPerSecond)
            let y = CGFloat(c.track) * Scene.trackHeight + 4
            let rect = CGRect(x: x, y: y, width: w, height: Scene.trackHeight - 8)
            // Peak decimation: one peak per ~2 px, as a real timeline does.
            let wanted = Swift.max(8, Int(w / 2))
            let stride = Swift.max(1, c.peaks.count / wanted)
            var decimated: [Float] = []
            decimated.reserveCapacity(wanted)
            var k = 0
            while k < c.peaks.count { decimated.append(c.peaks[k]); k += stride }
            let thumbs = Swift.max(1, Int(w / 96))
            out.append(ClipDraw(rect: rect, hue: c.hue, peaks: decimated[...],
                                title: c.title, thumbIndex: i % Scene.thumbnails.count,
                                thumbCount: thumbs))
        }
        return out
    }
}

// ---------------------------------------------------------------------------
// Renderer B — custom NSView + CoreGraphics
// ---------------------------------------------------------------------------
final class TimelineNSView: NSView {
    var viewport = Viewport()
    var onDraw: ((Double) -> Void)?
    override var isFlipped: Bool { true }
    override var wantsUpdateLayer: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        let t0 = CACurrentMediaTime()
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(NSColor(white: 0.11, alpha: 1).cgColor)
        ctx.fill(bounds)

        for cd in Layout.visibleClips(viewport, size: bounds.size) {
            let path = CGPath(roundedRect: cd.rect, cornerWidth: 4, cornerHeight: 4, transform: nil)
            ctx.saveGState()
            ctx.addPath(path); ctx.clip()

            // thumbnail strip
            let tw = cd.rect.width / CGFloat(cd.thumbCount)
            for t in 0..<cd.thumbCount {
                let img = Scene.thumbnails[(cd.thumbIndex + t) % Scene.thumbnails.count]
                ctx.draw(img, in: CGRect(x: cd.rect.minX + CGFloat(t)*tw, y: cd.rect.minY,
                                         width: tw, height: cd.rect.height * 0.55))
            }
            // waveform
            ctx.setStrokeColor(NSColor(white: 0.95, alpha: 0.85).cgColor)
            ctx.setLineWidth(1)
            let wy = cd.rect.minY + cd.rect.height * 0.78
            let wh = cd.rect.height * 0.2
            let n = cd.peaks.count
            if n > 1 {
                let dx = cd.rect.width / CGFloat(n)
                ctx.beginPath()
                for (j, p) in cd.peaks.enumerated() {
                    let x = cd.rect.minX + CGFloat(j) * dx
                    ctx.move(to: CGPoint(x: x, y: wy - CGFloat(p) * wh))
                    ctx.addLine(to: CGPoint(x: x, y: wy + CGFloat(p) * wh))
                }
                ctx.strokePath()
            }
            ctx.restoreGState()

            ctx.setStrokeColor(NSColor(hue: cd.hue, saturation: 0.7, brightness: 0.95, alpha: 1).cgColor)
            ctx.setLineWidth(1.5)
            ctx.addPath(path); ctx.strokePath()

            if cd.rect.width > 48 {
                let attrs: [NSAttributedString.Key: Any] = [
                    .font: NSFont.systemFont(ofSize: 10, weight: .medium),
                    .foregroundColor: NSColor.white
                ]
                NSAttributedString(string: cd.title, attributes: attrs)
                    .draw(at: CGPoint(x: cd.rect.minX + 5, y: cd.rect.minY + 3))
            }
        }
        onDraw?((CACurrentMediaTime() - t0) * 1000.0)
    }
}

// ---------------------------------------------------------------------------
// Renderer A — SwiftUI Canvas
// ---------------------------------------------------------------------------
final class VPModel: ObservableObject {
    @Published var viewport = Viewport()
    var onDraw: ((Double) -> Void)?
}

struct TimelineCanvas: View {
    @ObservedObject var model: VPModel
    var body: some View {
        Canvas(rendersAsynchronously: false) { ctx, size in
            let t0 = CACurrentMediaTime()
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(white: 0.11)))

            for cd in Layout.visibleClips(model.viewport, size: size) {
                let path = Path(roundedRect: cd.rect, cornerRadius: 4)
                ctx.drawLayer { layer in
                    layer.clip(to: path)
                    let tw = cd.rect.width / CGFloat(cd.thumbCount)
                    for t in 0..<cd.thumbCount {
                        let img = Scene.thumbnails[(cd.thumbIndex + t) % Scene.thumbnails.count]
                        layer.draw(Image(decorative: img, scale: 1),
                                   in: CGRect(x: cd.rect.minX + CGFloat(t)*tw, y: cd.rect.minY,
                                              width: tw, height: cd.rect.height * 0.55))
                    }
                    var wave = Path()
                    let wy = cd.rect.minY + cd.rect.height * 0.78
                    let wh = cd.rect.height * 0.2
                    let n = cd.peaks.count
                    if n > 1 {
                        let dx = cd.rect.width / CGFloat(n)
                        for (j, p) in cd.peaks.enumerated() {
                            let x = cd.rect.minX + CGFloat(j) * dx
                            wave.move(to: CGPoint(x: x, y: wy - CGFloat(p) * wh))
                            wave.addLine(to: CGPoint(x: x, y: wy + CGFloat(p) * wh))
                        }
                        layer.stroke(wave, with: .color(Color(white: 0.95).opacity(0.85)), lineWidth: 1)
                    }
                }
                ctx.stroke(path, with: .color(Color(hue: cd.hue, saturation: 0.7, brightness: 0.95)),
                           lineWidth: 1.5)
                if cd.rect.width > 48 {
                    ctx.draw(Text(cd.title).font(.system(size: 10, weight: .medium))
                                .foregroundColor(.white),
                             at: CGPoint(x: cd.rect.minX + 5, y: cd.rect.minY + 8), anchor: .topLeading)
                }
            }
            model.onDraw?((CACurrentMediaTime() - t0) * 1000.0)
        }
    }
}
