// S8 — App Sandbox + security-scoped bookmarks + sandboxed worker.
//
// Spec §13/§42 and AD-9: a non-destructive editor holds long-lived references
// to files the user chose weeks ago. Under the sandbox that only works via
// security-scoped bookmarks, which is why AD-9 puts them in the project schema
// from v1 — retrofitting them would force a format migration.
//
// The architecturally critical question is NOT the bookmark API. It is:
//   does the media WORKER PROCESS inherit access to the bookmarked file?
// AD-3 puts all decoding in a child process. If the child cannot open the file,
// the worker API must pass a file DESCRIPTOR instead of a path — a design
// change that is cheap now and expensive in Phase 3.
import Foundation
import AppKit

let fm = FileManager.default
let container = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
try? fm.createDirectory(at: container, withIntermediateDirectories: true)
let projectFile = container.appendingPathComponent("s8_project.bookmark")
let logFile = container.appendingPathComponent("s8_log.txt")

var log: [String] = []
func say(_ s: String) { log.append(s); print(s) }
func flush() {
    try? log.joined(separator: "\n").write(to: logFile, atomically: true, encoding: .utf8)
    print("\n(log: \(logFile.path))")
}

say("sandboxed: \(ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] != nil ? "yes" : "unknown")")
say("container: \(container.path)")

// ---------------------------------------------------------------------------
// PHASE 1 — record: store a security-scoped bookmark for a user-granted file.
//
// The file arrives via LaunchServices (`open -a App file`), which is a
// user-intent grant, NOT via argv -- a sandboxed process has no access to an
// arbitrary path handed to it on the command line, as the direct-exec run
// demonstrates. Receiving it requires an NSApplication event loop.
// ---------------------------------------------------------------------------
func record(_ url: URL) {
    say("\n== RECORD: \(url.path)")
    do {
        let bm = try url.bookmarkData(options: .withSecurityScope,
                                      includingResourceValuesForKeys: nil,
                                      relativeTo: nil)
        try bm.write(to: projectFile)
        say("  bookmark created: \(bm.count) bytes -> \(projectFile.lastPathComponent)")
        say("  (this is what AD-9 stores in the project file alongside the path)")
    } catch {
        say("  FAILED to create bookmark: \(error.localizedDescription)")
        say("  -> no user-granted access to this path")
    }
}

final class OpenDelegate: NSObject, NSApplicationDelegate {
    func application(_ s: NSApplication, open urls: [URL]) {
        for u in urls { record(u) }
        flush()
        NSApp.terminate(nil)
    }
    func applicationDidFinishLaunching(_ n: Notification) {
        // If nothing arrives, do not hang a test.
        DispatchQueue.main.asyncAfter(deadline: .now() + 6) {
            say("  no file delivered by LaunchServices within 6s")
            flush(); NSApp.terminate(nil)
        }
    }
}

if CommandLine.arguments.count > 1, CommandLine.arguments[1] == "--direct" {
    // Deliberate negative control: prove the sandbox denies an arbitrary path.
    record(URL(fileURLWithPath: CommandLine.arguments[2]))
    flush(); exit(0)
}

if !CommandLine.arguments.contains("--resolve") {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let d = OpenDelegate()
    app.delegate = d
    app.run()
    exit(0)
}

// ---------------------------------------------------------------------------
// PHASE 2 — resolve after "relaunch" and test access, parent AND child.
// ---------------------------------------------------------------------------
say("\n== RESOLVE (simulating a relaunch)")
guard let bm = try? Data(contentsOf: projectFile) else {
    say("  no stored bookmark; run the record phase first"); flush(); exit(1)
}
var stale = false
guard let url = try? URL(resolvingBookmarkData: bm, options: .withSecurityScope,
                         relativeTo: nil, bookmarkDataIsStale: &stale) else {
    say("  FAILED to resolve bookmark"); flush(); exit(1)
}
say("  resolved: \(url.path)")
say("  stale: \(stale)")

let scoped = url.startAccessingSecurityScopedResource()
say("  startAccessingSecurityScopedResource: \(scoped)")
defer { if scoped { url.stopAccessingSecurityScopedResource() } }

// Parent access
do {
    let fh = try FileHandle(forReadingFrom: url)
    let head = fh.readData(ofLength: 16)
    try? fh.close()
    say("  PARENT read: OK (\(head.count) bytes)")
} catch {
    say("  PARENT read: FAILED — \(error.localizedDescription)")
}

// Child access — the architecturally critical test
let workerURL = Bundle.main.bundleURL
    .appendingPathComponent("Contents/MacOS/s8-worker")
say("\n== CHILD PROCESS (does it inherit access?)")
if fm.fileExists(atPath: workerURL.path) {
    let p = Process()
    p.executableURL = workerURL
    p.arguments = [url.path]
    let pipe = Pipe()
    p.standardOutput = pipe
    p.standardError = pipe
    do {
        try p.run()
        let out = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        let text = String(data: out, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? "(no output)"
        say("  worker exit \(p.terminationStatus): \(text)")
        if p.terminationStatus == 0 {
            say("  -> child INHERITS access. Worker API may take a PATH.")
        } else {
            say("  -> child has NO access. Worker API must take a FILE DESCRIPTOR.")
        }
    } catch {
        say("  could not spawn worker: \(error.localizedDescription)")
    }
} else {
    say("  worker not found at \(workerURL.path)")
}

// Descriptor-passing fallback: works regardless of sandbox inheritance.
say("\n== FD INHERITANCE (the fallback if paths do not work)")
if let fh = try? FileHandle(forReadingFrom: url) {
    let p = Process()
    p.executableURL = workerURL
    p.arguments = ["--fd", "3"]
    p.standardInput = fh
    let pipe = Pipe(); p.standardOutput = pipe; p.standardError = pipe
    if (try? p.run()) != nil {
        let out = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        say("  via stdin: exit \(p.terminationStatus) — " +
            (String(data: out, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""))
    }
    try? fh.close()
}
flush()
