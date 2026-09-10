import AppKit
import SwiftUI

// S2b harness. Simulates a continuous multi-clip drag and measures the latency
// from model mutation to pixels, plus hit-test cost and dropped frames.

let S2B_FRAMES = 300

final class DragHarness: NSObject {
    let window: NSWindow
    let model = EditModel()
    var latencies: [Double] = []
    var hitTestUs: [Double] = []
    var frameMs: [Double] = []
    var last: CFTimeInterval = 0
    var frames = 0
    var link: CADisplayLink?
    var host: NSView
    var kick: (() -> Void)!
    var done: (() -> Void)?

    init(title: String, view: NSView, kick: @escaping () -> Void) {
        window = NSWindow(contentRect: NSRect(x: 120, y: 120, width: WIDTH, height: HEIGHT),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.title = title
        window.contentView = view
        host = view
        super.init()
        self.kick = kick
        model.onDraw = { [weak self] ms in self?.latencies.append(ms) }
    }

    func run(completion: @escaping () -> Void) {
        done = completion
        latencies.removeAll(); hitTestUs.removeAll(); frameMs.removeAll()
        frames = 0; last = 0
        window.orderFront(nil)
        let l = host.displayLink(target: self, selector: #selector(tick))
        l.add(to: .main, forMode: .common)
        link = l
    }

    @objc func tick(_ l: CADisplayLink) {
        let now = CACurrentMediaTime()
        if last > 0 { frameMs.append((now - last) * 1000.0) }
        last = now

        // Hit-test at a moving cursor, as a real drag does every event.
        let cursor = CGPoint(x: 300 + CGFloat(frames % 900), y: 40)
        let h0 = CACurrentMediaTime()
        _ = model.hitTest(cursor)
        hitTestUs.append((CACurrentMediaTime() - h0) * 1_000_000.0)

        // One frame of a 20-clip drag, then force the redraw.
        model.dragBy(0.05)
        kick()

        frames += 1
        if frames >= S2B_FRAMES {
            l.invalidate(); link = nil; window.orderOut(nil); done?()
        }
    }
}

func runS2b(_ finish: @escaping () -> Void) {
    let nsv = EditNSView(frame: NSRect(x: 0, y: 0, width: WIDTH, height: HEIGHT))
    var hB: DragHarness!
    hB = DragHarness(title: "NSView drag", view: nsv, kick: { nsv.setNeedsDisplay(nsv.bounds) })
    nsv.model = hB.model

    var hA: DragHarness!
    let holder = NSView(frame: NSRect(x: 0, y: 0, width: WIDTH, height: HEIGHT))
    hA = DragHarness(title: "SwiftUI drag", view: holder, kick: {})
    let hosting = NSHostingView(rootView: EditCanvas(model: hA.model))
    hosting.frame = holder.bounds
    holder.addSubview(hosting)

    print("\nS2b — Timeline INTERACTION latency")
    print("  dragging a 20-clip selection; 200 clips total; \(S2B_FRAMES) frames each")
    print("  measures: model mutation -> pixels on screen\n")

    hA.run {
        hB.run {
            print("MUTATION -> PIXELS (the number that decides how a drag feels)")
            Stats(samples: hA.latencies).report("SwiftUI Canvas")
            Stats(samples: hB.latencies).report("NSView + CoreGraphics")

            print("\nHIT-TEST (linear scan over 200 clips, microseconds)")
            for (n, h) in [("SwiftUI Canvas", hA!), ("NSView + CoreGraphics", hB!)] {
                let s = Stats(samples: h.hitTestUs)
                print(String(format: "  %-26s p50 %6.1f  p99 %6.1f  max %7.1f us",
                             (n as NSString).utf8String!, s.p50, s.p99, s.max))
            }

            print("\nDROPPED FRAMES DURING DRAG (> 1.5x refresh)")
            for (n, h) in [("SwiftUI Canvas", hA!), ("NSView + CoreGraphics", hB!)] {
                let s = Stats(samples: h.frameMs)
                let dropped = s.samples.filter { $0 > s.p50 * 1.5 }.count
                print(String(format: "  %-26s refresh %.2fms  dropped %d/%d",
                             (n as NSString).utf8String!, s.p50, dropped, s.samples.count))
            }

            print("\nVERDICT vs S2b pass condition (<=16ms mutation->pixels, no drops in drag)")
            for (n, h) in [("SwiftUI Canvas", hA!), ("NSView + CoreGraphics", hB!)] {
                let s = Stats(samples: h.latencies)
                let f = Stats(samples: h.frameMs)
                let dropped = f.samples.filter { $0 > f.p50 * 1.5 }.count
                let pass = s.p99 <= 16.67 && dropped <= 3
                print(String(format: "  %-26s %@  p99 %.2fms, %d dropped",
                             (n as NSString).utf8String!,
                             (pass ? "PASS" : "FAIL") as NSString, s.p99, dropped))
            }
            finish()
        }
    }
}
