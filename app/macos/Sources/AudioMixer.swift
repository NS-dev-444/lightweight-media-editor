import Foundation

/// Mixes the audio layers of a render plan.
///
/// AD-7: one audio stack, f32 internally, one project sample rate with
/// everything resampled on import. The decoder side does the resampling, so by
/// the time audio reaches here every source is already stereo f32 at the
/// project rate and mixing is a straight weighted sum.
///
/// Clip gain and fades arrive already combined in `MCPlanLayer.gain`, computed
/// by the core — the mixer applies what it is told rather than re-deriving it,
/// so preview, export and the model can never disagree about a fade.
final class AudioMixer {
    private var readers: [UInt64: OpaquePointer] = [:]
    private let sampleRate: Int32
    private let lock = NSLock()

    /// Audio decoders, like video decoders, are not safe for concurrent use.
    init(sampleRate: Int32 = 48_000) { self.sampleRate = sampleRate }

    private func reader(for asset: UInt64, path: String) -> OpaquePointer? {
        if let r = readers[asset] { return r }
        guard let r = mc_audio_open(path, sampleRate) else { return nil }
        readers[asset] = r
        return r
    }

    /// Mix `frameCount` stereo sample-frames for the layers active at a time.
    ///
    /// Returns interleaved stereo f32. Silence when nothing is playing — which
    /// must still be WRITTEN, or the audio track drifts out of sync with video.
    func mix(layers: [MCPlanLayer],
             assetPath: (UInt64) -> String?,
             frameCount: Int) -> [Float] {
        lock.lock()
        defer { lock.unlock() }

        var out = [Float](repeating: 0, count: frameCount * 2)
        var scratch = [Float](repeating: 0, count: frameCount * 2)

        for layer in layers where layer.kind == 1 {
            guard let path = assetPath(layer.asset_id),
                  let r = reader(for: layer.asset_id, path: path) else { continue }

            let n = scratch.withUnsafeMutableBufferPointer { buf in
                Int(mc_audio_read(r, buf.baseAddress, Int32(frameCount)))
            }
            if n <= 0 { continue }
            let gain = Float(layer.gain)
            for i in 0..<(n * 2) { out[i] += scratch[i] * gain }
        }

        // Clip rather than wrap. Summed layers can exceed full scale, and
        // wrapping produces loud digital crackle where clipping is merely loud.
        for i in 0..<out.count { out[i] = min(max(out[i], -1.0), 1.0) }
        return out
    }

    /// Position every reader for a fresh sequential pass.
    func seekAll(to ticks: Int64) {
        lock.lock(); defer { lock.unlock() }
        let ns = Int64(Double(ticks) / Double(TICKS_PER_SECOND) * 1e9)
        for (_, r) in readers { _ = mc_audio_seek(r, ns) }
    }

    func closeAll() {
        lock.lock(); defer { lock.unlock() }
        for (_, r) in readers { mc_audio_close(r) }
        readers.removeAll()
    }

    deinit { for (_, r) in readers { mc_audio_close(r) } }
}
