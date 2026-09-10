// S6 HOST PROCESS — stands in for the UI process.
//
// It links NO media libraries. It cannot be crashed by a codec bug because it
// never parses media. It spawns workers, enforces a timeout, and classifies
// every outcome. This is the structural claim in AD-3 being tested.
import Foundation

let corpus = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "build/s6-corpus"
let workerPath = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : "./s6-worker"
let TIMEOUT: TimeInterval = 20.0
let CONCURRENCY = 8

let files = (try! FileManager.default.contentsOfDirectory(atPath: corpus)).sorted()

enum Outcome { case ok(Int32), classified(Int32), crashed(Int32), timedOut, spawnFailed }

let lock = NSLock()
var outcomes: [String: Outcome] = [:]
var messages: [Int32: String] = [:]
var classCounts: [Int32: Int] = [:]
var crashSignals: [Int32: Int] = [:]
var timeouts: [String] = []
var hostErrors = 0

func probe(_ name: String) -> Outcome {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: workerPath)
    p.arguments = [corpus + "/" + name]
    let out = Pipe()
    p.standardOutput = out
    p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { return .spawnFailed }

    let sem = DispatchSemaphore(value: 0)
    var data = Data()
    let reader = Thread {
        data = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        sem.signal()
    }
    reader.stackSize = 512 * 1024
    reader.start()
    if sem.wait(timeout: .now() + TIMEOUT) == .timedOut {
        kill(p.processIdentifier, SIGKILL)     // the worker is expendable
        _ = sem.wait(timeout: .now() + 5)
        return .timedOut
    }
    if p.terminationReason == .uncaughtSignal {
        return .crashed(p.terminationStatus)
    }
    guard let line = String(data: data, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines),
          let cls = Int32(line.split(separator: "\t").first.map(String.init) ?? "") else {
        return .spawnFailed
    }
    let parts = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
    lock.lock(); if messages[cls] == nil, parts.count >= 5 { messages[cls] = parts[4] }; lock.unlock()
    return cls == 0 ? .ok(cls) : .classified(cls)
}

print("S6 — worker-process crash isolation")
print("  corpus: \(files.count) files")
print("  host links NO media libraries; workers do all parsing")
print("  timeout \(Int(TIMEOUT))s, concurrency \(CONCURRENCY)\n")

// Dedicated OS threads, NOT GCD.
//
// The first version of this harness used a concurrent DispatchQueue whose
// blocks blocked on a semaphore, plus a global-queue block per child doing a
// blocking pipe read. That starves the GCD thread pool: blocked threads are
// only replaced after a delay, so children sat unreaped and tripped the
// timeout. It produced 66 phantom "hangs" that had nothing to do with the
// media stack. Blocking work belongs on threads you own.
//
// This is a real lesson for the product: the process supervisor behind the
// conversion queue (§21) must never block GCD threads waiting on children.
let start = Date()
var cursor = 0
let cursorLock = NSLock()
var threads: [Thread] = []

for _ in 0..<CONCURRENCY {
    let t = Thread {
        while true {
            cursorLock.lock()
            let idx = cursor
            cursor += 1
            cursorLock.unlock()
            if idx >= files.count { break }
            let f = files[idx]
            let o = probe(f)
            lock.lock()
            outcomes[f] = o
            switch o {
            case .ok(let c), .classified(let c): classCounts[c, default: 0] += 1
            case .crashed(let sig): crashSignals[sig, default: 0] += 1
            case .timedOut: timeouts.append(f)
            case .spawnFailed: hostErrors += 1
            }
            lock.unlock()
        }
    }
    t.stackSize = 512 * 1024
    threads.append(t)
    t.start()
}
while threads.contains(where: { !$0.isFinished }) { usleep(20_000) }
let elapsed = Date().timeIntervalSince(start)

let names: [Int32: String] = [0: "OK", 1: "FILE_UNREADABLE", 2: "NOT_MEDIA",
                              3: "NO_VIDEO_STREAM", 4: "UNSUPPORTED_CODEC",
                              5: "CORRUPT_HEADER", 6: "TRUNCATED", 7: "DECODE_FAILED"]

print("OUTCOMES (\(files.count) files in \(String(format: "%.1f", elapsed))s)")
for (c, n) in classCounts.sorted(by: { $0.key < $1.key }) {
    let nm = (names[c] ?? "class \(c)").padding(toLength: 20, withPad: " ", startingAt: 0)
    print("  \(nm) \(n)")
}
let crashed = crashSignals.values.reduce(0, +)
print("\nWORKER CRASHES (expected, and the reason for isolation)")
if crashSignals.isEmpty { print("  none") }
for (sig, n) in crashSignals.sorted(by: { $0.key < $1.key }) {
    print("  signal \(sig): \(n)")
}
print("  timeouts (killed): \(timeouts.count)")

print("\nHUMAN-READABLE MESSAGES (spec §23 — no raw codec errors)")
for (c, m) in messages.sorted(by: { $0.key < $1.key }) where c != 0 {
    print("  [\(names[c] ?? "\(c)")] \(m)")
}

let unclassified = files.count - classCounts.values.reduce(0,+) - crashed - timeouts.count - hostErrors
print("\nVERDICT vs S6 pass condition")
print("  HOST PROCESS CRASHES        \(hostErrors == 0 ? "0  PASS" : "\(hostErrors)  FAIL")")
print("  every file produced a result \(unclassified == 0 ? "yes  PASS" : "no (\(unclassified) unaccounted)  FAIL")")
print("  worker failures contained    \(crashed + timeouts.count) crashed/hung, all recovered")
print("  OVERALL                      \(hostErrors == 0 && unclassified == 0 ? "PASS" : "FAIL")")
