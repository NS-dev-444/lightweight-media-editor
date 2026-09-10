import SwiftUI
import AppKit
import UniformTypeIdentifiers
import QuartzCore
import Metal

/// The UI's view of the core.
///
/// Holds NO copy of the timeline — `clips` is refreshed from the Rust core,
/// which owns document truth (AD-1). Two mutable copies either side of the FFI
/// is the state-divergence bug the architecture explicitly avoids.
@MainActor
final class EditorDocument: ObservableObject {
    private let core: Core

    @Published private(set) var clips: ArraySlice<MCClipView> = []

    /// What each clip is called on the timeline: the source file's name, or the
    /// first words of a title. "clip 102" tells you nothing about your footage,
    /// and picking the right clip is most of what the timeline is for (§30).
    ///
    /// Cached rather than derived at draw time — the timeline redraws while
    /// scrubbing, and reading a name back across the FFI on every frame for
    /// every clip is exactly the per-frame chatter AD-1 exists to avoid.
    @Published private(set) var clipLabels: [UInt64: String] = [:]
    private var assetNames: [UInt64: String] = [:]
    @Published private(set) var timecode: String = "00:00:00:00"
    @Published private(set) var status: String = ""
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false
    @Published private(set) var isDirty = false
    @Published var pixelsPerSecond: Double = 60
    @Published var scrollTicks: Int64 = 0
    @Published private(set) var playhead: Int64 = 0
    @Published private(set) var notices: [String] = []

    /// Shared with the exporter so both use one decoder pool. Opening a
    /// decoder costs ~1054 ms (S1), so duplicating the pool for export would
    /// pay that again for every asset.
    let frameSource = FrameSource()
    /// The preview's audio path. Export uses its OWN mixer — audio decoders,
    /// like video decoders, are not safe for concurrent use.
    private let previewMixer = AudioMixer()
    private lazy var audioPlayer = AudioPlayer(mixer: previewMixer)

    @Published private(set) var isPlaying = false
    private var displayLink: CADisplayLink?
    private var lastTick: CFTimeInterval = 0

    /// Set when a drag began on a clip, so movement retargets that clip.
    private var draggingClip: UInt64?
    private var dragOriginTicks: Int64 = 0
    private var currentPath: URL?

    var trackCount: Int { max(core.trackCount, 1) }
    var duration: Int64 { core.duration }

    init?() {
        guard let c = Core() else { return nil }
        core = c
        refresh()
        // Files passed on the command line are imported at launch. This makes
        // the app testable end-to-end without driving a file panel, which
        // matters for CI and for the failure matrix in §42.
        let args = LaunchArguments.timelinePaths
        if !args.isEmpty { importPaths(args) }

        // --demo applies a strong, known effect to the first clip so the
        // compositor can be verified visually without driving the UI.
        // --title adds a sample title so the text path is verifiable without
        // driving menus.
        if CommandLine.arguments.contains("--title") {
            for (i, name) in textPresets.enumerated() {
                core.playhead = 0
                _ = core.addText(track: core.trackId(at: 1),
                                 start: ticks(seconds: Double(i) * 0.001),
                                 duration: ticks(seconds: 30),
                                 preset: Int32(i), text: "\(name) — the quick brown fox")
                break   // one at a time; presets are compared by re-running
            }
            refresh()
        }

        // --export <path> runs a real export headlessly, so the whole
        // compositor -> encoder path is testable without a save panel.
        if let i = CommandLine.arguments.firstIndex(of: "--export"),
           i + 1 < CommandLine.arguments.count {
            let out = CommandLine.arguments[i + 1]
            // --preset <id> picks one; otherwise the smallest, because a test
            // export should be quick.
            let wanted = CommandLine.arguments.firstIndex(of: "--preset")
                .flatMap { $0 + 1 < CommandLine.arguments.count ? CommandLine.arguments[$0 + 1] : nil }
            let preset = ExportPreset.all.first { $0.id == (wanted ?? "email") }
                ?? ExportPreset.all[0]
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                self.exportTo(URL(fileURLWithPath: out), preset: preset)
            }
        }

        // --image <path> adds an overlay so §20's transparency is verifiable.
        if let i = CommandLine.arguments.firstIndex(of: "--image"),
           i + 1 < CommandLine.arguments.count {
            addImageOverlay(path: CommandLine.arguments[i + 1])
        }

        // --rotate <quarter turns> and --reframe <aspect> apply framing to the
        // first clip at launch, so the geometry path is verifiable in the
        // preview and in a real export without driving the inspector.
        if let i = CommandLine.arguments.firstIndex(of: "--rotate"),
           i + 1 < CommandLine.arguments.count,
           let turns = Int32(CommandLine.arguments[i + 1]),
           let first = clips.first {
            rotate(first.clip_id, quarterTurns: turns)
        }
        if let i = CommandLine.arguments.firstIndex(of: "--reframe"),
           i + 1 < CommandLine.arguments.count,
           let aspect = Float(CommandLine.arguments[i + 1]),
           let first = clips.first {
            reframe(first.clip_id, aspect: aspect)
        }

        // --lut <path> applies a look at launch, so the colour path is
        // verifiable in an exported frame rather than by eye.
        if let i = CommandLine.arguments.firstIndex(of: "--lut"),
           i + 1 < CommandLine.arguments.count, let first = clips.first {
            applyLut(first.clip_id, path: CommandLine.arguments[i + 1])
        }

        // --transcribe runs transcription at launch, so the caption path is
        // verifiable end to end without driving the panel.
        if CommandLine.arguments.contains("--transcribe") {
            let lang = CommandLine.arguments.firstIndex(of: "--lang")
                .flatMap { $0 + 1 < CommandLine.arguments.count ? CommandLine.arguments[$0 + 1] : nil }
            transcribe(language: lang ?? "auto")
        }

        // --burnin turns burn-in on once a transcript exists, so an export can
        // be checked for captions in its actual pixels.
        if CommandLine.arguments.contains("--burnin") {
            setCaptionBurnIn(true)
        }

        // --select puts the first clip in the inspector, so the inspector can
        // be looked at without driving a click.
        if CommandLine.arguments.contains("--select"), let first = clips.first {
            core.selectOnly(first.clip_id)
            refresh()
        }

        // --desilence and --loudness exercise the analysis paths headlessly.
        if CommandLine.arguments.contains("--desilence"), let first = clips.first {
            removeSilence(first.clip_id)
        }
        if CommandLine.arguments.contains("--loudness"), let first = clips.first {
            normaliseLoudness(first.clip_id)
        }

        if CommandLine.arguments.contains("--demo"), let first = clips.first {
            core.selectOnly(first.clip_id)
            var e = Core.EffectValues()
            e.saturation = -0.6
            e.contrast = 0.5
            e.temperature = 0.7
            setEffects(first.clip_id, e)
        }
    }

    /// Import specific paths. Shared by the file panel and the launch arguments.
    func importPaths(_ paths: [String]) {
        var imported = 0
        var newNotices: [String] = []
        for path in paths {
            let id = core.importFile(path)
            if id == 0 {
                newNotices.append("\((path as NSString).lastPathComponent): \(core.lastError)")
                continue
            }
            imported += 1
            // O-3: surface HDR/VFR notices at import. Silent conversion is what
            // produces "why does my video look washed out".
            let name = (path as NSString).lastPathComponent
            newNotices.append(contentsOf: core.assetNotices(id).map { "\(name): \($0)" })

            // §3 makes audio first-class: an audio-only file belongs on an
            // audio track, not stacked on the video track.
            let hasVideo = core.assetHasVideo(id)
            let trackIndex = hasVideo ? 0 : 3
            let track = core.trackId(at: trackIndex)
            let dur = core.assetDuration(id)
            let end = lastEnd(onTrack: track)
            core.addClip(track: track, asset: id, start: end,
                         sourceStart: 0, sourceDuration: dur)
            if !hasVideo { loadWaveform(id) }
        }
        notices = newNotices
        status = imported > 0 ? "Imported \(imported) file\(imported == 1 ? "" : "s")"
                              : (newNotices.first ?? "Nothing was imported")
        refresh()
    }

    func refresh() {
        clips = core.clips()
        refreshLabels()
        playhead = core.playhead
        timecode = core.timecode
        canUndo = core.canUndo
        canRedo = core.canRedo
        isDirty = core.isDirty
    }

    private func refreshLabels() {
        var labels: [UInt64: String] = [:]
        for c in clips {
            if c.asset_id != 0 {
                if let cached = assetNames[c.asset_id] {
                    labels[c.clip_id] = cached
                } else {
                    let name = core.assetPath(c.asset_id)
                        .map { ($0 as NSString).lastPathComponent } ?? "clip \(c.clip_id)"
                    assetNames[c.asset_id] = name
                    labels[c.clip_id] = name
                }
            } else if let spec = core.clipText(c.clip_id), !spec.text.isEmpty {
                // A title can be a paragraph; the timeline needs the first few
                // words, which is all that fits and all that identifies it.
                labels[c.clip_id] = spec.text.count > 40
                    ? spec.text.prefix(39) + "…" : spec.text
            } else {
                labels[c.clip_id] = "Title"
            }
        }
        clipLabels = labels
    }

    private func report(_ ok: Bool, _ success: String) {
        status = ok ? success : core.lastError
        refresh()
    }

    // ---- file --------------------------------------------------------------

    func importMedia() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.message = "Choose video or audio to import"
        guard panel.runModal() == .OK else { return }
        importPaths(panel.urls.map(\.path))
    }

    func save() {
        if let p = currentPath { report(core.save(p.path), "Saved") } else { saveAs() }
    }

    /// §24: autosave engages as soon as the project has a home on disk.
    private func armAutosave(_ url: URL) {
        currentPath = url
        _ = core.enableAutosave(url.path)
    }

    /// Offer recovery when a journal survives — the app did not exit cleanly.
    func checkForRecovery(_ url: URL) {
        guard Core.hasUnrecoveredWork(url.path) else { return }
        let alert = NSAlert()
        alert.messageText = "Recover unsaved changes?"
        alert.informativeText = "This project has changes that were not saved, "
            + "probably because the app closed unexpectedly. You can recover them "
            + "or open the last saved version."
        alert.addButton(withTitle: "Recover")
        alert.addButton(withTitle: "Open Last Saved")
        if alert.runModal() == .alertFirstButtonReturn {
            if let n = core.recover(url.path) {
                status = "Recovered \(n) unsaved change\(n == 1 ? "" : "s")"
            } else {
                status = core.lastError
            }
        } else {
            Core.discardRecovery(url.path)
            _ = core.open(url.path)
            status = "Opened the last saved version"
        }
        armAutosave(url)
        refresh()
    }

    func saveAs() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Untitled.mcproj"
        panel.allowedContentTypes = []
        guard panel.runModal() == .OK, let url = panel.url else { return }
        report(core.save(url.path), "Saved")
        armAutosave(url)
    }

    func open() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        openProject(url)
    }

    /// Open a project by URL. Shared by the panel and by Finder, which reaches
    /// it through `AppDelegate`.
    func openProject(_ url: URL) {
        // Check BEFORE opening: recovery replaces the loaded document.
        if Core.hasUnrecoveredWork(url.path) { checkForRecovery(url); return }
        report(core.open(url.path), "Opened \(url.lastPathComponent)")
        armAutosave(url)
    }

    /// The extension a project is saved with, in one place so Finder routing
    /// and the save panel cannot disagree.
    static let projectExtension = "mcproj"

    // ---- editing -----------------------------------------------------------

    func undo() { report(core.undo(), "Undo") }
    func redo() { report(core.redo(), "Redo") }

    /// §29: `S` splits whatever sits under the playhead.
    func splitAtPlayhead() {
        let t = core.playhead
        for i in 0..<core.trackCount {
            if core.splitAt(track: core.trackId(at: i), ticks: t) {
                report(true, "Split"); return
            }
        }
        report(false, "")
    }

    func deleteSelection() {
        guard core.selectionCount > 0 else { status = "Nothing is selected"; return }
        report(core.deleteSelected(), "Deleted")
    }

    func rippleDeleteSelection() {
        let selected = clips.filter { $0.selected != 0 }
        guard let c = selected.first else { status = "Nothing is selected"; return }
        report(core.rippleDelete(track: c.track_id, clip: c.clip_id), "Rippled")
    }

    // ---- playhead & navigation --------------------------------------------

    func scrub(to t: Int64) { pause(); core.playhead = max(0, t); refresh() }
    func step(_ frames: Int64) { core.stepFrames(frames); refresh() }
    func goToStart() { core.playhead = 0; refresh() }
    func goToEnd() { core.playhead = core.duration; refresh() }

    func zoom(_ factor: Double) {
        pixelsPerSecond = min(max(pixelsPerSecond * factor, 1), 4000)
    }

    // ---- drag --------------------------------------------------------------

    func drag(to t: Int64, trackIndex: Int, isStart: Bool) {
        if isStart {
            // Hit-test: S2b measured a linear scan over 200 clips at ~8 us, so
            // no spatial index is warranted for V1.
            let hit = clips.first { c in
                Int(c.track_index) == trackIndex
                    && t >= c.start_ticks && t < c.start_ticks + c.duration_ticks
            }
            if let hit {
                core.selectOnly(hit.clip_id)
                draggingClip = hit.clip_id
                dragOriginTicks = t - hit.start_ticks
            } else {
                core.clearSelection()
                draggingClip = nil
                core.playhead = max(0, t)
            }
            refresh()
            return
        }
        guard let id = draggingClip else { core.playhead = max(0, t); refresh(); return }
        let targetTrack = core.trackId(at: min(max(trackIndex, 0), core.trackCount - 1))
        // A rejected move leaves the document untouched (proved in Phase 2), so
        // a drag that would overlap simply does not move — no error spam.
        _ = core.moveClip(id, toTrack: targetTrack, toStart: max(0, t - dragOriginTicks))
        refresh()
    }

    func endDrag() { draggingClip = nil }

    // ---- playback ----------------------------------------------------------

    /// §16 play/pause.
    ///
    /// The playhead advances by WALL-CLOCK delta, not by a fixed increment per
    /// tick. A dropped frame then costs a dropped frame, not a drift in time —
    /// which is what keeps long playback in sync with the audio clock.
    func togglePlay() { isPlaying ? pause() : play() }

    func play() {
        guard !isPlaying else { return }
        if core.playhead >= core.duration { core.playhead = 0 }
        isPlaying = true
        lastTick = CACurrentMediaTime()
        // Audio follows the playhead from where playback starts.
        let core = self.core
        audioPlayer.start(at: core.playhead,
                          layers: { Array(core.plan(at: core.playhead)) },
                          assetPath: { core.assetPath($0) })
        let link = NSScreen.main?.displayLink(target: self, selector: #selector(tick))
        link?.add(to: .main, forMode: .common)
        displayLink = link
    }

    func pause() {
        audioPlayer.stop()
        isPlaying = false
        displayLink?.invalidate()
        displayLink = nil
        refresh()
    }

    @objc private func tick(_ link: CADisplayLink) {
        let now = CACurrentMediaTime()
        let delta = now - lastTick
        lastTick = now
        let next = core.playhead + ticks(seconds: delta)
        if next >= core.duration {
            core.playhead = core.duration
            pause()                      // stop at the end rather than wrapping
            return
        }
        core.playhead = next
        refresh()
    }

    // ---- preview -----------------------------------------------------------

    /// What to draw right now (AD-4).
    func currentPlan() -> ArraySlice<MCPlanLayer> { core.plan(at: core.playhead) }

    func assetPath(_ id: UInt64) -> String? { core.assetPath(id) }

    // ---- §19 text ----------------------------------------------------------

    var textPresets: [String] { core.textPresetNames }

    /// Add a title on the overlay track, starting at the playhead.
    func addText(preset: Int = 0, text: String = "Your title here") {
        let overlay = core.trackId(at: 1)
        let dur = ticks(seconds: 4.0)
        let id = core.addText(track: overlay, start: core.playhead,
                              duration: dur, preset: Int32(preset), text: text)
        if id == 0 { status = core.lastError } else {
            core.selectOnly(id)
            status = "Added title"
        }
        refresh()
    }

    func text(for clip: UInt64) -> TextSpec? { core.clipText(clip) }
    func image(for clip: UInt64) -> ImageSpec? { core.clipImage(clip) }
    func setImage(_ clip: UInt64, _ spec: ImageSpec) { _ = core.setClipImage(clip, spec); refresh() }

    /// §20: add a logo or graphic on the overlay track.
    func addImageOverlay() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.png, .jpeg, .webP, .tiff, .heic]
        panel.message = "Choose an image to overlay"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        addImageOverlay(path: url.path)
    }

    func addImageOverlay(path: String) {
        // Overlay 2, so a title on overlay 1 and a logo can coexist.
        let id = core.addImage(track: core.trackId(at: 2), start: core.playhead,
                               duration: ticks(seconds: 4.0), path: path)
        if id == 0 { status = core.lastError } else { core.selectOnly(id); status = "Added image" }
        refresh()
    }
    func setText(_ clip: UInt64, _ spec: TextSpec) { _ = core.setClipText(clip, spec); refresh() }

    // ---- §18 effects -------------------------------------------------------

    func effects(for clip: UInt64) -> Core.EffectValues { core.effects(clip) }

    /// Every change is a normal undoable edit — §28 covers effects too.
    func setEffects(_ clip: UInt64, _ e: Core.EffectValues) {
        _ = core.setEffects(clip, e)
        refresh()
    }

    func resetEffects(_ clip: UInt64) { setEffects(clip, Core.EffectValues()) }

    // ---- voiceover ------------------------------------------------------------

    let recorder = VoiceRecorder()

    /// Record a voiceover and put it on an audio track at the playhead.
    ///
    /// The take lands beside the project when there is one, and in the
    /// Movies folder when there is not — never in a temporary directory, which
    /// is where recordings go to be lost.
    func startVoiceover() {
        let dir = currentPath?.deletingLastPathComponent()
            ?? FileManager.default.urls(for: .moviesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        let stamp = ISO8601DateFormatter().string(from: Date())
            .replacingOccurrences(of: ":", with: "-")
        let url = dir.appendingPathComponent("Voiceover \(stamp).wav")
        let at = core.playhead
        recorder.start(to: url) { [weak self] path in
            Task { @MainActor in
                guard let self, let path else { return }
                self.placeVoiceover(path, at: at)
            }
        }
    }

    func stopVoiceover() { recorder.stop() }

    /// Put a finished take on the first audio track with room at that point.
    private func placeVoiceover(_ path: String, at: Int64) {
        let id = core.importFile(path)
        guard id != 0 else { status = core.lastError; refresh(); return }
        let dur = core.assetDuration(id)
        // Audio tracks are indices 3 and 4 in V1's layout.
        for index in [3, 4] {
            let track = core.trackId(at: index)
            guard track != 0 else { continue }
            let clash = clips.contains { c in
                c.track_id == track && c.start_ticks < at + dur
                    && c.start_ticks + c.duration_ticks > at
            }
            if clash { continue }
            core.addClip(track: track, asset: id, start: at, sourceStart: 0, sourceDuration: dur)
            loadWaveform(id)
            report(true, "Voiceover added")
            return
        }
        // Saved, but not placed: the take is not lost, and the message says so.
        status = "The recording was saved to \((path as NSString).lastPathComponent), "
               + "but both audio tracks are busy at the playhead."
        refresh()
    }

    // ---- captions (§8) -------------------------------------------------------

    let captions = CaptionsModel()

    /// The text spec for a burned-in caption at an instant, or nil.
    ///
    /// Captions reuse the TEXT PRESETS rather than having a look of their own —
    /// `Subtitle` exists for exactly this and is outlined for legibility over
    /// any footage.
    func captionSpec(at ticks: Int64) -> TextSpec? {
        let words = core.captionText(at: ticks)
        guard !words.isEmpty else { return nil }
        var spec = core.textPresetSpec(Int32(captions.preset), text: words)
        spec?.text = words
        return spec
    }

    func refreshCaptions() { captions.refresh(from: core) }

    /// Transcribe the audio under the playhead — or the first clip with sound.
    ///
    /// One clip, not the whole timeline: transcription is expensive and the
    /// thing people caption is a take, not an assembly. Doing the timeline
    /// would also mean rendering a mixdown first, which is a different feature.
    func transcribe(language: String = "auto") {
        guard let model = CaptionsModel.bundledModel else {
            status = "This build has no transcription model. Run tools/fetch_models.sh."
            return
        }
        let candidates = clips.filter { $0.track_kind == 2 || $0.track_kind == 0 }
        let under = candidates.first { c in
            core.playhead >= c.start_ticks && core.playhead < c.start_ticks + c.duration_ticks
        }
        guard let clip = under ?? candidates.first,
              let path = core.assetPath(clip.asset_id) else {
            status = "There is nothing on the timeline to transcribe"
            return
        }
        let seconds = Double(clip.duration_ticks) / Double(TICKS_PER_SECOND)
        captions.transcribe(path: path, model: model, language: language,
                            into: core, durationSeconds: seconds) { [weak self] in
            self?.refresh()
        }
    }

    func importSubtitles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Choose an SRT or WebVTT file"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let n = core.importSubtitles(url.path)
        if n < 0 { status = core.lastError; return }
        refreshCaptions()
        report(true, "Read \(n) caption\(n == 1 ? "" : "s")")
    }

    func exportSubtitles() {
        guard core.captionCount > 0 else { status = "There are no captions to export"; return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue =
            (currentPath?.deletingPathExtension().lastPathComponent ?? "Captions") + ".srt"
        panel.message = "Save captions as SRT or WebVTT (.srt or .vtt)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        report(core.exportSubtitles(url.path), "Saved \(url.lastPathComponent)")
    }

    func setCaptionText(_ index: Int, _ text: String) {
        guard core.setCaptionText(index, text) else { status = core.lastError; return }
        refreshCaptions()
        refresh()
    }

    func removeCaption(_ index: Int) {
        guard core.removeCaption(index) else { status = core.lastError; return }
        refreshCaptions()
        report(true, "Caption removed")
    }

    /// Nudge the whole transcript earlier or later — the usual fix when
    /// captions are consistently out of step.
    func shiftCaptions(seconds: Double) {
        guard core.shiftCaptions(ticks(seconds: seconds)) else { status = core.lastError; return }
        refreshCaptions()
        report(true, String(format: "Captions shifted %+.1fs", seconds))
    }

    func setCaptionBurnIn(_ on: Bool) {
        guard core.setCaptionBurnIn(on, preset: captions.preset) else { return }
        refreshCaptions()
        report(true, on ? "Captions will be burned in" : "Captions will not be burned in")
    }

    func setCaptionPreset(_ preset: Int32) {
        guard core.setCaptionBurnIn(captions.burnIn, preset: preset) else { return }
        refreshCaptions()
        refresh()
    }

    // ---- colour: LUTs --------------------------------------------------------

    func lut(for clip: UInt64) -> Core.LutSpec? { core.lut(clip) }

    /// Apply a .cube LUT. The file is validated here rather than at draw time,
    /// so a bad file produces a sentence now instead of a clip that silently
    /// looks unchanged.
    func applyLut(_ clip: UInt64, path: String, amount: Float = 1.0) {
        do {
            _ = try CubeLUT.load(path: path)
        } catch {
            status = "\(error)"
            return
        }
        report(core.setLut(clip, path: path, amount: amount),
               "Applied \((path as NSString).lastPathComponent)")
    }

    func setLutAmount(_ clip: UInt64, _ amount: Float) {
        guard let spec = core.lut(clip) else { return }
        report(core.setLut(clip, path: spec.path, amount: amount), "Look adjusted")
    }

    func removeLut(_ clip: UInt64) {
        report(core.setLut(clip, path: "", amount: 1), "Look removed")
    }

    func chooseLut(_ clip: UInt64) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Choose a .cube LUT"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        applyLut(clip, path: url.path)
    }

    // ---- sound --------------------------------------------------------------

    func levels(for clip: UInt64) -> Core.Levels { core.levels(clip) }

    func setGain(_ clip: UInt64, _ gain: Double) {
        report(core.setGain(clip, gain), "Volume changed")
    }

    func setFades(_ clip: UInt64, inSeconds: Double, outSeconds: Double) {
        report(core.setFades(clip, ticks(seconds: inSeconds), ticks(seconds: outSeconds)),
               "Fade changed")
    }

    /// §3's "detach audio": the sound moves to its own track so it can be
    /// trimmed, moved and levelled independently of the picture.
    func detachAudio(_ clip: UInt64) {
        let id = core.detachAudio(clip)
        if id == 0 { status = core.lastError; refresh(); return }
        loadWaveform(clipAsset(id) ?? 0)
        report(true, "Audio detached")
    }

    /// Trim a music clip to the length of the video, with a fade at the end.
    func fitToLength(_ clip: UInt64) {
        let video = clips.filter { $0.track_kind == 0 }
            .map { $0.start_ticks + $0.duration_ticks }.max() ?? 0
        guard video > 0 else { status = "There is no video to fit the music to"; return }
        guard let c = clips.first(where: { $0.clip_id == clip }) else { return }
        let target = video - c.start_ticks
        // Two seconds of fade, or a quarter of the clip if it is short. Music
        // that simply stops sounds like a mistake; a fade sounds deliberate.
        let fade = min(ticks(seconds: 2), target / 4)
        report(core.fitToLength(clip, target: target, fade: fade), "Fitted to the video")
    }

    /// §3's loudness targeting. -14 LUFS is what the streaming platforms
    /// normalise to, so hitting it means they leave the audio alone.
    static let loudnessTarget: Double = -14

    func normaliseLoudness(_ clip: UInt64) {
        guard let asset = clipAsset(clip), let path = core.assetPath(asset) else {
            status = "That clip has no file to measure"; return
        }
        status = "Measuring loudness…"
        let target = Self.loudnessTarget
        Task.detached(priority: .userInitiated) {
            let lufs = Core.loudness(ofFile: path)
            let peak = Core.samplePeak(ofFile: path)
            await MainActor.run { [weak self] in
                guard let self else { return }
                guard lufs > -100 else {
                    self.status = "There is no measurable sound in this clip"
                    return
                }
                var gain = pow(10, (target - lufs) / 20)
                // Refuse to clip. Raising a quiet-but-peaky recording to -14
                // LUFS can push samples past full scale, and distortion is a
                // worse outcome than being a little quiet.
                if peak > 0, peak * gain > 0.99 {
                    gain = 0.99 / peak
                    self.setGain(clip, gain)
                    self.status = String(format:
                        "Raised as far as it goes without distorting (%.1f LUFS → about %.1f)",
                        lufs, lufs + 20 * log10(gain))
                    return
                }
                self.setGain(clip, gain)
                self.status = String(format: "%.1f LUFS → %.0f LUFS", lufs, target)
            }
        }
    }

    /// §3's silence removal, and PRODUCT_DIRECTION §7's primary gesture:
    /// cutting the boring parts out of one long take.
    func removeSilence(_ clip: UInt64, thresholdDb: Double = -40,
                       minMs: Int32 = 700, padMs: Int32 = 120) {
        guard let asset = clipAsset(clip), let path = core.assetPath(asset) else {
            status = "That clip has no file to analyse"; return
        }
        guard let c = clips.first(where: { $0.clip_id == clip }) else { return }
        status = "Listening for gaps…"
        let sourceStart = c.source_start_ticks
        let clipStart = c.start_ticks
        let clipEnd = c.start_ticks + c.duration_ticks
        let speed = c.speed
        let track = c.track_id

        Task.detached(priority: .userInitiated) {
            let ranges = Core.silenceRanges(inFile: path, thresholdDb: thresholdDb,
                                            minMs: minMs, padMs: padMs)
            await MainActor.run { [weak self] in
                guard let self else { return }
                // Source time -> timeline time, and only the part of the file
                // this clip actually uses.
                let onTimeline: [(Int64, Int64)] = ranges.compactMap { r in
                    let s = clipStart + Int64(Double(r.0 - sourceStart) / speed)
                    let e = clipStart + Int64(Double(r.1 - sourceStart) / speed)
                    let a = max(s, clipStart), b = min(e, clipEnd)
                    return b > a ? (a, b) : nil
                }
                guard !onTimeline.isEmpty else {
                    self.status = "No silences long enough to cut"; return
                }
                // BACK TO FRONT, so each ripple does not move the ranges that
                // have not been cut yet.
                var removed = 0
                for (s, e) in onTimeline.sorted(by: { $0.0 > $1.0 }) {
                    if self.core.rippleDeleteRange(track: track, start: s, end: e) {
                        removed += 1
                    }
                }
                self.status = removed > 0
                    ? "Cut \(removed) silence\(removed == 1 ? "" : "s")"
                    : "Those gaps could not be cut"
                self.refresh()
            }
        }
    }

    /// §3's ducking: music steps aside under a voice.
    ///
    /// The voice clip is chosen automatically — the audio clip that overlaps
    /// this one and is not this one. With V1's two audio tracks that is
    /// unambiguous, and asking "which voice?" for the only other clip on the
    /// timeline is a dialogue that adds nothing.
    func duck(_ music: UInt64) {
        guard let m = clips.first(where: { $0.clip_id == music }) else { return }
        let span = (m.start_ticks, m.start_ticks + m.duration_ticks)
        let voice = clips.first { c in
            c.clip_id != music && c.track_kind == 2
                && c.start_ticks < span.1 && c.start_ticks + c.duration_ticks > span.0
        }
        guard let voice else {
            status = "There is no other audio clip over this music to duck under"
            return
        }
        let n = core.duck(music: music, under: voice.clip_id)
        if n < 0 { status = core.lastError; refresh(); return }
        report(true, "Ducked under \(n) passage\(n == 1 ? "" : "s")")
    }

    func clearDucking(_ clip: UInt64) {
        report(core.clearGainPoints(clip), "Volume curve removed")
    }

    func isDucked(_ clip: UInt64) -> Bool { core.gainPointCount(clip) > 0 }

    /// The asset a clip reads from, or nil.
    private func clipAsset(_ clip: UInt64) -> UInt64? {
        clips.first { $0.clip_id == clip }.map(\.asset_id)
    }

    // ---- framing: crop, rotate, flip, reframe -------------------------------

    func geometry(for clip: UInt64) -> Core.GeometryValues { core.geometry(clip) }

    /// The shape the preview is currently showing, for overlays that must land
    /// on the picture rather than on the letterbox around it. Falls back to
    /// 16:9 when nothing is under the playhead.
    var previewAspect: CGFloat {
        guard let base = clips.first(where: { $0.track_kind == 0 }) else { return 16.0 / 9.0 }
        let a = core.clipAspect(base.clip_id)
        return a > 0 ? CGFloat(a) : 16.0 / 9.0
    }

    func setGeometry(_ clip: UInt64, _ g: Core.GeometryValues) {
        report(core.setGeometry(clip, g), "Framing changed")
    }

    func resetGeometry(_ clip: UInt64) { setGeometry(clip, Core.GeometryValues()) }

    /// Quarter-turn a clip. Repeated turns wrap, so four presses come back to
    /// where they started rather than sticking at 3.
    func rotate(_ clip: UInt64, quarterTurns: Int32) {
        var g = core.geometry(clip)
        g.rotation = (g.rotation + quarterTurns) % 4
        if g.rotation < 0 { g.rotation += 4 }
        setGeometry(clip, g)
    }

    /// The aspect ratios people actually publish to, named by where they go.
    /// PRODUCT_DIRECTION §7: users think in destinations, not in numbers.
    static let reframeTargets: [(name: String, aspect: Float)] = [
        ("Original", 0),
        ("Vertical 9:16", 9.0 / 16.0),
        ("Square 1:1", 1),
        ("Portrait 4:5", 4.0 / 5.0),
        ("Widescreen 16:9", 16.0 / 9.0),
    ]

    /// Reframe every picture clip at once — "make the whole thing vertical".
    func reframeAll(aspect: Float) {
        let picture = clips.filter { $0.track_kind != 2 }
        guard !picture.isEmpty else { status = "There is nothing to reframe"; return }
        for c in picture { reframe(c.clip_id, aspect: aspect) }
        status = aspect > 0 ? "Reframed \(picture.count) clip\(picture.count == 1 ? "" : "s")"
                            : "Framing reset"
    }

    /// Rotate whatever is selected, or everything if nothing is.
    func rotateSelection(_ quarterTurns: Int32) {
        var targets = clips.filter { $0.selected != 0 && $0.track_kind != 2 }
        if targets.isEmpty { targets = clips.filter { $0.track_kind != 2 } }
        guard !targets.isEmpty else { status = "There is nothing to rotate"; return }
        for c in targets { rotate(c.clip_id, quarterTurns: quarterTurns) }
    }

    func reframe(_ clip: UInt64, aspect: Float) {
        if aspect <= 0 {
            // "Original" is the undo of a reframe, and it has to leave rotation
            // and flips alone — they are not part of the framing decision.
            var g = core.geometry(clip)
            g.cropX = 0; g.cropY = 0; g.cropW = 1; g.cropH = 1
            setGeometry(clip, g)
            return
        }
        report(core.reframe(clip, aspect: aspect), "Reframed")
    }

    /// Where the next clip should land on a given track.
    private func lastEnd(onTrack track: UInt64) -> Int64 {
        core.clips().filter { $0.track_id == track }
            .map { $0.start_ticks + $0.duration_ticks }.max() ?? 0
    }

    // ---- waveforms ---------------------------------------------------------

    /// Cached per asset. §17 requires generation off the UI thread; S7 measured
    /// zero main-thread stalls doing exactly this.
    @Published private(set) var waveforms: [UInt64: [Float]] = [:]

    private func loadWaveform(_ asset: UInt64) {
        guard waveforms[asset] == nil else { return }
        let core = self.core
        Task.detached(priority: .utility) {
            let peaks = core.waveform(asset)
            await MainActor.run { self.waveforms[asset] = peaks }
        }
    }

    /// Files dropped from Finder (§22).
    func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        var paths: [String] = []
        let group = DispatchGroup()
        for p in providers where p.hasItemConformingToTypeIdentifier("public.file-url") {
            group.enter()
            p.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                defer { group.leave() }
                if let data = item as? Data,
                   let url = URL(dataRepresentation: data, relativeTo: nil) {
                    paths.append(url.path)
                } else if let url = item as? URL {
                    paths.append(url.path)
                }
            }
        }
        group.notify(queue: .main) { [weak self] in
            guard let self, !paths.isEmpty else { return }
            self.importPaths(paths.sorted())
        }
        return true
    }

    // ---- §12 export --------------------------------------------------------

    @Published private(set) var exportProgress: Double?     // nil = not exporting
    @Published private(set) var exportMessage: String = ""
    private var exporter: Exporter?

    var exportPresets: [ExportPreset] { ExportPreset.all }

    /// §12: the default hides codecs and bitrates behind a named preset.
    func export(preset: ExportPreset) {
        guard duration > 0 else { status = "There is nothing on the timeline to export"; return }

        // A preset that promises a size has to say so BEFORE the export runs
        // when it cannot keep the promise. Finding out afterwards, from a file
        // that will not attach, is the version of this that wastes the time.
        if let warning = preset.sizeWarning(forSeconds: duration.asSeconds) {
            let alert = NSAlert()
            alert.messageText = "This will not fit in \(Int(preset.targetMegabytes ?? 0)) MB"
            alert.informativeText = warning
            alert.addButton(withTitle: "Export Anyway")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }

        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Export.mp4"
        panel.message = "Export \(preset.name)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        exportTo(url, preset: preset)
    }

    /// Export to a known path. Used by the panel and by `--export`, so the
    /// headless path exercises exactly the same code.
    func exportTo(_ url: URL, preset: ExportPreset) {

        // Snapshot everything the export needs, so editing during an export
        // cannot change what is being written.
        let dur = duration
        // Follow the PROJECT's rate. Exporting a 24 fps project at 30 changes
        // every clip's duration and desyncs anything cut to the beat.
        let rate = core.exportFPS
        let captionPreset = captions.preset
        let core = self.core
        // A SEPARATE decoder pool. FFmpeg decoders are not safe for concurrent
        // use, and sharing the preview's pool corrupted the encode.
        let frames = FrameSource()
        pause()                 // and stop the preview decoding underneath it
        exportProgress = 0
        exportMessage = "Starting…"

        let ex = Exporter(device: MTLCreateSystemDefaultDevice())
        exporter = ex
        Task.detached(priority: .userInitiated) {
            guard let ex else {
                await MainActor.run { self.finishExport("The graphics system is unavailable.") }
                return
            }
            let err = ex.export(
                to: url.path, preset: preset, durationTicks: dur,
                fpsNum: rate.num, fpsDen: rate.den,
                plan: { Array(core.plan(at: $0)) },
                assetPath: { core.assetPath($0) },
                textSpec: { core.clipText($0) },
                imageSpec: { core.clipImage($0) },
                lutSpec: { core.lut($0) },
                // Built from `core` directly rather than through the document:
                // this closure runs off the main actor, and the preset is
                // snapshotted with everything else before the export starts.
                captionSpec: { t in
                    let words = core.captionText(at: t)
                    guard !words.isEmpty else { return nil }
                    var spec = core.textPresetSpec(captionPreset, text: words)
                    spec?.text = words
                    return spec
                },
                frames: frames,
                progress: { p in
                    Task { @MainActor in
                        self.exportProgress = Double(p.frame) / Double(max(p.total, 1))
                        self.exportMessage = "Frame \(p.frame) of \(p.total) — "
                            + String(format: "%.0f fps", p.fps)
                    }
                })
            frames.closeAll()
            await MainActor.run {
                self.finishExport(err ?? "Exported to \(url.lastPathComponent)")
            }
        }
    }

    func cancelExport() { exporter?.cancel(); exportMessage = "Cancelling…" }

    private func finishExport(_ message: String) {
        exportProgress = nil
        exporter = nil
        status = message
        exportMessage = ""
    }

    func dismissNotices() { notices = [] }
}
