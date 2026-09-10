import AppKit
import SwiftUI

// S2 harness. Runs each renderer on-screen for a fixed number of display-link
// frames while animating scroll + zoom, then reports frame-time percentiles.
//
// Measured: time spent inside the draw path, and the interval between presented
// frames (which is what actually reveals dropped frames).
//
// .accessory activation policy: no dock icon and no focus stealing.

let FRAMES_PER_RUN = 240
// Zoom regimes, px/sec. 8 px/s puts the ENTIRE 200-clip timeline on screen at once —
// the worst realistic case, and the one that finds the knee.
let ZOOMS: [Double] = [80, 24, 8]
let WIDTH: CGFloat = 1600
let HEIGHT = Scene.trackHeight * CGFloat(Scene.trackCount)

final class Harness: NSObject {
    let window: NSWindow
    var drawMs: [Double] = []
    var frameMs: [Double] = []
    var lastFrame: CFTimeInterval = 0
    var frames = 0
    var displayLink: CADisplayLink?
    var advance: ((Viewport) -> Void)?
    var current = Viewport()
    var done: (() -> Void)?
    var hostView: NSView
    var zoom: Double = 80

    init(title: String, content: NSView) {
        window = NSWindow(contentRect: NSRect(x: 120, y: 120, width: WIDTH, height: HEIGHT),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.title = title
        window.contentView = content
        hostView = content
        super.init()
    }

    func run(zoom: Double, completion: @escaping () -> Void) {
        done = completion
        self.zoom = zoom
        drawMs.removeAll(); frameMs.removeAll(); frames = 0
        current = Viewport(scrollSeconds: 0, pixelsPerSecond: zoom)
        lastFrame = 0
        window.orderFront(nil)
        let link = hostView.displayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @objc func tick(_ link: CADisplayLink) {
        let now = CACurrentMediaTime()
        if lastFrame > 0 { frameMs.append((now - lastFrame) * 1000.0) }
        lastFrame = now

        // Fixed zoom per pass; scroll continuously so content churns every frame.
        let t = Double(frames) / 60.0
        current.scrollSeconds = t * (zoom / 12.0)
        current.pixelsPerSecond = zoom
        advance?(current)

        frames += 1
        if frames >= FRAMES_PER_RUN {
            link.invalidate(); displayLink = nil
            window.orderOut(nil)
            done?()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var harnesses: [(String, Harness)] = []
    var results: [(String, Double, Stats, Stats)] = []
    var index = 0
    var zoomIndex = 0

    func applicationDidFinishLaunching(_ n: Notification) {
        // Renderer B — custom NSView
        let nsv = TimelineNSView(frame: NSRect(x: 0, y: 0, width: WIDTH, height: HEIGHT))
        let hB = Harness(title: "NSView", content: nsv)
        nsv.onDraw = { [weak hB] ms in hB?.drawMs.append(ms) }
        hB.advance = { [weak nsv] vp in nsv?.viewport = vp; nsv?.setNeedsDisplay(nsv!.bounds) }

        // Renderer A — SwiftUI Canvas
        let model = VPModel()
        let host = NSHostingView(rootView: TimelineCanvas(model: model))
        host.frame = NSRect(x: 0, y: 0, width: WIDTH, height: HEIGHT)
        let hA = Harness(title: "SwiftUI Canvas", content: host)
        model.onDraw = { [weak hA] ms in hA?.drawMs.append(ms) }
        hA.advance = { vp in model.viewport = vp }

        harnesses = [("SwiftUI Canvas", hA), ("NSView + CoreGraphics", hB)]

        print("S2 — Timeline rendering spike")
        print("  \(Scene.clipCount) clips, \(Scene.trackCount) tracks, waveforms + thumbnail strips")
        print("  viewport \(Int(WIDTH))x\(Int(HEIGHT)), \(FRAMES_PER_RUN) frames per renderer")
        for z in ZOOMS {
            let sample = Layout.visibleClips(Viewport(scrollSeconds: 5, pixelsPerSecond: z),
                                             size: CGSize(width: WIDTH, height: HEIGHT))
            let peaks = sample.reduce(0) { $0 + $1.peaks.count }
            let thumbs = sample.reduce(0) { $0 + $1.thumbCount }
            print("  @\(Int(z))px/s: \(sample.count) clips visible, \(peaks) waveform segments, \(thumbs) thumbnails")
        }
        print("  budgets: 60fps = 16.67ms   120fps = 8.33ms\n")
        runNext()
    }

    func runNext() {
        guard index < harnesses.count else { return finish() }
        guard zoomIndex < ZOOMS.count else { index += 1; zoomIndex = 0; return runNext() }
        let (name, h) = harnesses[index]
        let z = ZOOMS[zoomIndex]
        h.run(zoom: z) { [weak self] in
            guard let self else { return }
            self.results.append((name, z, Stats(samples: h.drawMs), Stats(samples: h.frameMs)))
            self.zoomIndex += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.runNext() }
        }
    }

    func finish() {
        print("DRAW TIME by zoom regime")
        for (name, z, draw, _) in results { draw.report("\(name) @\(Int(z))px/s") }
        print("\nDROPPED FRAMES (interval > 1.5x the display refresh)")
        for (name, z, _, frame) in results {
            let refresh = frame.p50
            let dropped = frame.samples.filter { $0 > refresh * 1.5 }.count
            print(String(format: "  %-34s refresh %.2fms  dropped %d/%d",
                         ("\(name) @\(Int(z))px/s" as NSString).utf8String!,
                         refresh, dropped, frame.samples.count))
        }
        print("\nVERDICT vs S2 pass condition (>=60fps sustained, no draw >16.67ms)")
        for (name, z, draw, _) in results {
            let v = draw.p99 < 8.33 ? "PASS (120fps headroom)"
                  : draw.p99 < 16.67 ? "PASS (60fps only)" : "FAIL"
            print(String(format: "  %-34s p99 %6.2fms  %@",
                         ("\(name) @\(Int(z))px/s" as NSString).utf8String!, draw.p99, v))
        }
        runS2b { NSApp.terminate(nil) }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)   // no dock icon, no focus stealing
let delegate = AppDelegate()
app.delegate = delegate
app.run()
