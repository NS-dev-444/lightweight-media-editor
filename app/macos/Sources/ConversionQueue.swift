import Foundation

/// §21's conversion queue.
///
/// Requirements from the spec, and how each is met:
///   * **Pause** — between items only. Pausing mid-encode is not generally
///     possible, and §46 Rule 11 says to document the limitation rather than
///     imply a capability. The in-flight item finishes or is cancelled.
///   * **Cancel** — per item and queue-wide; a cancelled job still writes its
///     trailer, so the partial file is playable rather than corrupt.
///   * **Retry** — a failed item is requeued without re-adding it.
///   * **Errors** — human sentences (§23), never codec numbers.
///   * **Overwrite protection** — refuses to clobber an existing file (§4).
@MainActor
final class ConversionQueue: ObservableObject {

    enum State: Equatable {
        case waiting, running, done, failed(String), cancelled
        var isTerminal: Bool {
            switch self { case .done, .failed, .cancelled: return true; default: return false }
        }
        var label: String {
            switch self {
            case .waiting: return "Waiting"
            case .running: return "Converting…"
            case .done: return "Done"
            case .cancelled: return "Cancelled"
            case .failed(let m): return m
            }
        }
    }

    struct OutputFormat: Identifiable, Hashable {
        let id: String
        let name: String
        let ext: String
        /// nil = container conversion (video); otherwise an audio codec index.
        let audioCodec: Int32?

        static let all: [OutputFormat] = [
            OutputFormat(id: "mp4",  name: "MP4 (video)", ext: "mp4",  audioCodec: nil),
            OutputFormat(id: "mov",  name: "MOV (video)", ext: "mov",  audioCodec: nil),
            OutputFormat(id: "mkv",  name: "MKV (video)", ext: "mkv",  audioCodec: nil),
            OutputFormat(id: "mp3",  name: "MP3 (audio)", ext: "mp3",  audioCodec: 0),
            OutputFormat(id: "m4a",  name: "M4A / AAC",   ext: "m4a",  audioCodec: 1),
            OutputFormat(id: "wav",  name: "WAV",         ext: "wav",  audioCodec: 2),
            OutputFormat(id: "flac", name: "FLAC",        ext: "flac", audioCodec: 3),
        ]
    }

    /// What the resize and compress tools ask for. Absent means "keep the
    /// source as it is", which lets the lossless copy path stay available.
    struct VideoSettings: Hashable, Codable {
        /// 0 keeps the source size; giving one axis scales the other to match.
        var width: Int32 = 0
        var height: Int32 = 0
        /// 0 picks a bitrate from the frame size and rate.
        var bitrateKbps: Int32 = 0
        /// 0 = HEVC (better at a given bitrate), 1 = H.264 (plays everywhere).
        var codec: Int32 = 0
    }

    struct Job: Identifiable {
        let id = UUID()
        let input: String
        var output: String
        var format: OutputFormat
        var state: State = .waiting
        var progress: Double = 0
        /// True when this is a remux — instant and lossless.
        var willRemux: Bool = false
        /// Set only by tools that must re-encode.
        var video: VideoSettings?
        var inputName: String { (input as NSString).lastPathComponent }
        var outputName: String { (output as NSString).lastPathComponent }
    }

    @Published private(set) var jobs: [Job] = []
    @Published private(set) var isRunning = false
    @Published var format: OutputFormat = OutputFormat.all[0]
    @Published var outputDirectory: URL?
    @Published var overwrite = false

    private var cancelledJobs: Set<UUID> = []
    private var pauseRequested = false

    init() { jobs = Self.loadStored() }

    // ---- queue management --------------------------------------------------

    func add(paths: [String], video: VideoSettings? = nil, suffix: String = "") {
        for p in paths {
            let dir = outputDirectory?.path ?? (p as NSString).deletingLastPathComponent
            let base = ((p as NSString).lastPathComponent as NSString).deletingPathExtension
            var out = "\(dir)/\(base)\(suffix).\(format.ext)"
            // Never write over the file being converted. Converting an MP4 to
            // MP4 in its own directory lands on exactly the input path, and
            // with overwrite on that destroys the source while reading it.
            if out == p { out = "\(dir)/\(base)-converted.\(format.ext)" }
            var job = Job(input: p, output: out, format: format)
            job.video = video
            // Say up front whether this will be instant or slow. Anything that
            // changes the picture cannot be a copy, however compatible the
            // codecs are.
            job.willRemux = format.audioCodec == nil && video == nil
                && mc_convert_would_remux(p, out) != 0
            jobs.append(job)
        }
        persist()
    }

    func remove(_ id: UUID) {
        cancelledJobs.insert(id)
        jobs.removeAll { $0.id == id }
        persist()
    }

    func clearFinished() {
        jobs.removeAll { $0.state.isTerminal }
        persist()
    }

    func cancel(_ id: UUID) {
        cancelledJobs.insert(id)
        if let i = jobs.firstIndex(where: { $0.id == id }), !jobs[i].state.isTerminal {
            jobs[i].state = .cancelled
        }
        persist()
    }

    func retry(_ id: UUID) {
        guard let i = jobs.firstIndex(where: { $0.id == id }) else { return }
        cancelledJobs.remove(id)
        jobs[i].state = .waiting
        jobs[i].progress = 0
        persist()
    }

    /// §21: pause happens BETWEEN items; the running item finishes.
    func requestPause() { pauseRequested = true }

    func cancelAll() {
        for j in jobs where !j.state.isTerminal { cancelledJobs.insert(j.id) }
        pauseRequested = true
    }

    func start() {
        guard !isRunning else { return }
        pauseRequested = false
        isRunning = true
        Task { await runQueue() }
    }

    // ---- execution ---------------------------------------------------------

    private func runQueue() async {
        while let index = jobs.firstIndex(where: { $0.state == .waiting }) {
            if pauseRequested { break }
            let job = jobs[index]
            if cancelledJobs.contains(job.id) { jobs[index].state = .cancelled; continue }

            if !overwrite && FileManager.default.fileExists(atPath: job.output) {
                jobs[index].state = .failed("A file with that name already exists.")
                continue
            }
            jobs[index].state = .running

            let id = job.id
            let snapshot = job
            let result: State = await Task.detached(priority: .userInitiated) {
                await Self.convert(snapshot) { p in
                    Task { @MainActor [weak self] in
                        if let i = self?.jobs.firstIndex(where: { $0.id == id }) {
                            self?.jobs[i].progress = p
                        }
                    }
                } cancelled: { await self.isCancelled(id) }
            }.value

            if let i = jobs.firstIndex(where: { $0.id == id }) {
                jobs[i].state = cancelledJobs.contains(id) ? .cancelled : result
                if jobs[i].state == .done { jobs[i].progress = 1 }
            }
            persist()
        }
        isRunning = false
        pauseRequested = false
        persist()
    }

    private func isCancelled(_ id: UUID) -> Bool { cancelledJobs.contains(id) }

    // ---- persistence (§21) -------------------------------------------------
    //
    // A batch of conversions is exactly the kind of work that outlives a
    // session: it is long, it is unattended, and closing the window while it
    // runs is a normal thing to do. Losing the list on quit means rebuilding it
    // by hand.
    //
    // Unfinished work is restored as **waiting**, never restarted. Resuming an
    // encode on launch would write files the user did not ask for at that
    // moment, possibly over something they have since put there. The list comes
    // back; pressing Convert stays their decision.

    private struct StoredJob: Codable {
        var input: String
        var output: String
        var formatID: String
        var failed: String?
        var video: VideoSettings?
    }

    private static var storeURL: URL? {
        guard let base = FileManager.default.urls(for: .applicationSupportDirectory,
                                                  in: .userDomainMask).first else { return nil }
        let dir = base.appendingPathComponent("dev.mediacore.editor", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("queue.json")
    }

    private static func loadStored() -> [Job] {
        guard let url = storeURL, let data = try? Data(contentsOf: url),
              let stored = try? JSONDecoder().decode([StoredJob].self, from: data)
        else { return [] }
        return stored.compactMap { s in
            guard let f = OutputFormat.all.first(where: { $0.id == s.formatID }) else { return nil }
            // A source that has since been moved or deleted is dropped rather
            // than restored as a job that can only fail.
            guard FileManager.default.fileExists(atPath: s.input) else { return nil }
            var j = Job(input: s.input, output: s.output, format: f)
            j.video = s.video
            j.willRemux = s.video == nil && f.audioCodec == nil
                && mc_convert_would_remux(s.input, s.output) != 0
            if let m = s.failed { j.state = .failed(m) }
            return j
        }
    }

    /// Write the unfinished part of the queue. Finished and cancelled items are
    /// history, not work, so they are not carried across launches.
    private func persist() {
        guard let url = Self.storeURL else { return }
        let keep: [StoredJob] = jobs.compactMap { j in
            switch j.state {
            case .waiting, .running:
                return StoredJob(input: j.input, output: j.output,
                                 formatID: j.format.id, failed: nil, video: j.video)
            case .failed(let m):
                return StoredJob(input: j.input, output: j.output,
                                 formatID: j.format.id, failed: m, video: j.video)
            case .done, .cancelled:
                return nil
            }
        }
        if keep.isEmpty {
            try? FileManager.default.removeItem(at: url)
            return
        }
        if let data = try? JSONEncoder().encode(keep) {
            try? data.write(to: url, options: .atomic)
        }
    }

    private static func convert(_ job: Job,
                                progress: @escaping (Double) -> Void,
                                cancelled: @escaping () async -> Bool) async -> State {
        if let codec = job.format.audioCodec {
            guard let c = mc_audio_convert_open(job.input, job.output, codec, 192, 48_000) else {
                return .failed("This file has no audio that could be converted.")
            }
            defer { _ = mc_audio_convert_close(c) }
            var steps = 0
            while true {
                let rc = mc_audio_convert_step(c)
                if rc == 0 { break }
                if rc < 0 { return .failed("This file could not be converted. It may be damaged.") }
                steps += 1
                if steps % 8 == 0 {
                    progress(min(0.99, Double(steps) / 200.0))
                    if await cancelled() { return .cancelled }
                }
            }
            return .done
        }

        // Copy the streams when nothing about the picture is changing and the
        // container will take them: near-instant and lossless, and the most
        // common conversion people actually ask for.
        if job.video == nil, let c = mc_convert_open(job.input, job.output, 1) {
            defer { _ = mc_convert_close(c) }
            var n = 0
            while true {
                let rc = mc_convert_step(c)
                if rc == 0 { break }
                if rc < 0 { return .failed("The converted file could not be written.") }
                n += 1
                if n % 32 == 0 {
                    progress(Double(mc_convert_progress(c)))
                    if await cancelled() { return .cancelled }
                }
            }
            return .done
        }

        // Otherwise re-encode.
        let v = job.video ?? VideoSettings()
        guard let c = mc_video_convert_open(job.input, job.output, v.codec,
                                            v.bitrateKbps, v.width, v.height) else {
            return .failed("This file could not be converted to that format. "
                         + "It may have no video track, or use a format this "
                         + "system cannot read.")
        }
        defer { _ = mc_video_convert_close(c) }
        var n = 0
        while true {
            let rc = mc_video_convert_step(c)
            if rc == 0 { break }
            if rc < 0 {
                // §23: the core writes a sentence; use it rather than a number.
                let msg = mc_video_convert_error(c).flatMap { String(cString: $0) } ?? ""
                return .failed(msg.isEmpty ? "This video could not be converted." : msg)
            }
            n += 1
            if n % 32 == 0 {
                progress(Double(mc_video_convert_progress(c)))
                if await cancelled() { return .cancelled }
            }
        }
        return .done
    }
}
