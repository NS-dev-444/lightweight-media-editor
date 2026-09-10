import Foundation
import AVFoundation

/// §3's voiceover recording.
///
/// Records the default input to a WAV beside the project and hands back the
/// path, which the document then imports like any other audio file. Writing a
/// plain file rather than holding samples in memory is deliberate: a voiceover
/// is a take, and a take that only exists until the app quits is a take you
/// will lose.
///
/// **Permission is asked for once, when recording is first attempted** — not at
/// launch. An editor that demands microphone access before you have asked it to
/// record is an editor people distrust.
@MainActor
final class VoiceRecorder: ObservableObject {

    enum State: Equatable {
        case idle, requesting, recording, denied
        var isRecording: Bool { self == .recording }
    }

    @Published private(set) var state: State = .idle
    @Published private(set) var status = ""
    /// Live input level, 0-1, for a meter. Without one you cannot tell a silent
    /// take from a working one until you play it back.
    @Published private(set) var level: Float = 0
    @Published private(set) var seconds: Double = 0

    private var engine: AVAudioEngine?
    private var file: AVAudioFile?
    private var started: Date?
    private var timer: Timer?

    /// Start recording into `url`. `finished` is called with the file's path on
    /// success, or nil if nothing usable was recorded.
    func start(to url: URL, finished: @escaping (String?) -> Void) {
        guard state == .idle else { return }
        state = .requesting
        status = "Asking for permission to use the microphone…"
        AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
            Task { @MainActor in
                guard let self else { return }
                guard granted else {
                    self.state = .denied
                    self.status = "This Mac has not given the app permission to use the "
                                + "microphone. You can grant it in System Settings, under "
                                + "Privacy & Security."
                    finished(nil)
                    return
                }
                self.begin(url: url, finished: finished)
            }
        }
    }

    private func begin(url: URL, finished: @escaping (String?) -> Void) {
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0 else {
            state = .idle
            status = "No microphone is available."
            finished(nil)
            return
        }

        // WAV at the input's own rate: no resampling on the way in, so the take
        // on disk is exactly what was heard. Import resamples to the project
        // rate afterwards, once, where AD-7 says resampling belongs.
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: format.sampleRate,
            AVNumberOfChannelsKey: min(format.channelCount, 2),
            AVLinearPCMBitDepthKey: 24,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]
        guard let file = try? AVAudioFile(forWriting: url, settings: settings) else {
            state = .idle
            status = "The recording could not be started — that folder may not be writable."
            finished(nil)
            return
        }

        self.file = file
        self.engine = engine
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buf, _ in
            try? file.write(from: buf)
            let peak = Self.peak(of: buf)
            Task { @MainActor in self?.level = peak }
        }
        do {
            try engine.start()
        } catch {
            state = .idle
            status = "The microphone could not be started."
            self.engine = nil
            self.file = nil
            finished(nil)
            return
        }
        started = Date()
        state = .recording
        status = "Recording…"
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, let started = self.started else { return }
                self.seconds = Date().timeIntervalSince(started)
            }
        }
        onFinish = finished
    }

    private var onFinish: ((String?) -> Void)?

    func stop() {
        guard state == .recording else { return }
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        timer?.invalidate(); timer = nil
        let url = file?.url
        engine = nil
        file = nil            // closes the file, writing the WAV header
        state = .idle
        level = 0
        let length = seconds
        seconds = 0
        started = nil

        // A tap that never fired leaves a header-only file. Importing that
        // produces a zero-length clip, which looks like a bug rather than like
        // "nothing was recorded".
        guard let url, length > 0.25 else {
            status = "Nothing was recorded."
            if let url { try? FileManager.default.removeItem(at: url) }
            onFinish?(nil)
            onFinish = nil
            return
        }
        status = String(format: "Recorded %.1f seconds", length)
        onFinish?(url.path)
        onFinish = nil
    }

    /// Peak of a buffer, for the meter.
    private nonisolated static func peak(of buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData else { return 0 }
        let n = Int(buffer.frameLength)
        var peak: Float = 0
        for ch in 0..<Int(buffer.format.channelCount) {
            let p = data[ch]
            for i in 0..<n { peak = max(peak, abs(p[i])) }
        }
        return min(peak, 1)
    }
}
