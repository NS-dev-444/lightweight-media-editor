import Foundation
import VideoToolbox
import ImageIO
import UniformTypeIdentifiers
import CoreVideo

/// §31's Extract Frames tool — pull stills out of a video.
///
/// **Why this does not go through the conversion queue.** Every other tool is a
/// preset over the same converter (PRODUCT_DIRECTION §4), because every other
/// tool turns one file into one file. This one turns one file into many, so a
/// job's single `output` path and single progress figure do not describe it.
/// Sharing the machinery would mean bending it, which is worse than a small
/// dedicated path.
///
/// **Why ImageIO rather than FFmpeg.** The shipped LGPL build has no PNG or
/// MJPEG encoder — `--disable-autodetect` leaves them out, and
/// `this_build_has_no_still_image_encoder` in the media tests pins that fact so
/// a future build change cannot quietly invalidate this comment. It also
/// matches what Phase 3 found reading stills, where FFmpeg reported
/// "unspecified size" for a perfectly valid PNG and ImageIO replaced it.
/// Stills go through the platform's imaging framework in both directions.
@MainActor
final class FrameExtractor: ObservableObject {

    enum Format: String, CaseIterable, Identifiable, Sendable {
        case png = "PNG", jpeg = "JPEG"
        var id: String { rawValue }
        var ext: String { self == .png ? "png" : "jpg" }
        var type: UTType { self == .png ? .png : .jpeg }
    }

    @Published private(set) var isRunning = false
    @Published private(set) var progress: Double = 0
    @Published private(set) var status = ""
    @Published private(set) var written = 0

    /// Cancellation is read from a background thread while the extraction runs,
    /// so it cannot live on the main actor.
    final class CancelFlag: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var isSet: Bool { lock.lock(); defer { lock.unlock() }; return value }
        func set() { lock.lock(); value = true; lock.unlock() }
    }
    private var flag = CancelFlag()

    func cancel() { flag.set() }

    /// Extract one frame every `interval` seconds into `directory`.
    ///
    /// An interval of 0 would mean every frame, which for a ten-minute clip is
    /// tens of thousands of files — so the caller picks an interval and `limit`
    /// stops a long video from filling a disk by surprise.
    func run(input: String, directory: URL, interval: Double,
             format: Format, limit: Int = 2000) {
        guard !isRunning else { return }
        isRunning = true
        flag = CancelFlag()
        written = 0
        progress = 0
        status = "Reading the video…"

        let step = max(0.05, interval)
        let base = ((input as NSString).lastPathComponent as NSString).deletingPathExtension

        let flag = self.flag
        let report: @Sendable (Int, Double) -> Void = { [weak self] n, p in
            Task { @MainActor in
                guard let self else { return }
                self.written = n
                self.progress = p
            }
        }
        let finish: @Sendable (String) -> Void = { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                self.isRunning = false
                self.progress = 1
                self.status = result
            }
        }
        // The detached work captures the two callbacks and nothing else, so
        // there is no reference to the extractor crossing threads.
        Task.detached(priority: .userInitiated) {
            finish(Self.extract(input: input, directory: directory, base: base,
                                step: step, format: format, limit: limit,
                                cancelled: flag, report: report))
        }
    }

    private nonisolated static func extract(input: String, directory: URL, base: String,
                                            step: Double, format: Format, limit: Int,
                                            cancelled: CancelFlag,
                                            report: @Sendable @escaping (Int, Double) -> Void) -> String {
        guard let d = mc_open(input) else {
            return "This video could not be opened."
        }
        defer { mc_close(d) }

        var info = MCInfo()
        guard mc_info(d, &info) == 0, info.width > 0 else {
            return "This file has no video to take frames from."
        }
        let duration = info.duration_sec > 0 ? info.duration_sec : Double(limit) * step
        let total = min(limit, max(1, Int(duration / step)))

        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        } catch {
            return "That folder could not be created."
        }

        var n = 0
        for i in 0..<total {
            if cancelled.isSet { break }
            let seconds = Double(i) * step
            _ = mc_seek(d, Int64(seconds * 1e9))

            // A seek lands on the preceding keyframe, so run forward to the
            // frame actually asked for.
            var picked: CVPixelBuffer?
            var guardCount = 0
            while guardCount < 240 {
                guardCount += 1
                var raw: UnsafeRawPointer?
                var pts: Int64 = 0
                if mc_next_frame(d, &raw, &pts) <= 0 { break }
                guard let raw else { continue }
                picked = Unmanaged<CVPixelBuffer>.fromOpaque(raw).takeRetainedValue()
                if Double(pts) / 1e9 >= seconds { break }
            }
            guard let pb = picked else { break }

            var cg: CGImage?
            guard VTCreateCGImageFromCVPixelBuffer(pb, options: nil, imageOut: &cg) == noErr,
                  let image = cg else { continue }

            let name = String(format: "%@-%04d.%@", base, i + 1, format.ext)
            let url = directory.appendingPathComponent(name)
            guard let dest = CGImageDestinationCreateWithURL(
                    url as CFURL, format.type.identifier as CFString, 1, nil) else { continue }
            // JPEG at 0.9 is visually indistinguishable at a fraction of PNG's
            // size; PNG ignores the hint.
            CGImageDestinationAddImage(dest, image,
                                       [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
            guard CGImageDestinationFinalize(dest) else { continue }

            n += 1
            report(n, Double(i + 1) / Double(total))
        }

        if cancelled.isSet { return "Stopped after \(n) frame\(n == 1 ? "" : "s")." }
        if n == 0 { return "No frames could be read from this video." }
        return "Saved \(n) frame\(n == 1 ? "" : "s") to \(directory.lastPathComponent)."
    }
}
