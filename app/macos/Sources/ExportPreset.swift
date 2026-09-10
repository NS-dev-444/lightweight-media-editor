import Foundation

/// §5 / §12 export presets.
///
/// "The user should not need to understand codecs or bitrate to produce a good
/// result." So presets are named by DESTINATION (PRODUCT_DIRECTION.md §7), and
/// the bitrates come from S4b's measurements rather than from intuition.
///
/// S4b measured hardware HEVC needing ~1.74x libx264's bitrate for equal
/// quality (and hardware H.264 ~2.31x), so these carry that multiplier. Quoting
/// software-encoder numbers here would ship visibly worse files.
///
/// Deliberately free of Metal, CoreVideo and the FFI, so the arithmetic below
/// can be tested on its own — see `tools/swift_selftest/`.
struct ExportPreset: Identifiable, Hashable {
    let id: String
    let name: String
    let width: Int
    let height: Int
    /// 0 = HEVC, 1 = H.264.
    let codec: Int32
    let bitrateKbps: Int32
    /// When set, the bitrate is solved for so the file lands under this size.
    let targetMegabytes: Double?

    /// The technical line under the name — resolution and codec, for the person
    /// who does want to know.
    var detail: String {
        let codecName = codec == 1 ? "H.264" : "HEVC"
        if let mb = targetMegabytes {
            return "\(width) × \(height) · \(codecName) · under \(Int(mb)) MB"
        }
        return "\(width) × \(height) · \(codecName) · \(bitrateKbps / 1000) Mbps"
    }

    /// Presets are named by **where the file is going**, not by its resolution
    /// (PRODUCT_DIRECTION.md §7). Users think in destinations; "1080p — Smaller
    /// File" asks them to translate. The numbers are still there in `detail`
    /// for anyone who wants them.
    ///
    /// The vertical and square entries set the CANVAS. Footage shot landscape
    /// is pillarboxed inside it unless the clips are reframed — which is what
    /// the Framing inspector and Timeline ▸ Reframe All are for.
    static let all: [ExportPreset] = [
        ExportPreset(id: "yt-4k", name: "YouTube 4K",
                     width: 3840, height: 2160, codec: 0, bitrate: 60_000),
        ExportPreset(id: "yt-1080", name: "YouTube 1080p",
                     width: 1920, height: 1080, codec: 0, bitrate: 16_000),
        ExportPreset(id: "reel", name: "Reels, TikTok, Shorts",
                     width: 1080, height: 1920, codec: 0, bitrate: 12_000),
        ExportPreset(id: "square", name: "Square Post",
                     width: 1080, height: 1080, codec: 0, bitrate: 10_000),
        ExportPreset(id: "portrait", name: "Portrait Post (4:5)",
                     width: 1080, height: 1350, codec: 0, bitrate: 11_000),
        // Solved from the timeline's length rather than guessed: 25 MB is the
        // common mail attachment ceiling, and "it was 26 MB" is a failure.
        ExportPreset(id: "email", name: "Email or Messaging",
                     width: 1280, height: 720, codec: 1, bitrate: 5_000,
                     targetMegabytes: 25),
        // H.264 for players that cannot handle HEVC. Its bitrate is higher for
        // the same quality — S4b measured 2.31x vs libx264 — and saying so
        // honestly is better than shipping a worse file under the same name.
        ExportPreset(id: "compat", name: "Maximum Compatibility",
                     width: 1920, height: 1080, codec: 1, bitrate: 22_000),
    ]

    init(id: String, name: String, width: Int, height: Int, codec: Int32,
         bitrate: Int32, targetMegabytes: Double? = nil) {
        self.id = id; self.name = name; self.width = width; self.height = height
        self.codec = codec; self.bitrateKbps = bitrate
        self.targetMegabytes = targetMegabytes
    }

    /// The bitrate to actually encode at, given how long the timeline is.
    ///
    /// A size target is a bitrate problem: bits = size × 8, and the audio track
    /// takes its share whatever the video does. The 6% headroom covers
    /// container overhead and the encoder's own overshoot — a "under 25 MB"
    /// file that comes out at 25.4 MB has failed at the only thing it promised.
    func bitrate(forSeconds seconds: Double) -> Int32 {
        guard let mb = targetMegabytes, seconds > 0.5 else { return bitrateKbps }
        let audioKbps = 192.0
        let totalKbps = (mb * 8 * 1024 * 1024 / 1000) / seconds * 0.94
        let videoKbps = totalKbps - audioKbps
        // Below about 300 kbps at 720p the result is not worth sending. Clamp
        // rather than produce something unwatchable in silence; the size
        // promise then holds for anything of a sane length.
        return Int32(videoKbps.clamped(to: Double(Self.floorKbps)...Double(bitrateKbps)))
    }

    /// Below this, 720p is not worth sending, so the size target is abandoned
    /// rather than met with something unwatchable.
    static let floorKbps: Int32 = 300

    /// Roughly how large the file will be, in MB.
    func estimatedMegabytes(forSeconds seconds: Double) -> Double {
        let kbps = Double(bitrate(forSeconds: seconds)) + 192
        return kbps * seconds * 1000 / 8 / 1024 / 1024
    }

    /// Why the size promise cannot be kept, or `nil` when it can.
    ///
    /// §46 Rule 11: state the limit rather than imply a capability. A preset
    /// called "Email or Messaging" promises a size, and a long timeline cannot
    /// have both that size and a watchable picture — so say which one is going
    /// to give, before the export runs rather than after.
    func sizeWarning(forSeconds seconds: Double) -> String? {
        guard let mb = targetMegabytes, seconds > 0.5 else { return nil }
        let estimate = estimatedMegabytes(forSeconds: seconds)
        guard estimate > mb else { return nil }
        return String(format:
            "This is too long to fit in %d MB and still be watchable. "
            + "The file will be about %.0f MB. Shorten it, or export at a "
            + "lower resolution.", Int(mb), estimate)
    }
}

private extension Double {
    func clamped(to r: ClosedRange<Double>) -> Double { Swift.min(Swift.max(self, r.lowerBound), r.upperBound) }
}

