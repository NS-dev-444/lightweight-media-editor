import Foundation
import AppKit

/// §31's merge tool — join several videos into one.
///
/// Like Extract Frames, and unlike everything else in TOOLS, this does not run
/// through the conversion queue: a queue job is one file in, one file out, and
/// merge is many in, one out. The queue's shape would have to be bent to hold
/// it, which is worse than a small dedicated path.
///
/// The **order matters and is the user's**. Files are joined exactly as listed,
/// never re-sorted, because "part 1, part 2, part 3" has to mean what it says.
@MainActor
final class Merger: ObservableObject {
    @Published private(set) var isRunning = false
    @Published private(set) var progress: Double = 0
    @Published private(set) var status = ""
    /// The chosen files, in the order they will be joined.
    @Published var inputs: [URL] = []

    final class CancelFlag: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var isSet: Bool { lock.lock(); defer { lock.unlock() }; return value }
        func set() { lock.lock(); value = true; lock.unlock() }
    }
    private var flag = CancelFlag()
    func cancel() { flag.set() }

    func add(_ urls: [URL]) {
        inputs.append(contentsOf: urls.filter { u in !inputs.contains(u) })
        status = ""
    }
    func remove(at i: Int) { if inputs.indices.contains(i) { inputs.remove(at: i) } }
    func move(from i: Int, to j: Int) {
        guard inputs.indices.contains(i), j >= 0, j < inputs.count else { return }
        let u = inputs.remove(at: i)
        inputs.insert(u, at: j)
    }
    func clear() { inputs.removeAll(); status = "" }

    /// Whether this merge will copy the streams (instant, lossless) or
    /// re-encode. Worth saying before starting: they are minutes apart.
    var wouldCopy: Bool {
        guard inputs.count > 1 else { return true }
        let paths = inputs.map { strdup($0.path) }
        defer { paths.forEach { free($0) } }
        var ptrs = paths.map { UnsafePointer<CChar>($0) }
        return ptrs.withUnsafeMutableBufferPointer { b in
            mc_merge_would_copy(b.baseAddress, Int32(b.count)) != 0
        }
    }

    func run(to output: URL) {
        guard !isRunning, !inputs.isEmpty else { return }
        isRunning = true
        flag = CancelFlag()
        progress = 0
        status = "Joining…"

        let paths = inputs.map(\.path)
        let flag = self.flag
        let report: @Sendable (Double) -> Void = { [weak self] p in
            Task { @MainActor in self?.progress = p }
        }
        let finish: @Sendable (String) -> Void = { [weak self] message in
            Task { @MainActor in
                guard let self else { return }
                self.isRunning = false
                self.progress = 1
                self.status = message
            }
        }
        Task.detached(priority: .userInitiated) {
            finish(Self.merge(paths: paths, output: output.path,
                              cancelled: flag, report: report))
        }
    }

    private nonisolated static func merge(paths: [String], output: String,
                                          cancelled: CancelFlag,
                                          report: @Sendable @escaping (Double) -> Void) -> String {
        let c = paths.map { strdup($0) }
        defer { c.forEach { free($0) } }
        var ptrs = c.map { UnsafePointer<CChar>($0) }

        let handle: OpaquePointer? = ptrs.withUnsafeMutableBufferPointer { b in
            mc_merge_open(b.baseAddress, Int32(b.count), output)
        }
        guard let m = handle else {
            return "These files could not be joined. One of them may be damaged, "
                 + "or not a video at all."
        }
        defer { _ = mc_merge_close(m) }

        var n = 0
        while true {
            let rc = mc_merge_step(m)
            if rc == 0 { break }
            if rc < 0 {
                let msg = mc_merge_error(m).flatMap { String(cString: $0) } ?? ""
                return msg.isEmpty ? "The joined file could not be written." : msg
            }
            n += 1
            if n % 32 == 0 {
                report(Double(mc_merge_progress(m)))
                // A cancelled merge still writes its trailer, so what exists on
                // disk is a shorter playable video rather than a broken one.
                if cancelled.isSet { return "Stopped. The part written so far is playable." }
            }
        }
        let copied = mc_merge_mode(m) == MC_MERGE_COPY
        return copied ? "Joined \(paths.count) files without re-encoding."
                      : "Joined \(paths.count) files."
    }
}
