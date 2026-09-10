import SwiftUI
import Metal
import MetalKit
import CoreVideo
import QuartzCore

/// Metal preview.
///
/// Consumes a render plan from the core (AD-4: the core says WHAT, the
/// compositor says HOW) and draws it with the zero-copy path S1 validated —
/// VideoToolbox produces a `CVPixelBuffer`, `CVMetalTextureCache` binds it as
/// an `MTLTexture`, and no pixel crosses the FFI or touches the CPU.
struct PreviewPane: View {
    @ObservedObject var doc: EditorDocument
    // --guide <id> selects one at launch, so the overlay is verifiable without
    // driving the picker.
    @State private var guide: SafeAreaGuide = {
        if let i = CommandLine.arguments.firstIndex(of: "--guide"),
           i + 1 < CommandLine.arguments.count,
           let g = SafeAreaGuide.all.first(where: { $0.id == CommandLine.arguments[i + 1] }) {
            return g
        }
        return SafeAreaGuide.all[0]
    }()

    var body: some View {
        ZStack {
            MetalPreview(doc: doc)
            if !guide.isOff {
                SafeAreaOverlay(guide: guide, contentAspect: doc.previewAspect)
            }
            VStack {
                HStack {
                    // Small and out of the way: guides are a check you make
                    // occasionally, not a mode you live in.
                    Picker("", selection: $guide) {
                        ForEach(SafeAreaGuide.all) { Text($0.name).tag($0) }
                    }
                    .labelsHidden()
                    .controlSize(.small)
                    .frame(width: 168)
                    .padding(8)
                    Spacer()
                }
                Spacer()
                HStack {
                    Spacer()
                    Text(doc.timecode)
                        .font(.system(size: 11, design: .monospaced))
                        .padding(.horizontal, 6).padding(.vertical, 3)
                        .background(.black.opacity(0.55))
                        .cornerRadius(4)
                        .padding(8)
                }
            }
        }
        .background(Color.black)
    }
}

struct MetalPreview: NSViewRepresentable {
    @ObservedObject var doc: EditorDocument

    func makeNSView(context: Context) -> PreviewMTKView {
        let v = PreviewMTKView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        v.frames = doc.frameSource
        return v
    }
    func updateNSView(_ v: PreviewMTKView, context: Context) {
        v.plan = Array(doc.currentPlan())
        v.assetPath = { doc.assetPath($0) }
        v.textSpec = { doc.text(for: $0) }
        v.imageSpec = { doc.image(for: $0) }
        v.lutSpec = { doc.lut(for: $0) }
        v.captionSpec = { doc.captionSpec(at: $0) }
        v.needsDisplay = true
    }
}

final class FrameSource {
    private var decoders: [UInt64: OpaquePointer] = [:]
    private var order: [UInt64] = []
    private let maxOpen = 4          // R-23: ~337 MB each at 4K
    /// FFmpeg decoders are NOT safe for concurrent use. Sharing one pool
    /// between the preview thread and an export thread tripped an internal
    /// assertion (`fctx->async_lock failed`) and produced a 44-byte file.
    /// Export now uses its own FrameSource, and this lock guards the rest.
    private let lock = NSLock()
    /// Last PTS delivered per asset, so sequential reads can skip the seek.
    private var lastPts: [UInt64: Int64] = [:]

    private func decoderLocked(for asset: UInt64, path: String) -> OpaquePointer? {
        if let d = decoders[asset] {
            order.removeAll { $0 == asset }; order.append(asset)
            return d
        }
        guard let d = mc_open(path) else { return nil }
        decoders[asset] = d
        order.append(asset)
        while order.count > maxOpen, let evict = order.first {
            order.removeFirst()
            if let old = decoders.removeValue(forKey: evict) { mc_close(old) }
            lastPts.removeValue(forKey: evict)
        }
        return d
    }

    /// Decode the frame at `sourceTicks`, seeking only when necessary.
    ///
    /// A seek lands on the preceding KEYFRAME and the decoder must then run
    /// forward to the requested frame — inherent to inter-frame codecs.
    ///
    /// Seeking before EVERY frame therefore makes sequential playback and
    /// export pathologically slow: each frame pays a seek plus a decode-forward
    /// from the last keyframe. Export was visibly crawling because of exactly
    /// that. When the request is just ahead of where the decoder already sits,
    /// keep decoding forward instead — which is the normal case for both
    /// playback and export.
    func frame(asset: UInt64, path: String, sourceTicks: Int64) -> CVPixelBuffer? {
        lock.lock()
        defer { lock.unlock() }
        guard let d = decoderLocked(for: asset, path: path) else { return nil }
        let target = Int64(Double(sourceTicks) / Double(TICKS_PER_SECOND) * 1e9)

        // Within 2 seconds ahead: decode forward. Behind, or a long way ahead:
        // seek. Two seconds is comfortably more than a GOP but far less than
        // the cost of decoding through one.
        let previous = lastPts[asset]
        let sequential = previous.map { target >= $0 && target - $0 < 2_000_000_000 } ?? false
        if !sequential { _ = mc_seek(d, target) }

        var best: CVPixelBuffer?
        var guardCount = 0
        while guardCount < 240 {                 // never spin forever on bad media
            guardCount += 1
            var raw: UnsafeRawPointer?
            var pts: Int64 = 0
            let r = mc_next_frame(d, &raw, &pts)
            if r <= 0 { break }
            guard let raw else { continue }
            best = Unmanaged<CVPixelBuffer>.fromOpaque(raw).takeRetainedValue()
            lastPts[asset] = pts
            if pts >= target { break }
        }
        return best
    }

    deinit { for (_, d) in decoders { mc_close(d) } }

    /// Close everything; used when an export finishes with its own pool.
    func closeAll() {
        lock.lock(); defer { lock.unlock() }
        for (_, d) in decoders { mc_close(d) }
        decoders.removeAll(); order.removeAll(); lastPts.removeAll()
    }
}


/// A thin host for the shared `Compositor`.
///
/// Preview and export deliberately share one compositor: R-18 is the risk that
/// exported output diverges from the preview, and the surest guard is having
/// only one implementation.
final class PreviewMTKView: MTKView {
    var frames: FrameSource?
    var plan: [MCPlanLayer] = []
    var assetPath: ((UInt64) -> String?)?
    var textSpec: ((UInt64) -> TextSpec?)?
    var imageSpec: ((UInt64) -> ImageSpec?)?
    var lutSpec: ((UInt64) -> Core.LutSpec?)?
    var captionSpec: ((Int64) -> TextSpec?)?
    private var compositor: Compositor?

    override init(frame: CGRect, device: MTLDevice?) {
        super.init(frame: frame, device: device)
        isPaused = true
        enableSetNeedsDisplay = true
        colorPixelFormat = .bgra8Unorm
        clearColor = MTLClearColorMake(0, 0, 0, 1)
        if let device { compositor = Compositor(device: device) }
    }
    required init(coder: NSCoder) { fatalError("not used") }

    override func draw(_ rect: CGRect) {
        guard let compositor, let drawable = currentDrawable,
              let frames else { return }
        compositor.render(plan: plan, into: drawable.texture,
                          assetPath: { self.assetPath?($0) },
                          textSpec: { self.textSpec?($0) },
                          imageSpec: { self.imageSpec?($0) },
                          lutSpec: { self.lutSpec?($0) },
                          captionSpec: { self.captionSpec?($0) },
                          frames: frames,
                          drawable: drawable)
    }
}
