import Foundation
import simd

/// A colour lookup table read from an Adobe `.cube` file.
///
/// The format is plain text and tiny to parse, which is why this is here rather
/// than pulled in as a dependency: a LUT is a size, an optional domain, and
/// then `size³` lines of "r g b". Everything else in the file is a comment or a
/// title.
///
/// Parsing is deliberately forgiving about whitespace, comments, key case and
/// blank lines — real .cube files come from a dozen different tools and vary in
/// all of those — and deliberately strict about the DATA, because a table with
/// the wrong number of entries would sample garbage rather than fail.
struct CubeLUT: Equatable {
    /// Edge length of the cube; 33 is the usual size.
    let size: Int
    /// `size³` RGB triples, red varying fastest — the order the file uses and
    /// the order a 3D texture wants.
    let rgb: [SIMD3<Float>]
    /// The input range the table covers. Almost always 0-1.
    let domainMin: SIMD3<Float>
    let domainMax: SIMD3<Float>

    enum Failure: Error, CustomStringConvertible {
        case unreadable
        case noSize
        case badSize(Int)
        case wrongCount(expected: Int, found: Int)

        var description: String {
            switch self {
            case .unreadable:
                return "That LUT file could not be read."
            case .noSize:
                return "That file does not look like a .cube LUT — it has no LUT_3D_SIZE."
            case .badSize(let n):
                return "That LUT says its size is \(n), which is not a size a LUT can have."
            case .wrongCount(let expected, let found):
                return "That LUT is incomplete: it should hold \(expected) entries but has \(found)."
            }
        }
    }

    static func load(path: String) throws -> CubeLUT {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else {
            // Some tools write Latin-1. Worth a second try before giving up.
            guard let fallback = try? String(contentsOfFile: path, encoding: .isoLatin1) else {
                throw Failure.unreadable
            }
            return try parse(fallback)
        }
        return try parse(text)
    }

    static func parse(_ text: String) throws -> CubeLUT {
        var size = 0
        var lo = SIMD3<Float>(0, 0, 0)
        var hi = SIMD3<Float>(1, 1, 1)
        var rgb: [SIMD3<Float>] = []

        for rawLine in text.split(separator: "\n", omittingEmptySubsequences: false) {
            // A comment can follow data on the same line.
            let line = rawLine.split(separator: "#", maxSplits: 1,
                                     omittingEmptySubsequences: false)[0]
                              .trimmingCharacters(in: .whitespacesAndNewlines)
            if line.isEmpty { continue }
            let parts = line.split(whereSeparator: { $0 == " " || $0 == "\t" })
            guard let head = parts.first else { continue }

            switch head.uppercased() {
            case "LUT_3D_SIZE":
                size = parts.count > 1 ? (Int(parts[1]) ?? 0) : 0
            case "LUT_1D_SIZE":
                // A 1D LUT is a different thing and would be silently wrong if
                // treated as a cube edge length.
                throw Failure.noSize
            case "DOMAIN_MIN" where parts.count >= 4:
                lo = SIMD3(Float(parts[1]) ?? 0, Float(parts[2]) ?? 0, Float(parts[3]) ?? 0)
            case "DOMAIN_MAX" where parts.count >= 4:
                hi = SIMD3(Float(parts[1]) ?? 1, Float(parts[2]) ?? 1, Float(parts[3]) ?? 1)
            case "TITLE":
                continue
            default:
                guard parts.count >= 3,
                      let r = Float(parts[0]), let g = Float(parts[1]), let b = Float(parts[2])
                else { continue }
                rgb.append(SIMD3(r, g, b))
            }
        }

        guard size > 0 else { throw Failure.noSize }
        // 2 is the smallest cube that means anything; past 129 the texture
        // stops being sensible and the file is almost certainly malformed.
        guard size >= 2, size <= 129 else { throw Failure.badSize(size) }
        let expected = size * size * size
        guard rgb.count == expected else {
            throw Failure.wrongCount(expected: expected, found: rgb.count)
        }
        return CubeLUT(size: size, rgb: rgb, domainMin: lo, domainMax: hi)
    }

    /// The table as tightly packed RGBA float32, ready for a 3D texture.
    /// Alpha is 1 throughout; a LUT does not touch it.
    func rgbaFloats() -> [Float] {
        var out = [Float](repeating: 1, count: rgb.count * 4)
        for (i, c) in rgb.enumerated() {
            out[i * 4] = c.x; out[i * 4 + 1] = c.y; out[i * 4 + 2] = c.z
        }
        return out
    }
}

// MARK: - GPU

import Metal

/// Loaded LUTs, as 3D textures.
///
/// Cached by path because a grade is applied to many clips and re-reading a
/// several-megabyte table per frame would be absurd. Failures are cached too:
/// a LUT that will not parse must not be re-read and re-reported sixty times a
/// second.
final class LUTCache {
    private let device: MTLDevice
    private var textures: [String: MTLTexture] = [:]
    private var failed: Set<String> = []
    private let lock = NSLock()

    /// A 2×2×2 identity table, bound whenever a clip has no LUT of its own.
    /// Metal refuses to draw with an unbound texture argument even when the
    /// shader's branch never samples it.
    let neutral: MTLTexture

    init(device: MTLDevice) {
        self.device = device
        let d = MTLTextureDescriptor()
        d.textureType = .type3D
        d.pixelFormat = .rgba32Float
        d.width = 2; d.height = 2; d.depth = 2
        d.usage = .shaderRead
        // A texture is guaranteed here in practice, and a compositor with no
        // device to make one has already failed for other reasons.
        neutral = device.makeTexture(descriptor: d)!
        var identity = [Float]()
        for b in 0..<2 { for g in 0..<2 { for r in 0..<2 {
            identity += [Float(r), Float(g), Float(b), 1]
        } } }
        identity.withUnsafeBytes { raw in
            neutral.replace(region: MTLRegionMake3D(0, 0, 0, 2, 2, 2),
                            mipmapLevel: 0, slice: 0,
                            withBytes: raw.baseAddress!,
                            bytesPerRow: 2 * 4 * MemoryLayout<Float>.size,
                            bytesPerImage: 2 * 2 * 4 * MemoryLayout<Float>.size)
        }
    }

    func texture(for path: String) -> MTLTexture? {
        lock.lock(); defer { lock.unlock() }
        if let t = textures[path] { return t }
        if failed.contains(path) { return nil }
        guard let lut = try? CubeLUT.load(path: path) else {
            failed.insert(path)
            return nil
        }
        let d = MTLTextureDescriptor()
        d.textureType = .type3D
        d.pixelFormat = .rgba32Float
        d.width = lut.size; d.height = lut.size; d.depth = lut.size
        d.usage = .shaderRead
        guard let tex = device.makeTexture(descriptor: d) else {
            failed.insert(path)
            return nil
        }
        let floats = lut.rgbaFloats()
        floats.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            tex.replace(region: MTLRegionMake3D(0, 0, 0, lut.size, lut.size, lut.size),
                        mipmapLevel: 0, slice: 0, withBytes: base,
                        bytesPerRow: lut.size * 4 * MemoryLayout<Float>.size,
                        bytesPerImage: lut.size * lut.size * 4 * MemoryLayout<Float>.size)
        }
        textures[path] = tex
        return tex
    }

    /// Forget a path, so a re-saved LUT is picked up rather than the old one.
    func forget(_ path: String) {
        lock.lock(); defer { lock.unlock() }
        textures.removeValue(forKey: path)
        failed.remove(path)
    }
}
