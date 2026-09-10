import AVFoundation
import Foundation

/// Real-time audio playback for the preview.
///
/// A source node pulls mixed samples on the audio render thread. That thread
/// has a hard deadline and must never block, so it does NO decoding — it copies
/// from a ring buffer that a background thread keeps filled. Decoding on the
/// render thread is the classic cause of audio glitching.
final class AudioPlayer {
    private let engine = AVAudioEngine()
    private var sourceNode: AVAudioSourceNode?
    private let mixer: AudioMixer
    private let sampleRate: Double = 48_000

    /// Ring buffer, interleaved stereo f32.
    private var ring: [Float]
    private var writeIndex = 0
    private var readIndex = 0
    private let ringLock = NSLock()
    private let capacityFrames = 48_000        // one second of slack

    private var producer: Thread?
    private var running = false
    private var position: Int64 = 0
    private var layersProvider: (() -> [MCPlanLayer])?
    private var pathProvider: ((UInt64) -> String?)?

    init(mixer: AudioMixer) {
        self.mixer = mixer
        self.ring = [Float](repeating: 0, count: capacityFrames * 2)
    }

    /// Start playing from `ticks`.
    func start(at ticks: Int64,
               layers: @escaping () -> [MCPlanLayer],
               assetPath: @escaping (UInt64) -> String?) {
        stop()
        position = ticks
        layersProvider = layers
        pathProvider = assetPath
        mixer.seekAll(to: ticks)

        ringLock.lock(); writeIndex = 0; readIndex = 0
        for i in 0..<ring.count { ring[i] = 0 }
        ringLock.unlock()

        running = true
        let t = Thread { [weak self] in self?.produce() }
        t.stackSize = 512 * 1024
        producer = t
        t.start()

        guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 2)
        else { return }

        let node = AVAudioSourceNode(format: format) { [weak self] _, _, frameCount, buffers -> OSStatus in
            guard let self else { return noErr }
            let abl = UnsafeMutableAudioBufferListPointer(buffers)
            let frames = Int(frameCount)

            // Render-thread work: a lock and a copy. No decoding, no allocation.
            self.ringLock.lock()
            let available = (self.writeIndex - self.readIndex + self.ring.count) % self.ring.count
            let take = min(frames * 2, available)
            var scratch = [Float](repeating: 0, count: frames * 2)
            for i in 0..<take {
                scratch[i] = self.ring[(self.readIndex + i) % self.ring.count]
            }
            self.readIndex = (self.readIndex + take) % self.ring.count
            self.ringLock.unlock()

            // Non-interleaved output: split into per-channel buffers.
            if abl.count >= 2 {
                let l = abl[0].mData!.assumingMemoryBound(to: Float.self)
                let r = abl[1].mData!.assumingMemoryBound(to: Float.self)
                for i in 0..<frames {
                    l[i] = scratch[i * 2]
                    r[i] = scratch[i * 2 + 1]
                }
            } else if let mono = abl.first?.mData?.assumingMemoryBound(to: Float.self) {
                for i in 0..<frames { mono[i] = (scratch[i * 2] + scratch[i * 2 + 1]) * 0.5 }
            }
            return noErr
        }

        sourceNode = node
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)
        do { try engine.start() } catch { stop() }
    }

    func stop() {
        running = false
        if engine.isRunning { engine.stop() }
        if let n = sourceNode { engine.detach(n) }
        sourceNode = nil
        producer = nil
    }

    /// Keeps the ring buffer fed, off the render thread.
    private func produce() {
        let chunkFrames = 2048
        while running {
            ringLock.lock()
            let used = (writeIndex - readIndex + ring.count) % ring.count
            let free = ring.count - used - 2
            ringLock.unlock()

            if free < chunkFrames * 2 {
                usleep(4_000)                 // buffer is full; wait for the device
                continue
            }

            let layers = layersProvider?() ?? []
            let paths = pathProvider ?? { _ in nil }
            let mixed = mixer.mix(layers: layers, assetPath: paths, frameCount: chunkFrames)

            ringLock.lock()
            for v in mixed {
                ring[writeIndex] = v
                writeIndex = (writeIndex + 1) % ring.count
            }
            ringLock.unlock()

            position += Int64(Double(chunkFrames) / sampleRate * Double(TICKS_PER_SECOND))
        }
    }
}
