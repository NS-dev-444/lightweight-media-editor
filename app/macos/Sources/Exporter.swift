import Foundation
import Metal
import CoreVideo
import QuartzCore

/// Renders the timeline to a file.
///
/// Every frame is composited straight into a surface the ENCODER owns. The
/// hardware path only accepts frames from its own pool — verified by
/// measurement, not assumed — and rendering into that buffer keeps export
/// zero-copy end to end, exactly like the preview.
final class Exporter {
    struct Progress { var frame: Int; var total: Int; var fps: Double }

    private let device: MTLDevice
    private let compositor: Compositor
    private var cancelled = false

    init?(device: MTLDevice?) {
        guard let device, let c = Compositor(device: device) else { return nil }
        self.device = device
        self.compositor = c
    }

    func cancel() { cancelled = true }

    /// Export synchronously. Call from a background thread.
    ///
    /// `plan` supplies the layers at a time; `frameSource` decodes; `textSpec`
    /// resolves titles. Returns nil on success, or a human-readable reason.
    func export(to path: String,
                preset: ExportPreset,
                durationTicks: Int64,
                fpsNum: Int32,
                fpsDen: Int32,
                plan: (Int64) -> [MCPlanLayer],
                assetPath: (UInt64) -> String?,
                textSpec: (UInt64) -> TextSpec?,
                imageSpec: (UInt64) -> ImageSpec?,
                lutSpec: (UInt64) -> Core.LutSpec? = { _ in nil },
                captionSpec: (Int64) -> TextSpec? = { _ in nil },
                frames: FrameSource,
                progress: @escaping (Progress) -> Void) -> String?
    {
        guard durationTicks > 0 else { return "There is nothing on the timeline to export." }
        let seconds = Double(durationTicks) / Double(TICKS_PER_SECOND)
        guard mc_encoder_available(preset.codec) != 0 else {
            return "This Mac cannot encode that format."
        }
        // §4: an export without sound is not an export. The audio stream must
        // be declared here, before the header is written.
        let audioRate: Int32 = 48_000
        guard let enc = mc_encoder_open(path, Int32(preset.width), Int32(preset.height),
                                        fpsNum, fpsDen, preset.codec,
                                        preset.bitrate(forSeconds: seconds),
                                        audioRate) else {
            return "The export file could not be created. Check the folder and free space."
        }
        defer { _ = mc_encoder_finish(enc) }

        var cache: CVMetalTextureCache?
        CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &cache)
        guard let cache else { return "The graphics system could not be prepared." }

        let fps = Double(fpsNum) / Double(max(fpsDen, 1))
        let total = Int((Double(durationTicks) / Double(TICKS_PER_SECOND) * fps).rounded())
        let started = CACurrentMediaTime()

        let mixer = AudioMixer(sampleRate: audioRate)
        defer { mixer.closeAll() }
        // Accumulate the sample budget in a double: at 30 fps and 48 kHz that
        // is 1600 exactly, but at 29.97 it is not, and rounding per frame would
        // drift audio against video over a long export.
        var audioDebt = 0.0

        for i in 0..<total {
            if cancelled { return "Export cancelled." }

            var raw: UnsafeMutableRawPointer?
            guard mc_encoder_acquire(enc, &raw) == 0, let raw else {
                return "The encoder stopped accepting frames."
            }
            let pb = Unmanaged<CVPixelBuffer>.fromOpaque(raw).takeUnretainedValue()

            var texRef: CVMetalTexture?
            let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb)
            guard CVMetalTextureCacheCreateTextureFromImage(
                    kCFAllocatorDefault, cache, pb, nil, .bgra8Unorm, w, h, 0, &texRef)
                    == kCVReturnSuccess,
                  let texRef, let target = CVMetalTextureGetTexture(texRef) else {
                return "A frame could not be prepared for encoding."
            }

            let t = Int64(Double(i) / fps * Double(TICKS_PER_SECOND))
            compositor.render(plan: plan(t), into: target,
                              assetPath: assetPath, textSpec: textSpec,
                              imageSpec: imageSpec, lutSpec: lutSpec,
                              captionSpec: captionSpec, frames: frames)

            guard mc_encoder_submit(enc) == 0 else { return "A frame could not be encoded." }

            // Audio for this frame's interval. Written even when silent, or the
            // audio track drifts out of sync with the video.
            if mc_encoder_has_audio(enc) != 0 {
                audioDebt += Double(audioRate) / fps
                let want = Int(audioDebt)
                audioDebt -= Double(want)
                if want > 0 {
                    var samples = mixer.mix(layers: plan(t), assetPath: assetPath,
                                            frameCount: want)
                    samples.withUnsafeMutableBufferPointer { buf in
                        _ = mc_encoder_push_audio(enc, buf.baseAddress, Int32(want))
                    }
                }
            }

            if i % 10 == 0 || i == total - 1 {
                let elapsed = CACurrentMediaTime() - started
                progress(Progress(frame: i + 1, total: total,
                                  fps: elapsed > 0 ? Double(i + 1) / elapsed : 0))
            }
        }
        return nil
    }
}
