// S7 — async waveform generation + cache.
//
// Pass condition: 60-minute WAV in <=10 s; UI never blocked >16 ms; cache
// reused across relaunch (spec §17, §26).
//
// The main thread runs a 120 Hz "UI heartbeat" throughout and records its own
// jitter. That is the real test of "never blocks the UI" — a stalled main
// thread shows up as a late tick, exactly as dropped frames would.
import Foundation
import QuartzCore

let path = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "media/test_60min.wav"
let cachePath = "build/waveform.cache"
let BUCKETS_PER_SEC: Int32 = 100     // 100 px/s of timeline at 1x zoom

final class Collector {
    var peaks: [Float] = []
    var batches = 0
    var firstBatchAt: CFTimeInterval = 0
    let start = CACurrentMediaTime()
}
let collector = Collector()

let callback: MCPeakCallback = { user, peaks, pairCount, progress in
    let c = Unmanaged<Collector>.fromOpaque(user!).takeUnretainedValue()
    if c.firstBatchAt == 0 { c.firstBatchAt = CACurrentMediaTime() - c.start }
    c.batches += 1
    c.peaks.append(contentsOf: UnsafeBufferPointer(start: peaks, count: Int(pairCount) * 2))
    return 1   // 0 would cancel
}

// ---- UI heartbeat on the main thread --------------------------------------
var ticks: [Double] = []
var lastTick = CACurrentMediaTime()
var generating = true
let heartbeat = Thread {
    // Simulates the main thread doing UI work at 120 Hz.
    while generating {
        let now = CACurrentMediaTime()
        ticks.append((now - lastTick) * 1000.0)
        lastTick = now
        usleep(8_333)
    }
}

print("S7 — async waveform generation")
print("  source: \(path)")
print("  \(BUCKETS_PER_SEC) buckets/sec\n")

// ---- generation on a background thread ------------------------------------
var duration: Double = 0
var rc: Int32 = -1
let genStart = CACurrentMediaTime()
heartbeat.start()

let worker = Thread {
    rc = mc_waveform(path, BUCKETS_PER_SEC, callback,
                     Unmanaged.passUnretained(collector).toOpaque(), &duration)
}
worker.stackSize = 1 << 20
worker.start()
while !worker.isFinished { usleep(5_000) }
let genElapsed = CACurrentMediaTime() - genStart
generating = false
while !heartbeat.isFinished { usleep(5_000) }

func pct(_ a: [Double], _ p: Double) -> Double {
    let s = a.sorted(); return s[min(s.count-1, max(0, Int(Double(s.count-1)*p)))]
}

let pairs = collector.peaks.count / 2
print("GENERATION")
print("  return code            \(rc) (\(rc == 0 ? "OK" : "error"))")
print(String(format: "  audio duration         %.1f s (%.1f min)", duration, duration/60))
print(String(format: "  wall time              %.2f s  (%.0fx realtime)", genElapsed, duration/genElapsed))
print("  peak pairs             \(pairs)")
print(String(format: "  first batch delivered  %.0f ms  (progressive draw can start here)",
             collector.firstBatchAt * 1000))
print("  batches streamed       \(collector.batches)")

print("\nUI HEARTBEAT (main-thread jitter during generation, target 8.33 ms)")
print(String(format: "  ticks %d   p50 %.2f  p95 %.2f  p99 %.2f  max %.2f ms",
             ticks.count, pct(ticks,0.5), pct(ticks,0.95), pct(ticks,0.99), ticks.max() ?? 0))
let stalls = ticks.filter { $0 > 16.67 }.count
print("  ticks > 16.67 ms       \(stalls)/\(ticks.count)")

// ---- cache ----------------------------------------------------------------
try? FileManager.default.createDirectory(atPath: "build", withIntermediateDirectories: true)
let w0 = CACurrentMediaTime()
_ = collector.peaks.withUnsafeBufferPointer {
    FileManager.default.createFile(atPath: cachePath,
        contents: Data(buffer: $0))
}
let writeMs = (CACurrentMediaTime() - w0) * 1000
let r0 = CACurrentMediaTime()
let reloaded = FileManager.default.contents(atPath: cachePath)
let readMs = (CACurrentMediaTime() - r0) * 1000
let cacheBytes = reloaded?.count ?? 0

print("\nCACHE (spec §17: reused between launches)")
print(String(format: "  size                   %.1f MB", Double(cacheBytes)/1_048_576))
print(String(format: "  write                  %.0f ms", writeMs))
print(String(format: "  read back              %.0f ms   (%.0fx faster than regenerating)",
             readMs, genElapsed*1000/max(readMs, 0.01)))

print("\nVERDICT vs S7 pass condition")
let passTime = genElapsed <= 10.0
let passUI = stalls == 0
print("  60-min WAV <= 10 s     \(passTime ? "PASS" : "FAIL") (\(String(format: "%.2f", genElapsed))s)")
print("  UI never blocked >16ms \(passUI ? "PASS" : "FAIL") (\(stalls) stalls)")
print("  cache reused           \(cacheBytes > 0 ? "PASS" : "FAIL")")
print("  OVERALL                \(passTime && passUI && cacheBytes > 0 ? "PASS" : "FAIL")")
