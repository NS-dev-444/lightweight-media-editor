import AppKit
import SwiftUI

// S2b — interaction latency.
//
// S2 only animated the VIEWPORT. This mutates the CONTENT: a multi-clip
// selection is dragged, so clip positions change every frame and the change
// must propagate through the framework's update cycle before pixels appear.
// That is the path where SwiftUI's per-update overhead actually shows.
//
// Measured:
//   mutation -> pixels   time from the model mutation to the draw completing
//   hit-test             cost of finding the clip under the cursor
//   dropped frames       during a continuous drag

final class EditModel: ObservableObject {
    @Published var starts: [Double]
    @Published var selection: Set<Int>
    var viewport = Viewport(scrollSeconds: 0, pixelsPerSecond: 40)
    var mutationTime: CFTimeInterval = 0
    var onDraw: ((Double) -> Void)?

    init() {
        starts = Scene.clips.map { $0.start }
        selection = Set(0..<20)          // 20-clip multi-selection being dragged
    }

    /// One frame of a drag: move every selected clip, then stamp the mutation.
    func dragBy(_ seconds: Double) {
        mutationTime = CACurrentMediaTime()
        for i in selection { starts[i] += seconds }
    }

    /// Hit-test: which clip is under this point? Linear scan over 200 clips,
    /// which is what a real implementation would do before optimising.
    func hitTest(_ p: CGPoint) -> Int? {
        let track = Int(p.y / Scene.trackHeight)
        for (i, c) in Scene.clips.enumerated() where c.track == track {
            let x = viewport.xFor(starts[i])
            let w = CGFloat(c.duration * viewport.pixelsPerSecond)
            if p.x >= x && p.x <= x + w { return i }
        }
        return nil
    }

    func draws(size: CGSize) -> [(ClipDraw, Bool)] {
        let range = viewport.visibleRange(width: size.width)
        var out: [(ClipDraw, Bool)] = []
        for (i, c) in Scene.clips.enumerated() {
            let st = starts[i], end = st + c.duration
            guard end >= range.lowerBound, st <= range.upperBound else { continue }
            let x = viewport.xFor(st)
            let w = CGFloat(c.duration * viewport.pixelsPerSecond)
            let y = CGFloat(c.track) * Scene.trackHeight + 4
            let rect = CGRect(x: x, y: y, width: w, height: Scene.trackHeight - 8)
            let wanted = Swift.max(8, Int(w / 2))
            let stride = Swift.max(1, c.peaks.count / wanted)
            var dec: [Float] = []; dec.reserveCapacity(wanted)
            var k = 0; while k < c.peaks.count { dec.append(c.peaks[k]); k += stride }
            out.append((ClipDraw(rect: rect, hue: c.hue, peaks: dec[...], title: c.title,
                                 thumbIndex: i % Scene.thumbnails.count,
                                 thumbCount: Swift.max(1, Int(w / 96))),
                        selection.contains(i)))
        }
        return out
    }
}

private func drawScene(_ items: [(ClipDraw, Bool)], _ ctx: CGContext, _ bounds: CGRect) {
    ctx.setFillColor(NSColor(white: 0.11, alpha: 1).cgColor)
    ctx.fill(bounds)
    for (cd, selected) in items {
        let path = CGPath(roundedRect: cd.rect, cornerWidth: 4, cornerHeight: 4, transform: nil)
        ctx.saveGState(); ctx.addPath(path); ctx.clip()
        let tw = cd.rect.width / CGFloat(cd.thumbCount)
        for t in 0..<cd.thumbCount {
            let img = Scene.thumbnails[(cd.thumbIndex + t) % Scene.thumbnails.count]
            ctx.draw(img, in: CGRect(x: cd.rect.minX + CGFloat(t)*tw, y: cd.rect.minY,
                                     width: tw, height: cd.rect.height * 0.55))
        }
        ctx.setStrokeColor(NSColor(white: 0.95, alpha: 0.85).cgColor); ctx.setLineWidth(1)
        let wy = cd.rect.minY + cd.rect.height * 0.78, wh = cd.rect.height * 0.2
        if cd.peaks.count > 1 {
            let dx = cd.rect.width / CGFloat(cd.peaks.count)
            ctx.beginPath()
            for (j, p) in cd.peaks.enumerated() {
                let x = cd.rect.minX + CGFloat(j) * dx
                ctx.move(to: CGPoint(x: x, y: wy - CGFloat(p)*wh))
                ctx.addLine(to: CGPoint(x: x, y: wy + CGFloat(p)*wh))
            }
            ctx.strokePath()
        }
        ctx.restoreGState()
        ctx.setStrokeColor(selected ? NSColor.systemYellow.cgColor
                                    : NSColor(hue: cd.hue, saturation: 0.7, brightness: 0.95, alpha: 1).cgColor)
        ctx.setLineWidth(selected ? 3 : 1.5)
        ctx.addPath(path); ctx.strokePath()
    }
}

final class EditNSView: NSView {
    var model: EditModel!
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        drawScene(model.draws(size: bounds.size), ctx, bounds)
        if model.mutationTime > 0 {
            if model.mutationTime > 0 {
                model.onDraw?((CACurrentMediaTime() - model.mutationTime) * 1000.0)
            }
        }
    }
}

struct EditCanvas: View {
    @ObservedObject var model: EditModel
    var body: some View {
        Canvas(rendersAsynchronously: false) { ctx, size in
            let items = model.draws(size: size)
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(white: 0.11)))
            for (cd, selected) in items {
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
                    let wy = cd.rect.minY + cd.rect.height * 0.78, wh = cd.rect.height * 0.2
                    if cd.peaks.count > 1 {
                        let dx = cd.rect.width / CGFloat(cd.peaks.count)
                        for (j, p) in cd.peaks.enumerated() {
                            let x = cd.rect.minX + CGFloat(j) * dx
                            wave.move(to: CGPoint(x: x, y: wy - CGFloat(p)*wh))
                            wave.addLine(to: CGPoint(x: x, y: wy + CGFloat(p)*wh))
                        }
                        layer.stroke(wave, with: .color(Color(white: 0.95).opacity(0.85)), lineWidth: 1)
                    }
                }
                ctx.stroke(path, with: selected ? .color(.yellow)
                                : .color(Color(hue: cd.hue, saturation: 0.7, brightness: 0.95)),
                           lineWidth: selected ? 3 : 1.5)
            }
            if model.mutationTime > 0 {
                model.onDraw?((CACurrentMediaTime() - model.mutationTime) * 1000.0)
            }
        }
    }
}
