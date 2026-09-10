import Metal
import MetalKit
import CoreVideo
import AppKit

/// The Metal compositor.
///
/// AD-4: the core decides WHAT to draw (a render plan), this decides HOW.
///
/// Preview and export both call `render`. That is deliberate — R-18 is the risk
/// that exported output diverges from what the preview showed, and the surest
/// guard is that there is only one implementation to diverge from.
final class Compositor {
    private let device: MTLDevice
    let queue: MTLCommandQueue
    private var pipeline: MTLRenderPipelineState?
    private var textPipeline: MTLRenderPipelineState?
    private var texCache: CVMetalTextureCache?
    private let textRenderer: TextRenderer
    private let imageCache: ImageCache
    private let lutCache: LUTCache
    private var imagePipeline: MTLRenderPipelineState?

    /// Must match `Params` in the shader; keep the field order in step.
    struct Params {
        var scaleX: Float = 1
        var scaleY: Float = 1
        var hasChroma: Float = 0
        var opacity: Float = 1
        var brightness: Float = 0
        var contrast: Float = 0
        var saturation: Float = 0
        var temperature: Float = 0
        var tint: Float = 0
        var grayscale: Float = 0
        var transfer: Float = 0
        var blur: Float = 0
        var sharpen: Float = 0
        /// 1/width, 1/height of the SOURCE frame, so kernel taps land on real
        /// neighbouring pixels regardless of how the frame is scaled on screen.
        var texelX: Float = 0
        var texelY: Float = 0
        // Framing. Normalised source coordinates, straight from the plan.
        var cropX: Float = 0
        var cropY: Float = 0
        var cropW: Float = 1
        var cropH: Float = 1
        var rotation: Float = 0
        var flipH: Float = 0
        var flipV: Float = 0
        /// 0 = no colour lookup table on this clip.
        var lutAmount: Float = 0
    }

    init?(device: MTLDevice) {
        self.device = device
        guard let q = device.makeCommandQueue() else { return nil }
        self.queue = q
        self.textRenderer = TextRenderer(device: device)
        self.imageCache = ImageCache(device: device)
        self.lutCache = LUTCache(device: device)
        CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &texCache)
        buildPipelines()
        if pipeline == nil { return nil }
    }

    // MARK: - Shaders

    private func buildPipelines() {
        let src = """
        #include <metal_stdlib>
        using namespace metal;
        struct VOut { float4 pos [[position]]; float2 uv; };
        // SCALARS ONLY, in the same order as the Swift struct.
        //
        // Metal aligns `float2` to 8 bytes and `float4` to 16 inside a constant
        // buffer; Swift packs a struct of `Float`s at 4. A `float2` after an odd
        // number of floats therefore reads from four bytes further along than
        // Swift wrote — which is silent, because the numbers are all plausible.
        // It was already happening to `texel` before crop was added: blur and
        // sharpen were sampling with the wrong step. Keeping every field scalar
        // makes the two layouts identical by construction.
        struct Params {
            float scaleX; float scaleY; float hasChroma; float opacity;
            float brightness; float contrast; float saturation;
            float temperature; float tint; float grayscale;
            float transfer; float blur; float sharpen;
            float texelX; float texelY;                 // 1/width, 1/height
            float cropX; float cropY; float cropW; float cropH;  // normalised
            float rotation;                             // quarter turns clockwise
            float flipH; float flipV;
            float lutAmount;                            // 0 = no LUT
        };

        // A QUAD, not the usual full-screen triangle.
        //
        // The full-screen-triangle trick relies on an oversized triangle
        // covering the viewport with UVs derived from clip position. Scaling
        // its vertices to letterbox breaks both halves: it no longer covers,
        // and its hypotenuse becomes a visible diagonal tear across the frame.
        vertex VOut v_main(uint vid [[vertex_id]], constant Params &p [[buffer(0)]]) {
            float2 v[4]  = { float2(-1,-1), float2(1,-1), float2(-1,1), float2(1,1) };
            float2 uv[4] = { float2(0,1),   float2(1,1),  float2(0,0),  float2(1,0) };
            VOut o;
            o.pos = float4(v[vid].x * p.scaleX, v[vid].y * p.scaleY, 0, 1);

            // Framing happens HERE, on four vertices, rather than per pixel in
            // the fragment shader. Crop, rotation and flip are all affine in
            // texture coordinates, so the rasteriser interpolates them for
            // free — at 4K that is four transforms instead of eight million.
            float2 t = uv[vid];
            if (p.flipH > 0.5) t.x = 1.0 - t.x;
            if (p.flipV > 0.5) t.y = 1.0 - t.y;
            // Rotating the PICTURE clockwise means rotating the coordinates we
            // read by the same amount anticlockwise.
            int r = int(p.rotation + 0.5);
            if      (r == 1) t = float2(t.y, 1.0 - t.x);
            else if (r == 2) t = float2(1.0 - t.x, 1.0 - t.y);
            else if (r == 3) t = float2(1.0 - t.y, t.x);
            o.uv = float2(p.cropX, p.cropY) + t * float2(p.cropW, p.cropH);
            return o;
        }

        // --- HDR -> SDR (O-3) --------------------------------------------
        // V1 is an SDR Rec.709 pipeline, so PQ and HLG are tone-mapped
        // DELIBERATELY. Treating them as 709 is what produces washed-out
        // output. Curve: Hable filmic — provisional per O-3, but it rolls off
        // highlights rather than clipping them, which is the failure that
        // actually looks bad.
        float3 hable(float3 x) {
            const float A=0.15, B=0.50, C=0.10, D=0.20, E=0.02, F=0.30;
            return ((x*(A*x+C*B)+D*E)/(x*(A*x+B)+D*F))-E/F;
        }
        float3 tonemap(float3 c) {
            const float W = 11.2;
            return saturate(hable(c * 2.0) / hable(float3(W)));
        }
        float3 pq_to_linear(float3 e) {
            const float m1=0.1593017578125, m2=78.84375, c1=0.8359375, c2=18.8515625, c3=18.6875;
            float3 p = pow(max(e, 0.0), 1.0/m2);
            return pow(max(p - c1, 0.0) / (c2 - c3 * p), 1.0/m1);
        }
        float3 hlg_to_linear(float3 e) {
            const float a=0.17883277, b=0.28466892, c=0.55991073;
            return select(exp((e - c)/a + b) / 12.0, (e*e)/3.0, e <= 0.5);
        }

        fragment float4 f_main(VOut in [[stage_in]],
                               constant Params &p [[buffer(0)]],
                               texture2d<float> lumaTex   [[texture(0)]],
                               texture2d<float> chromaTex [[texture(1)]],
                               texture3d<float> lut       [[texture(2)]]) {
            constexpr sampler s(filter::linear, address::clamp_to_edge);

            // §18 blur and sharpen need neighbouring samples, unlike every
            // other effect here. A 3x3 kernel is enough for the "basic blur /
            // basic sharpen" the spec asks for and stays cheap at 4K; a proper
            // gaussian would need a separable two-pass and is not what §18
            // describes.
            float2 uv = in.uv;
            float3 rgb;
            if (p.hasChroma < 0.5) {
                float y = lumaTex.sample(s, uv).r;
                if (p.blur > 0.001) {
                    float acc = 0.0;
                    for (int dy = -1; dy <= 1; dy++)
                        for (int dx = -1; dx <= 1; dx++)
                            acc += lumaTex.sample(s, uv + float2(dx, dy) * float2(p.texelX, p.texelY) * p.blur * 4.0).r;
                    y = mix(y, acc / 9.0, saturate(p.blur));
                }
                rgb = float3(y);
            } else {
                float  y    = lumaTex.sample(s, uv).r;
                if (p.blur > 0.001) {
                    float acc = 0.0;
                    for (int dy = -1; dy <= 1; dy++)
                        for (int dx = -1; dx <= 1; dx++)
                            acc += lumaTex.sample(s, uv + float2(dx, dy) * float2(p.texelX, p.texelY) * p.blur * 4.0).r;
                    y = mix(y, acc / 9.0, saturate(p.blur));
                }
                if (p.sharpen > 0.001) {
                    // Unsharp mask: original + (original - blurred) * amount.
                    float acc = 0.0;
                    for (int dy = -1; dy <= 1; dy++)
                        for (int dx = -1; dx <= 1; dx++)
                            acc += lumaTex.sample(s, uv + float2(dx, dy) * float2(p.texelX, p.texelY)).r;
                    y = saturate(y + (y - acc / 9.0) * p.sharpen * 3.0);
                }
                float2 cbcr = chromaTex.sample(s, uv).rg;
                // Rec.709 limited range: Y in [16,235], C in [16,240].
                float yy = (y - 16.0/255.0) * (255.0/219.0);
                float cb = cbcr.r - 128.0/255.0;
                float cr = cbcr.g - 128.0/255.0;
                rgb = float3(yy + 1.5748*cr,
                             yy - 0.1873*cb - 0.4681*cr,
                             yy + 1.8556*cb);
            }

            if (p.transfer > 0.5 && p.transfer < 1.5)      rgb = tonemap(pq_to_linear(saturate(rgb)));
            else if (p.transfer > 1.5)                     rgb = tonemap(hlg_to_linear(saturate(rgb)));
            rgb = saturate(rgb);

            // §18 effects, in an order that behaves predictably: exposure,
            // then contrast about mid-grey, then temperature, then saturation.
            // Saturation before contrast makes the two fight each other.
            rgb += p.brightness;
            rgb = (rgb - 0.5) * (1.0 + p.contrast) + 0.5;
            rgb.r += p.temperature * 0.12;
            rgb.b -= p.temperature * 0.12;
            rgb.g += p.tint * 0.12;

            float luma709 = dot(rgb, float3(0.2126, 0.7152, 0.0722));
            rgb = (p.grayscale > 0.5) ? float3(luma709)
                                      : mix(float3(luma709), rgb, 1.0 + p.saturation);
            rgb = saturate(rgb);

            // The lookup table goes LAST, on the finished picture.
            //
            // A LUT is a look, and a look is applied to a graded image — the
            // sliders are the grade. Sampling before them would have the
            // sliders fighting the look instead of feeding it.
            //
            // Trilinear filtering is the hardware's own, which is why the table
            // is a 3D texture rather than an array: a 33-cube interpolated by
            // hand is far more code and slower. The half-texel inset is what
            // keeps the cube's outer faces from being smeared by clamping.
            if (p.lutAmount > 0.001) {
                float n = float(lut.get_width());
                float3 c = (rgb * (n - 1.0) + 0.5) / n;
                constexpr sampler ls(filter::linear, address::clamp_to_edge);
                rgb = mix(rgb, lut.sample(ls, c).rgb, saturate(p.lutAmount));
            }
            return float4(saturate(rgb), p.opacity);
        }
        """

        // Image overlays (§20): RGBA with alpha preserved, placed by a
        // centre/scale/rotation transform. Aspect is taken from the IMAGE so a
        // logo is never stretched.
        let imageSrc = """
        #include <metal_stdlib>
        using namespace metal;
        struct IOut { float4 pos [[position]]; float2 uv; };
        struct IParams { float cx; float cy; float hw; float hh; float rotation; float opacity; };
        vertex IOut i_main(uint vid [[vertex_id]], constant IParams &p [[buffer(0)]]) {
            float2 v[4]  = { float2(-1,-1), float2(1,-1), float2(-1,1), float2(1,1) };
            float2 uv[4] = { float2(0,1),   float2(1,1),  float2(0,0),  float2(1,0) };
            float c = cos(p.rotation), s = sin(p.rotation);
            float2 local = v[vid] * float2(p.hw, p.hh);
            float2 rot = float2(local.x * c - local.y * s, local.x * s + local.y * c);
            IOut o;
            o.pos = float4(rot + float2(p.cx, p.cy), 0, 1);
            o.uv = uv[vid];
            return o;
        }
        fragment float4 i_frag(IOut in [[stage_in]],
                               constant IParams &p [[buffer(0)]],
                               texture2d<float> tex [[texture(0)]]) {
            constexpr sampler s(filter::linear, address::clamp_to_edge);
            float4 c = tex.sample(s, in.uv);
            // Premultiplied alpha from ImageIO: scale colour with alpha.
            return float4(c.rgb * p.opacity, c.a * p.opacity);
        }
        """

        // Text is already RGBA from CoreText — no YCbCr, no tone mapping.
        let textSrc = """
        #include <metal_stdlib>
        using namespace metal;
        struct TOut { float4 pos [[position]]; float2 uv; };
        // Text is transformed by the SAME letterbox scale as the video, or a
        // title at y=0.85 lands in the black bar: visible in the preview,
        // absent from the export.
        vertex TOut t_main(uint vid [[vertex_id]], constant float2 &scale [[buffer(1)]]) {
            float2 v[4]  = { float2(-1,-1), float2(1,-1), float2(-1,1), float2(1,1) };
            float2 uv[4] = { float2(0,1),   float2(1,1),  float2(0,0),  float2(1,0) };
            TOut o;
            o.pos = float4(v[vid].x * scale.x, v[vid].y * scale.y, 0, 1);
            o.uv = uv[vid];
            return o;
        }
        fragment float4 t_frag(TOut in [[stage_in]],
                               constant float &opacity [[buffer(0)]],
                               texture2d<float> tex [[texture(0)]]) {
            constexpr sampler s(filter::linear, address::clamp_to_edge);
            float4 c = tex.sample(s, in.uv);
            return float4(c.rgb, c.a * opacity);
        }
        """

        func blend(_ d: MTLRenderPipelineDescriptor, premultiplied: Bool) {
            d.colorAttachments[0].pixelFormat = .bgra8Unorm
            d.colorAttachments[0].isBlendingEnabled = true
            d.colorAttachments[0].rgbBlendOperation = .add
            d.colorAttachments[0].alphaBlendOperation = .add
            d.colorAttachments[0].sourceRGBBlendFactor = premultiplied ? .one : .sourceAlpha
            d.colorAttachments[0].sourceAlphaBlendFactor = .one
            d.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
            d.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
        }

        if let lib = try? device.makeLibrary(source: src, options: nil) {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = lib.makeFunction(name: "v_main")
            d.fragmentFunction = lib.makeFunction(name: "f_main")
            blend(d, premultiplied: false)
            pipeline = try? device.makeRenderPipelineState(descriptor: d)
        }
        if let ilib = try? device.makeLibrary(source: imageSrc, options: nil) {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = ilib.makeFunction(name: "i_main")
            d.fragmentFunction = ilib.makeFunction(name: "i_frag")
            blend(d, premultiplied: true)   // ImageIO gives premultiplied alpha
            imagePipeline = try? device.makeRenderPipelineState(descriptor: d)
        }
        if let tlib = try? device.makeLibrary(source: textSrc, options: nil) {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = tlib.makeFunction(name: "t_main")
            d.fragmentFunction = tlib.makeFunction(name: "t_frag")
            blend(d, premultiplied: true)   // CoreText gives premultiplied alpha
            textPipeline = try? device.makeRenderPipelineState(descriptor: d)
        }
    }

    // MARK: - Render

    /// Composite a plan into `target`. Used by BOTH preview and export.
    struct ImageParams { var cx: Float = 0; var cy: Float = 0
                         var hw: Float = 0; var hh: Float = 0
                         var rotation: Float = 0; var opacity: Float = 1 }

    func render(plan: [MCPlanLayer],
                into target: MTLTexture,
                assetPath: (UInt64) -> String?,
                textSpec: (UInt64) -> TextSpec?,
                imageSpec: (UInt64) -> ImageSpec?,
                lutSpec: (UInt64) -> Core.LutSpec? = { _ in nil },
                captionSpec: (Int64) -> TextSpec? = { _ in nil },
                frames: FrameSource,
                drawable: CAMetalDrawable? = nil)
    {
        guard let pipeline, let cache = texCache,
              let cb = queue.makeCommandBuffer() else { return }

        let rpd = MTLRenderPassDescriptor()
        rpd.colorAttachments[0].texture = target
        rpd.colorAttachments[0].loadAction = .clear
        rpd.colorAttachments[0].storeAction = .store
        rpd.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 1)

        let size = CGSize(width: target.width, height: target.height)
        // The GPU reads these AFTER this returns; releasing a CVMetalTexture
        // early lets its IOSurface be recycled mid-draw, which shows as tearing.
        var keepAlive: [Any] = []

        struct Draw { var luma: MTLTexture; var chroma: MTLTexture?
                      var lut: MTLTexture?; var params: Params }
        var draws: [Draw] = []
        let paneAspect = Double(size.width) / Double(max(size.height, 1))

        for layer in plan where layer.kind == 0 && layer.is_text == 0 && layer.is_image == 0 {
            guard let path = assetPath(layer.asset_id),
                  let pb = frames.frame(asset: layer.asset_id, path: path,
                                        sourceTicks: layer.source_time_ticks) else { continue }
            keepAlive.append(pb)

            let w = CVPixelBufferGetWidthOfPlane(pb, 0)
            let h = CVPixelBufferGetHeightOfPlane(pb, 0)
            // 10-bit HDR decodes to 16-bit planes; r8Unorm would read the
            // wrong bytes entirely.
            let f = CVPixelBufferGetPixelFormatType(pb)
            let deep = f != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                    && f != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            var lumaRef: CVMetalTexture?
            guard CVMetalTextureCacheCreateTextureFromImage(
                    kCFAllocatorDefault, cache, pb, nil, deep ? .r16Unorm : .r8Unorm,
                    w, h, 0, &lumaRef) == kCVReturnSuccess,
                  let lumaRef, let lumaTex = CVMetalTextureGetTexture(lumaRef) else { continue }
            keepAlive.append(lumaRef)

            var chromaTex: MTLTexture?
            if CVPixelBufferGetPlaneCount(pb) > 1 {
                var cRef: CVMetalTexture?
                if CVMetalTextureCacheCreateTextureFromImage(
                    kCFAllocatorDefault, cache, pb, nil, deep ? .rg16Unorm : .rg8Unorm,
                    CVPixelBufferGetWidthOfPlane(pb, 1),
                    CVPixelBufferGetHeightOfPlane(pb, 1), 1, &cRef) == kCVReturnSuccess,
                   let cRef {
                    chromaTex = CVMetalTextureGetTexture(cRef)
                    keepAlive.append(cRef)
                }
            }

            // The letterbox must follow what is actually DRAWN, not what was
            // shot: a cropped-and-rotated 16:9 clip is a different shape, and
            // using the source's aspect would stretch it.
            let sourceAspect = h > 0 ? Double(w) / Double(h) : 16.0 / 9.0
            let cropW = max(Double(layer.crop_w), 0.0001)
            let cropH = max(Double(layer.crop_h), 0.0001)
            var frameAspect = sourceAspect * cropW / cropH
            if layer.rotation % 2 == 1 { frameAspect = 1.0 / frameAspect }
            var p = Params()
            if paneAspect > frameAspect { p.scaleX = Float(frameAspect / paneAspect); p.scaleY = 1 }
            else { p.scaleX = 1; p.scaleY = Float(paneAspect / frameAspect) }
            p.hasChroma   = chromaTex != nil ? 1 : 0
            p.opacity     = Float(layer.opacity)
            p.brightness  = layer.brightness
            p.contrast    = layer.contrast
            p.saturation  = layer.saturation
            p.temperature = layer.temperature
            p.tint        = layer.tint
            p.grayscale   = layer.grayscale != 0 ? 1 : 0
            p.transfer    = Float(layer.transfer)
            p.blur        = layer.blur
            p.sharpen     = layer.sharpen
            p.texelX      = 1.0 / Float(max(w, 1))
            p.texelY      = 1.0 / Float(max(h, 1))
            p.cropX       = layer.crop_x
            p.cropY       = layer.crop_y
            p.cropW       = layer.crop_w
            p.cropH       = layer.crop_h
            p.rotation    = Float(layer.rotation)
            p.flipH       = layer.flip_h != 0 ? 1 : 0
            p.flipV       = layer.flip_v != 0 ? 1 : 0
            var lutTex: MTLTexture?
            if layer.lut_amount > 0.001, let spec = lutSpec(layer.clip_id),
               let t = lutCache.texture(for: spec.path) {
                lutTex = t
                p.lutAmount = layer.lut_amount * spec.amount
            }
            draws.append(Draw(luma: lumaTex, chroma: chromaTex, lut: lutTex, params: p))
        }

        // Titles use the frame's geometry, so they land where they will land in
        // the export rather than where the pane happens to be shaped.
        let frameScale = draws.first.map { SIMD2<Float>($0.params.scaleX, $0.params.scaleY) }
                      ?? SIMD2<Float>(1, 1)
        let frameScaleX = frameScale.x, frameScaleY = frameScale.y
        let textSize = CGSize(width: max(size.width * CGFloat(frameScale.x), 16),
                              height: max(size.height * CGFloat(frameScale.y), 16))
        // Image overlays sit above the video and below titles.
        var imageDraws: [(MTLTexture, ImageParams)] = []
        for layer in plan where layer.kind == 0 && layer.is_image != 0 {
            guard let spec = imageSpec(layer.clip_id),
                  let tex = imageCache.texture(for: spec.path) else { continue }
            let imgAspect = Double(tex.width) / Double(max(tex.height, 1))
            // Height is the controlled dimension: a fraction of the FRAME's
            // height, so a logo is the same relative size at any resolution.
            //
            // Width must then be derived from that already-scaled height, or
            // the letterbox scale distorts the image — scaling height by
            // frameScaleY and width by frameScaleX independently turns a circle
            // into an ellipse, which is exactly what happened first.
            let hh = Double(spec.scale) * Double(frameScaleY)
            let hw = hh * imgAspect / max(paneAspect, 0.0001)
            var p = ImageParams()
            p.cx = Float(Double(spec.x) * 2 - 1) * frameScaleX
            p.cy = Float(1 - Double(spec.y) * 2) * frameScaleY
            p.hw = Float(hw)
            p.hh = Float(hh)
            p.rotation = spec.rotation * .pi / 180
            p.opacity = Float(layer.opacity) * spec.opacity
            imageDraws.append((tex, p))
        }

        var textDraws: [(MTLTexture, Float)] = []
        for layer in plan where layer.kind == 0 && layer.is_text != 0 {
            // A caption layer carries no clip of its own — the words come from
            // the caption list at this instant. Everything after that is the
            // ordinary text path, which is the whole point of representing a
            // burned-in caption as a text layer.
            let spec = layer.is_caption != 0
                ? captionSpec(layer.source_time_ticks)
                : textSpec(layer.clip_id)
            guard let spec, let tex = textRenderer.texture(for: spec, size: textSize)
            else { continue }
            textDraws.append((tex, Float(layer.opacity) * spec.opacity))
        }

        if let enc = cb.makeRenderCommandEncoder(descriptor: rpd) {
            if !draws.isEmpty {
                enc.setRenderPipelineState(pipeline)
                for var d in draws {
                    enc.setVertexBytes(&d.params, length: MemoryLayout<Params>.stride, index: 0)
                    enc.setFragmentBytes(&d.params, length: MemoryLayout<Params>.stride, index: 0)
                    enc.setFragmentTexture(d.luma, index: 0)
                    if let c = d.chroma { enc.setFragmentTexture(c, index: 1) }
                    // Always bind SOMETHING at slot 2: Metal will not draw with
                    // an unbound texture argument even when the shader's branch
                    // never reads it.
                    enc.setFragmentTexture(d.lut ?? lutCache.neutral, index: 2)
                    enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
                }
            }
            if let imagePipeline, !imageDraws.isEmpty {
                enc.setRenderPipelineState(imagePipeline)
                for (tex, params) in imageDraws {
                    var p = params
                    enc.setVertexBytes(&p, length: MemoryLayout<ImageParams>.stride, index: 0)
                    enc.setFragmentBytes(&p, length: MemoryLayout<ImageParams>.stride, index: 0)
                    enc.setFragmentTexture(tex, index: 0)
                    enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
                }
            }
            if let textPipeline, !textDraws.isEmpty {
                enc.setRenderPipelineState(textPipeline)
                var scale = frameScale
                enc.setVertexBytes(&scale, length: MemoryLayout<SIMD2<Float>>.stride, index: 1)
                for (tex, alpha) in textDraws {
                    var a = alpha
                    enc.setFragmentBytes(&a, length: MemoryLayout<Float>.stride, index: 0)
                    enc.setFragmentTexture(tex, index: 0)
                    enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
                }
            }
            enc.endEncoding()
        }

        cb.addCompletedHandler { _ in _ = keepAlive }
        if let drawable { cb.present(drawable) }
        cb.commit()
        // Export must not race ahead of the GPU: the encoder reads the surface
        // as soon as we return. The preview does not wait.
        if drawable == nil { cb.waitUntilCompleted() }
    }
}
