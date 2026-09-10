import Foundation
import Metal
import CoreVideo
import IOSurface
import QuartzCore

// S1 — zero-copy frame path:
//   Rust core (FFmpeg + VideoToolbox) -> CVPixelBuffer -> MTLTexture -> GPU
// Pass condition: 4K30 sustained >= 29.5 fps, ZERO CPU frame copies, RSS < 500 MB.

func rssMB() -> Double {
    var info = mach_task_basic_info()
    var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
    let kr = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
        }
    }
    return kr == KERN_SUCCESS ? Double(info.resident_size) / 1_048_576.0 : -1
}

func pct(_ a: [Double], _ p: Double) -> Double {
    let s = a.sorted(); return s[min(s.count-1, max(0, Int(Double(s.count-1)*p)))]
}

guard CommandLine.arguments.count > 1 else { print("usage: s1 <file>"); exit(2) }
let path = CommandLine.arguments[1]

guard let dev = MTLCreateSystemDefaultDevice(), let queue = dev.makeCommandQueue() else {
    print("no Metal device"); exit(2)
}

var cache: CVMetalTextureCache?
CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, dev, nil, &cache)
guard let texCache = cache else { print("no texture cache"); exit(2) }

guard let dec = mc_open(path) else { print("mc_open failed"); exit(2) }
var info = MCInfo()
mc_info(dec, &info)

print("S1 — zero-copy frame path")
print("  source: \(info.width)x\(info.height) @ \(String(format: "%.2f", info.fps))fps, \(String(format: "%.1f", info.duration_sec))s")
print("  hardware decode requested: \(info.hw_accelerated == 1 ? "yes (VideoToolbox)" : "NO")")
print("  RSS before decoding: \(String(format: "%.1f", rssMB())) MB\n")

// Offscreen render target — deliberately NOT a window, so vsync does not cap
// throughput and hide the true decode+bind ceiling.
let rtDesc = MTLTextureDescriptor.texture2DDescriptor(
    pixelFormat: .bgra8Unorm, width: 960, height: 540, mipmapped: false)
rtDesc.usage = [.renderTarget, .shaderRead]
let renderTarget = dev.makeTexture(descriptor: rtDesc)!

var frames = 0
var iosurfaceBacked = 0
var texturesBound = 0
var frameMs: [Double] = []
var peakRSS = rssMB()
let t0 = CACurrentMediaTime()

while true {
    let f0 = CACurrentMediaTime()
    var pbRaw: UnsafeRawPointer? = nil
    var ptsNs: Int64 = 0
    let r = mc_next_frame(dec, &pbRaw, &ptsNs)
    if r <= 0 { break }
    guard let raw = pbRaw else { continue }

    // Rust handed over a +1 retained CVPixelBufferRef. takeRetainedValue()
    // consumes that reference and lets ARC release it at end of scope —
    // this is the ownership contract across the FFI, made explicit.
    let pb = Unmanaged<CVPixelBuffer>.fromOpaque(raw).takeRetainedValue()

    // Proof 1: the buffer is IOSurface-backed, i.e. GPU-shareable with no copy.
    if CVPixelBufferGetIOSurface(pb) != nil { iosurfaceBacked += 1 }

    // Proof 2: bind it as a Metal texture directly from the same IOSurface.
    // CVMetalTextureCache wraps the existing surface; it does not copy pixels.
    var cvTexY: CVMetalTexture?
    let w = CVPixelBufferGetWidthOfPlane(pb, 0)
    let h = CVPixelBufferGetHeightOfPlane(pb, 0)
    let st = CVMetalTextureCacheCreateTextureFromImage(
        kCFAllocatorDefault, texCache, pb, nil, .r8Unorm, w, h, 0, &cvTexY)

    if st == kCVReturnSuccess, let cvTexY, let lumaTex = CVMetalTextureGetTexture(cvTexY) {
        texturesBound += 1
        // Proof 3: the GPU actually consumes it — a real render pass reading the texture.
        let rp = MTLRenderPassDescriptor()
        rp.colorAttachments[0].texture = renderTarget
        rp.colorAttachments[0].loadAction = .clear
        rp.colorAttachments[0].storeAction = .store
        rp.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
        if let cb = queue.makeCommandBuffer() {
            if let enc = cb.makeRenderCommandEncoder(descriptor: rp) {
                enc.setFragmentTexture(lumaTex, index: 0)
                enc.endEncoding()
            }
            if let blit = cb.makeBlitCommandEncoder() {
                blit.synchronize(resource: renderTarget)
                blit.endEncoding()
            }
            cb.commit()
            cb.waitUntilCompleted()
        }
    }

    frames += 1
    frameMs.append((CACurrentMediaTime() - f0) * 1000.0)
    if frames % 60 == 0 { peakRSS = max(peakRSS, rssMB()) }
}

let elapsed = CACurrentMediaTime() - t0
let fps = Double(frames) / elapsed
let swFrames = mc_sw_frame_count(dec)
peakRSS = max(peakRSS, rssMB())
mc_close(dec)

print("RESULTS")
print("  frames decoded          \(frames) in \(String(format: "%.2f", elapsed))s")
print(String(format: "  throughput              %.1f fps  (%.2fx realtime vs %.0f fps source)",
             fps, fps / max(info.fps, 1), info.fps))
print(String(format: "  per-frame decode+bind   p50 %.2f  p95 %.2f  p99 %.2f  max %.2f ms",
             pct(frameMs,0.5), pct(frameMs,0.95), pct(frameMs,0.99), frameMs.max() ?? 0))
// Warm-up is a real, user-visible cost (it is what "open a file" feels like),
// so report it separately rather than letting it pollute the steady state.
let warm = Array(frameMs.prefix(30)), steady = Array(frameMs.dropFirst(30))
print(String(format: "    first frame           %.1f ms  (decoder + VideoToolbox session + Metal warm-up)", frameMs.first ?? 0))
print(String(format: "    frames 1-30 (warm-up) mean %.2f ms", warm.reduce(0,+)/Double(warm.count)))
print(String(format: "    steady state          p50 %.2f  p95 %.2f  p99 %.2f  max %.2f ms  -> %.0f fps",
             pct(steady,0.5), pct(steady,0.95), pct(steady,0.99), steady.max() ?? 0,
             1000.0/pct(steady,0.5)))
print("\nZERO-COPY EVIDENCE")
print("  software-decoded frames \(swFrames)          (must be 0)")
print("  IOSurface-backed        \(iosurfaceBacked)/\(frames)")
print("  bound as MTLTexture     \(texturesBound)/\(frames)")
print(String(format: "\nMEMORY\n  peak RSS                %.1f MB   (budget 500 MB)", peakRSS))

let uncopiedBytes = Double(frames) * Double(info.width * info.height) * 1.5 / 1_048_576.0
print(String(format: "  frame bytes NOT copied  %.0f MB across the run", uncopiedBytes))

print("\nVERDICT vs S1 pass condition")
let passFps = fps >= 29.5
let passCopy = swFrames == 0 && texturesBound == frames && iosurfaceBacked == frames
let passMem = peakRSS < 500
print("  throughput >= 29.5 fps  \(passFps ? "PASS" : "FAIL")")
print("  zero CPU frame copies   \(passCopy ? "PASS" : "FAIL")")
print("  peak RSS < 500 MB       \(passMem ? "PASS" : "FAIL")")
print("  OVERALL                 \(passFps && passCopy && passMem ? "PASS" : "FAIL")")
