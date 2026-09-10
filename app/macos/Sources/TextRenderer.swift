import AppKit
import Metal
import CoreText
import ImageIO

/// Mirrors `mediacore_model::timeline::TextSpec`.
struct TextSpec: Codable, Hashable {
    var text: String
    var font: String
    var size: Float
    var weight: UInt16
    var tracking: Float
    var line_height: Float
    var align: UInt8
    var x: Float
    var y: Float
    var opacity: Float
    var colour: [Float]
    var outline_width: Float
    var outline_colour: [Float]
    var shadow_radius: Float
    var shadow_opacity: Float
    var background: [Float]
    var background_padding: Float
}

/// Rasterises text overlays and caches the result.
///
/// Text changes rarely and rasterising is comparatively expensive, so a spec is
/// rendered once and reused until it changes. Without the cache this would run
/// CoreText on every frame for a static title.
///
/// PRODUCT_DIRECTION §7: shadow and outline are here because text sits over
/// moving footage — white-on-anything is unreadable half the time, and that is
/// the single biggest reason amateur titles look amateur.
final class TextRenderer {
    private var cache: [Key: MTLTexture] = [:]
    private let device: MTLDevice

    private struct Key: Hashable { let spec: TextSpec; let width: Int; let height: Int }

    init(device: MTLDevice) { self.device = device }

    func texture(for spec: TextSpec, size: CGSize) -> MTLTexture? {
        let key = Key(spec: spec, width: Int(size.width), height: Int(size.height))
        if let t = cache[key] { return t }
        guard let image = rasterise(spec, size: size),
              let tex = upload(image, size: size) else { return nil }
        // Bounded: a handful of distinct overlays is normal; unbounded growth
        // while someone types is not.
        if cache.count > 32 { cache.removeAll() }
        cache[key] = tex
        return tex
    }

    private func colour(_ c: [Float], _ alpha: Float = 1) -> CGColor {
        let v = c.count >= 4 ? c : [1, 1, 1, 1]
        return CGColor(red: CGFloat(v[0]), green: CGFloat(v[1]),
                       blue: CGFloat(v[2]), alpha: CGFloat(v[3] * alpha))
    }

    private func rasterise(_ spec: TextSpec, size: CGSize) -> CGImage? {
        let w = Int(size.width), h = Int(size.height)
        guard w > 0, h > 0 else { return nil }
        let cs = CGColorSpaceCreateDeviceRGB()
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8,
                                  bytesPerRow: 0, space: cs,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.clear(CGRect(x: 0, y: 0, width: w, height: h))

        // Sizes are fractions of frame HEIGHT, so a title keeps its proportions
        // at any resolution — 1080p and 4K look the same, which they must.
        let pointSize = CGFloat(spec.size) * size.height
        let weight = NSFont.Weight(rawValue: (CGFloat(spec.weight) - 400) / 400.0)
        let font = NSFont.systemFont(ofSize: pointSize, weight: weight)

        let para = NSMutableParagraphStyle()
        para.alignment = [.left, .center, .right][Int(spec.align).clamped(0, 2)]
        para.lineHeightMultiple = CGFloat(spec.line_height)

        var attrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor(cgColor: colour(spec.colour, spec.opacity))!,
            .paragraphStyle: para,
            // Tracking is expressed relative to size, so it scales with the frame.
            .kern: CGFloat(spec.tracking) * pointSize,
        ]
        if spec.outline_width > 0 {
            attrs[.strokeWidth] = -Double(spec.outline_width) * 100.0   // negative = stroke AND fill
            attrs[.strokeColor] = NSColor(cgColor: colour(spec.outline_colour, spec.opacity))!
        }

        let attributed = NSAttributedString(string: spec.text, attributes: attrs)
        let bounds = attributed.boundingRect(
            with: CGSize(width: size.width * 0.9, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading])

        // Position is normalised, and y is measured from the TOP so it matches
        // how the frame is displayed.
        let cx = CGFloat(spec.x) * size.width
        let cy = (1.0 - CGFloat(spec.y)) * size.height
        var rect = CGRect(x: cx - bounds.width / 2, y: cy - bounds.height / 2,
                          width: bounds.width, height: bounds.height)

        let gctx = NSGraphicsContext(cgContext: ctx, flipped: false)
        NSGraphicsContext.current = gctx
        defer { NSGraphicsContext.current = nil }

        if spec.background.count >= 4 && spec.background[3] > 0.001 {
            let pad = CGFloat(spec.background_padding) * size.height
            let plate = rect.insetBy(dx: -pad, dy: -pad * 0.6)
            ctx.setFillColor(colour(spec.background, spec.opacity))
            ctx.addPath(CGPath(roundedRect: plate, cornerWidth: pad * 0.5,
                               cornerHeight: pad * 0.5, transform: nil))
            ctx.fillPath()
        }

        if spec.shadow_opacity > 0.001 {
            let shadow = NSShadow()
            shadow.shadowColor = NSColor(white: 0, alpha: CGFloat(spec.shadow_opacity))
            shadow.shadowBlurRadius = CGFloat(spec.shadow_radius) * size.height
            shadow.shadowOffset = .zero
            shadow.set()
        }

        rect.size.width = max(rect.size.width, 1)
        attributed.draw(with: rect, options: [.usesLineFragmentOrigin, .usesFontLeading])
        return ctx.makeImage()
    }

    private func upload(_ image: CGImage, size: CGSize) -> MTLTexture? {
        let w = image.width, h = image.height
        let d = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba8Unorm, width: w, height: h, mipmapped: false)
        d.usage = [.shaderRead]
        guard let tex = device.makeTexture(descriptor: d) else { return nil }
        let bytesPerRow = w * 4
        var data = [UInt8](repeating: 0, count: bytesPerRow * h)
        guard let ctx = CGContext(data: &data, width: w, height: h, bitsPerComponent: 8,
                                  bytesPerRow: bytesPerRow,
                                  space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        tex.replace(region: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0,
                    withBytes: data, bytesPerRow: bytesPerRow)
        return tex
    }
}

extension Int {
    func clamped(_ lo: Int, _ hi: Int) -> Int { Swift.min(Swift.max(self, lo), hi) }
}


/// Mirrors `mediacore_model::timeline::ImageSpec` (§20).
struct ImageSpec: Codable, Hashable {
    var path: String
    var x: Float
    var y: Float
    var scale: Float
    var rotation: Float
    var opacity: Float
}

/// Loads and caches image overlays (§20).
///
/// Uses **ImageIO**, not FFmpeg. FFmpeg's still-image demuxers are awkward for
/// single files — a perfectly valid PNG reports "unspecified size" and fails to
/// open — while ImageIO is the platform's own decoder: it handles PNG alpha,
/// JPEG, WebP, TIFF and HEIC, and produces the colours users expect. FFmpeg
/// remains the video path. (The Windows port has the same option in WIC.)
///
/// An overlay logo is decoded ONCE, not per frame.
final class ImageCache {
    private var cache: [String: MTLTexture] = [:]
    private let device: MTLDevice
    init(device: MTLDevice) { self.device = device }

    func texture(for path: String) -> MTLTexture? {
        if let t = cache[path] { return t }
        guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
              let cg = CGImageSourceCreateImageAtIndex(src, 0, nil) else { return nil }

        let w = cg.width, h = cg.height
        guard w > 0, h > 0 else { return nil }

        // Redraw into a known RGBA layout: the source may use any bit depth,
        // colour space or alpha arrangement, and §20 requires alpha to survive.
        var data = [UInt8](repeating: 0, count: w * h * 4)
        guard let ctx = CGContext(data: &data, width: w, height: h,
                                  bitsPerComponent: 8, bytesPerRow: w * 4,
                                  space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.clear(CGRect(x: 0, y: 0, width: w, height: h))
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))

        let d = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba8Unorm, width: w, height: h, mipmapped: false)
        d.usage = [.shaderRead]
        guard let tex = device.makeTexture(descriptor: d) else { return nil }
        tex.replace(region: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0,
                    withBytes: data, bytesPerRow: w * 4)
        if cache.count > 24 { cache.removeAll() }
        cache[path] = tex
        return tex
    }
}
