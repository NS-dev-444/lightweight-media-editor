import Foundation

/// Swift wrapper over the `mcs_*` C ABI.
///
/// Two rules from the FFI design (see `session.rs`):
///   * bulk reads — `clips()` and `plan(at:)` are ONE call each, into a reused
///     buffer, because the timeline reads them every frame;
///   * the Rust core owns the document. This type never keeps a second mutable
///     copy of the timeline, which is the state-divergence bug AD-1 warns about.
final class Core {
    private let handle: OpaquePointer

    init?() {
        guard let h = mcs_new() else { return nil }
        handle = h
    }
    deinit { mcs_free(handle) }

    // MCDecoder/MCSession are incomplete C structs, so Swift imports pointers
    // to them as OpaquePointer. Passing the handle straight through is both
    // correct and simpler than trying to name the type.
    private var ptr: OpaquePointer { handle }

    /// Human-readable text for the last failure (§23).
    var lastError: String {
        guard let c = mcs_last_error(ptr) else { return "" }
        return String(cString: c)
    }

    // ---- document ----------------------------------------------------------
    var isDirty: Bool { mcs_is_dirty(ptr) != 0 }
    var duration: Int64 { mcs_duration(ptr) }
    var trackCount: Int { Int(mcs_track_count(ptr)) }

    /// The project's rational frame rate — export must follow it, not assume 30.
    var frameRate: (num: Int32, den: Int32) {
        var n: Int32 = 24, d: Int32 = 1
        mcs_frame_rate(ptr, &n, &d)
        return (n > 0 ? n : 24, d > 0 ? d : 1)
    }
    /// Rounded rate for encoders, which take an integer fps pair.
    var exportFPS: (num: Int32, den: Int32) {
        let r = frameRate
        return (r.num, r.den)
    }
    func trackId(at index: Int) -> UInt64 { mcs_track_id_at(ptr, Int32(index)) }

    @discardableResult func open(_ path: String) -> Bool { mcs_open(ptr, path) == 0 }
    @discardableResult func enableAutosave(_ path: String) -> Bool { mcs_enable_autosave(ptr, path) == 0 }
    static func hasUnrecoveredWork(_ path: String) -> Bool { mcs_has_unrecovered_work(path) != 0 }
    /// Returns the number of edits recovered, or nil on failure.
    func recover(_ path: String) -> Int? {
        let n = mcs_recover(ptr, path)
        return n >= 0 ? Int(n) : nil
    }
    static func discardRecovery(_ path: String) { _ = mcs_discard_recovery(path) }
    @discardableResult func save(_ path: String) -> Bool { mcs_save(ptr, path) == 0 }

    // ---- bulk reads --------------------------------------------------------
    private var clipBuffer = [MCClipView](repeating: MCClipView(), count: 1024)

    /// All clips, in one FFI call. The buffer is reused between frames.
    func clips() -> ArraySlice<MCClipView> {
        let needed = Int(mcs_clip_count(ptr))
        if needed > clipBuffer.count {
            clipBuffer = [MCClipView](repeating: MCClipView(), count: max(needed, clipBuffer.count * 2))
        }
        let n = clipBuffer.withUnsafeMutableBufferPointer { buf in
            Int(mcs_clips(ptr, buf.baseAddress, Int32(buf.count)))
        }
        return clipBuffer[0..<n]
    }

    private var planBuffer = [MCPlanLayer](repeating: MCPlanLayer(), count: 32)

    /// What to draw at a time (AD-4: the core says what, the platform says how).
    func plan(at ticks: Int64) -> ArraySlice<MCPlanLayer> {
        let n = planBuffer.withUnsafeMutableBufferPointer { buf in
            Int(mcs_plan_at(ptr, ticks, buf.baseAddress, Int32(buf.count)))
        }
        return planBuffer[0..<n]
    }

    // ---- edits -------------------------------------------------------------
    @discardableResult
    func addClip(track: UInt64, asset: UInt64, start: Int64,
                 sourceStart: Int64, sourceDuration: Int64) -> UInt64 {
        mcs_add_clip(ptr, track, asset, start, sourceStart, sourceDuration)
    }
    @discardableResult func splitAt(track: UInt64, ticks: Int64) -> Bool {
        mcs_split_at(ptr, track, ticks) == 0
    }
    @discardableResult func rippleDelete(track: UInt64, clip: UInt64) -> Bool {
        mcs_ripple_delete(ptr, track, clip) == 0
    }
    @discardableResult func deleteSelected() -> Bool { mcs_delete_selected(ptr) == 0 }
    @discardableResult func moveClip(_ clip: UInt64, toTrack: UInt64, toStart: Int64) -> Bool {
        mcs_move_clip(ptr, clip, toTrack, toStart) == 0
    }

    @discardableResult func undo() -> Bool { mcs_undo(ptr) == 0 }
    @discardableResult func redo() -> Bool { mcs_redo(ptr) == 0 }
    var canUndo: Bool { mcs_can_undo(ptr) != 0 }
    var canRedo: Bool { mcs_can_redo(ptr) != 0 }

    // ---- playhead & selection ---------------------------------------------
    var playhead: Int64 {
        get { mcs_playhead(ptr) }
        set { mcs_set_playhead(ptr, newValue) }
    }
    func stepFrames(_ n: Int64) { mcs_step_frames(ptr, n) }
    var timecode: String {
        guard let c = mcs_timecode(ptr) else { return "00:00:00:00" }
        return String(cString: c)
    }
    func selectOnly(_ clip: UInt64) { mcs_select_only(ptr, clip) }
    func selectToggle(_ clip: UInt64) { mcs_select_toggle(ptr, clip) }
    func clearSelection() { mcs_select_clear(ptr) }
    var selectionCount: Int { Int(mcs_selection_count(ptr)) }

    // ---- import ------------------------------------------------------------
    /// Probe and add. Returns 0 on failure; `lastError` explains why in words.
    func importFile(_ path: String) -> UInt64 { mcs_import(ptr, path) }
    var assetCount: Int { Int(mcs_asset_count(ptr)) }
    func assetDuration(_ id: UInt64) -> Int64 { mcs_asset_duration(ptr, id) }
    func assetHasVideo(_ id: UInt64) -> Bool { mcs_asset_has_video(ptr, id) != 0 }

    // ---- §19 text ----------------------------------------------------------
    @discardableResult
    func addText(track: UInt64, start: Int64, duration: Int64,
                 preset: Int32, text: String) -> UInt64 {
        mcs_add_text(ptr, track, start, duration, preset, text)
    }

    func clipText(_ clip: UInt64) -> TextSpec? {
        guard let c = mcs_clip_text(ptr, clip) else { return nil }
        let json = String(cString: c)
        guard !json.isEmpty, let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(TextSpec.self, from: data)
    }

    @discardableResult
    func setClipText(_ clip: UInt64, _ spec: TextSpec) -> Bool {
        guard let data = try? JSONEncoder().encode(spec),
              let json = String(data: data, encoding: .utf8) else { return false }
        return mcs_set_clip_text(ptr, clip, json) == 0
    }

    // ---- §20 image overlays ------------------------------------------------
    @discardableResult
    func addImage(track: UInt64, start: Int64, duration: Int64, path: String) -> UInt64 {
        mcs_add_image(ptr, track, start, duration, path)
    }
    func clipImage(_ clip: UInt64) -> ImageSpec? {
        guard let c = mcs_clip_image(ptr, clip) else { return nil }
        let json = String(cString: c)
        guard !json.isEmpty, let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(ImageSpec.self, from: d)
    }
    @discardableResult
    func setClipImage(_ clip: UInt64, _ spec: ImageSpec) -> Bool {
        guard let d = try? JSONEncoder().encode(spec),
              let json = String(data: d, encoding: .utf8) else { return false }
        return mcs_set_clip_image(ptr, clip, json) == 0
    }

    var textPresetNames: [String] {
        (0..<Int(mcs_text_preset_count())).compactMap {
            mcs_text_preset_name(ptr, Int32($0)).map { String(cString: $0) }
        }
    }

    // ---- §18 effects -------------------------------------------------------
    struct EffectValues {
        var brightness: Float = 0, contrast: Float = 0, saturation: Float = 0
        var temperature: Float = 0, tint: Float = 0
        var grayscale: Bool = false, blur: Float = 0, sharpen: Float = 0
    }

    func effects(_ clip: UInt64) -> EffectValues {
        var buf = [Float](repeating: 0, count: 8)
        let n = buf.withUnsafeMutableBufferPointer { b in
            Int(mcs_get_effects(ptr, clip, b.baseAddress, 8))
        }
        guard n == 8 else { return EffectValues() }
        return EffectValues(brightness: buf[0], contrast: buf[1], saturation: buf[2],
                            temperature: buf[3], tint: buf[4],
                            grayscale: buf[5] > 0.5, blur: buf[6], sharpen: buf[7])
    }

    @discardableResult
    func setEffects(_ clip: UInt64, _ e: EffectValues) -> Bool {
        mcs_set_effects(ptr, clip, e.brightness, e.contrast, e.saturation,
                        e.temperature, e.tint, e.grayscale ? 1 : 0, e.blur, e.sharpen) == 0
    }

    /// Duck `music` under `voice`. Returns the number of ducked passages,
    /// or -1 with `lastError` set.
    func duck(music: UInt64, under voice: UInt64,
              db: Double = -12, rampMs: Int32 = 250) -> Int32 {
        mcs_duck(ptr, music, voice, db, rampMs)
    }

    @discardableResult
    func clearGainPoints(_ clip: UInt64) -> Bool { mcs_clear_gain_points(ptr, clip) == 0 }

    func gainPointCount(_ clip: UInt64) -> Int32 { mcs_gain_point_count(ptr, clip) }

    /// Ripple-delete a span: the gap closes rather than becoming a hole.
    @discardableResult
    func rippleDeleteRange(track: UInt64, start: Int64, end: Int64) -> Bool {
        mcs_ripple_delete_range(ptr, track, start, end) == 0
    }

    /// A text preset's look applied to given words, without creating a clip.
    func textPresetSpec(_ preset: Int32, text: String) -> TextSpec? {
        guard let c = mcs_text_preset_spec(ptr, preset, text) else { return nil }
        let json = String(cString: c)
        guard !json.isEmpty, let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(TextSpec.self, from: d)
    }

    // ---- captions (§8) -------------------------------------------------------

    struct CaptionEntry: Codable, Equatable {
        var start: Int64
        var end: Int64
        var text: String
    }

    var captionCount: Int { Int(mcs_caption_count(ptr)) }

    func caption(at index: Int) -> CaptionEntry? {
        guard let c = mcs_caption_at_index(ptr, Int32(index)) else { return nil }
        let json = String(cString: c)
        guard !json.isEmpty, let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(CaptionEntry.self, from: d)
    }

    func allCaptions() -> [CaptionEntry] {
        (0..<captionCount).compactMap { caption(at: $0) }
    }

    /// The caption showing at an instant, or empty.
    func captionText(at ticks: Int64) -> String {
        guard let c = mcs_caption_at(ptr, ticks) else { return "" }
        return String(cString: c)
    }

    @discardableResult
    func setCaptions(_ items: [CaptionEntry]) -> Bool {
        guard let d = try? JSONEncoder().encode(items),
              let json = String(data: d, encoding: .utf8) else { return false }
        return mcs_set_captions(ptr, json) == 0
    }

    /// Reads SRT or WebVTT. Returns the number of captions, or -1.
    func importSubtitles(_ path: String) -> Int32 { mcs_import_subtitles(ptr, path) }

    @discardableResult
    func exportSubtitles(_ path: String) -> Bool { mcs_export_subtitles(ptr, path) == 0 }

    @discardableResult
    func setCaptionText(_ index: Int, _ text: String) -> Bool {
        mcs_set_caption_text(ptr, Int32(index), text) == 0
    }

    @discardableResult
    func removeCaption(_ index: Int) -> Bool { mcs_remove_caption(ptr, Int32(index)) == 0 }

    @discardableResult
    func shiftCaptions(_ ticks: Int64) -> Bool { mcs_shift_captions(ptr, ticks) == 0 }

    @discardableResult
    func setCaptionBurnIn(_ on: Bool, preset: Int32) -> Bool {
        mcs_set_caption_burn_in(ptr, on ? 1 : 0, preset) == 0
    }

    var captionBurnIn: Bool { mcs_caption_burn_in(ptr) != 0 }
    var captionPreset: Int32 { mcs_caption_preset(ptr) }

    // ---- colour --------------------------------------------------------------

    /// A colour lookup table on a clip. Mirrors `mediacore_model::timeline::LutSpec`.
    struct LutSpec: Codable, Equatable {
        var path: String
        var amount: Float
    }

    func lut(_ clip: UInt64) -> LutSpec? {
        guard let c = mcs_clip_lut(ptr, clip) else { return nil }
        let json = String(cString: c)
        guard !json.isEmpty, let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(LutSpec.self, from: data)
    }

    @discardableResult
    func setLut(_ clip: UInt64, path: String, amount: Float) -> Bool {
        mcs_set_clip_lut(ptr, clip, path, amount) == 0
    }

    // ---- sound -------------------------------------------------------------

    struct Levels: Equatable {
        var gain: Double = 1
        var fadeIn: Int64 = 0
        var fadeOut: Int64 = 0
    }

    func levels(_ clip: UInt64) -> Levels {
        var buf = [Double](repeating: 0, count: 3)
        let n = buf.withUnsafeMutableBufferPointer { b in
            Int(mcs_get_levels(ptr, clip, b.baseAddress, 3))
        }
        guard n == 3 else { return Levels() }
        return Levels(gain: buf[0], fadeIn: Int64(buf[1]), fadeOut: Int64(buf[2]))
    }

    @discardableResult
    func setGain(_ clip: UInt64, _ gain: Double) -> Bool { mcs_set_gain(ptr, clip, gain) == 0 }

    @discardableResult
    func setFades(_ clip: UInt64, _ inTicks: Int64, _ outTicks: Int64) -> Bool {
        mcs_set_fades(ptr, clip, inTicks, outTicks) == 0
    }

    /// Move a clip's sound onto its own audio track. 0 on failure.
    func detachAudio(_ clip: UInt64) -> UInt64 { mcs_detach_audio(ptr, clip) }

    @discardableResult
    func fitToLength(_ clip: UInt64, target: Int64, fade: Int64) -> Bool {
        mcs_fit_to_length(ptr, clip, target, fade) == 0
    }

    /// Integrated loudness in LUFS. -200 means nothing measurable.
    static func loudness(ofFile path: String) -> Double { mc_loudness(path) }
    static func samplePeak(ofFile path: String) -> Double { mc_sample_peak(path) }

    /// Stretches of near-silence, in TICKS, as (start, end) pairs.
    static func silenceRanges(inFile path: String, thresholdDb: Double,
                              minMs: Int32, padMs: Int32) -> [(Int64, Int64)] {
        var buf = [Int64](repeating: 0, count: 4096)
        let n = buf.withUnsafeMutableBufferPointer { b in
            Int(mc_silence_ranges(path, thresholdDb, minMs, padMs,
                                  b.baseAddress, Int32(b.count)))
        }
        guard n > 0 else { return [] }
        // Nanoseconds from the core; ticks everywhere in the model.
        let toTicks = { (ns: Int64) in
            Int64(Double(ns) / 1e9 * Double(TICKS_PER_SECOND))
        }
        return (0..<n).map { (toTicks(buf[$0 * 2]), toTicks(buf[$0 * 2 + 1])) }
    }

    /// How wide the clip is drawn relative to its height, after crop and
    /// rotation. 0 when it has no picture.
    func clipAspect(_ clip: UInt64) -> Double { Double(mcs_clip_aspect(ptr, clip)) }

    /// Crop, rotation and flip. Crop values are normalised source coordinates.
    struct GeometryValues: Equatable {
        var cropX: Float = 0, cropY: Float = 0
        var cropW: Float = 1, cropH: Float = 1
        /// Quarter turns clockwise, 0-3.
        var rotation: Int32 = 0
        var flipH: Bool = false, flipV: Bool = false
        var isNeutral: Bool { self == GeometryValues() }
    }

    func geometry(_ clip: UInt64) -> GeometryValues {
        var buf = [Float](repeating: 0, count: 7)
        let n = buf.withUnsafeMutableBufferPointer { b in
            Int(mcs_get_geometry(ptr, clip, b.baseAddress, 7))
        }
        guard n == 7 else { return GeometryValues() }
        return GeometryValues(cropX: buf[0], cropY: buf[1], cropW: buf[2], cropH: buf[3],
                              rotation: Int32(buf[4]),
                              flipH: buf[5] > 0.5, flipV: buf[6] > 0.5)
    }

    @discardableResult
    func setGeometry(_ clip: UInt64, _ g: GeometryValues) -> Bool {
        mcs_set_geometry(ptr, clip, g.cropX, g.cropY, g.cropW, g.cropH,
                         g.rotation, g.flipH ? 1 : 0, g.flipV ? 1 : 0) == 0
    }

    /// Crop to the largest centred rectangle of the given aspect.
    @discardableResult
    func reframe(_ clip: UInt64, aspect: Float) -> Bool {
        mcs_reframe(ptr, clip, aspect) == 0
    }

    /// Min/max peak pairs for an asset's audio. S7 measured 60 min of FLAC in
    /// 3.18 s, so this is an import-time job to cache — never a per-frame call.
    func waveform(_ id: UInt64, bucketsPerSecond: Int32 = 40) -> [Float] {
        let seconds = max(assetDuration(id).asSeconds, 0.1)
        let capacity = Int(Double(bucketsPerSecond) * seconds) + 64
        var buf = [Float](repeating: 0, count: capacity * 2)
        let pairs = buf.withUnsafeMutableBufferPointer { b in
            Int(mcs_waveform(ptr, id, bucketsPerSecond, b.baseAddress, Int32(capacity)))
        }
        return Array(buf[0..<(pairs * 2)])
    }

    /// HDR / VFR notices for an asset, empty when there is nothing to say.
    func assetNotices(_ id: UInt64) -> [String] {
        guard let c = mcs_asset_notices(ptr, id) else { return [] }
        let s = String(cString: c)
        return s.isEmpty ? [] : s.components(separatedBy: "\n")
    }

    func assetPath(_ id: UInt64) -> String? {
        guard let c = mcs_asset_path(ptr, id) else { return nil }
        let s = String(cString: c)
        return s.isEmpty ? nil : s
    }
}

/// Ticks per second, mirroring `mediacore_model::time::TICKS_PER_SECOND`.
///
/// Chosen so 24/25/30/48/50/60, the 1001-family NTSC rates and 44.1/48/96/192
/// kHz all divide exactly — no drift accumulates over a long timeline.
let TICKS_PER_SECOND: Int64 = 705_600_000

extension Int64 {
    var asSeconds: Double { Double(self) / Double(TICKS_PER_SECOND) }
}
func ticks(seconds: Double) -> Int64 { Int64((seconds * Double(TICKS_PER_SECOND)).rounded()) }
