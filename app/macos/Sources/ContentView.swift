import SwiftUI

/// §31's three modes, as ratified in PRODUCT_DIRECTION.md §4.
///
/// Three, not four: the separate AUDIO mode is gone. Its useful parts live in
/// EDIT, where the work actually happens — a dedicated audio mode is what
/// pushed §3 toward rebuilding Audacity.
enum AppMode: String, CaseIterable, Identifiable {
    case edit = "Edit", convert = "Convert", tools = "Tools"
    var id: String { rawValue }
}

struct ContentView: View {
    @ObservedObject var doc: EditorDocument
    @ObservedObject var queue: ConversionQueue
    @ObservedObject private var openRequests = OpenRequests.shared
    // --mode edit|convert|tools makes each mode reachable for testing without
    // driving the segmented control.
    @State private var mode: AppMode = {
        if let i = CommandLine.arguments.firstIndex(of: "--mode"),
           i + 1 < CommandLine.arguments.count,
           let m = AppMode.allCases.first(where: {
               $0.rawValue.lowercased() == CommandLine.arguments[i + 1].lowercased() }) {
            return m
        }
        return .edit
    }()

    var body: some View {
        VStack(spacing: 0) {
            Picker("", selection: $mode) {
                ForEach(AppMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .frame(width: 280)
            .padding(.vertical, 6)

            Divider()

            switch mode {
            case .edit:    EditModeView(doc: doc)
            case .convert: ConvertView(queue: queue)
            case .tools:   ToolsView(queue: queue)
            }
        }
        .background(Color(white: 0.12))
        // Files from Finder, the Dock, or `open Editor.app file.mp4`. They can
        // arrive before this view exists, so the mailbox is drained on first
        // appearance as well as on every later delivery.
        .onAppear { handleOpenRequests() }
        .onChange(of: openRequests.urls) { _, _ in handleOpenRequests() }
    }

    /// Route what was opened by what it is, not by asking the user.
    ///
    /// A project replaces the document — that is what opening a project means.
    /// Media is *added*, and where it goes follows the mode you are already in:
    /// dropping a file on the app while converting means "convert this", not
    /// "abandon what I was doing and edit it". Mixed selections do both.
    private func handleOpenRequests() {
        let urls = openRequests.drain()
        guard !urls.isEmpty else { return }
        let isProject = { (u: URL) in
            u.pathExtension.lowercased() == EditorDocument.projectExtension
        }
        var target = mode
        if let project = urls.first(where: isProject) {
            target = .edit
            mode = .edit
            doc.openProject(project)
        }
        let media = urls.filter { !isProject($0) }.map(\.path)
        guard !media.isEmpty else { return }
        switch target {
        case .edit:              doc.importPaths(media)
        case .convert, .tools:   queue.add(paths: media)
        }
    }
}

/// §30's layout: media library, preview, inspector, timeline underneath.
/// "Do not overload the interface" — so this is exactly those four regions.
struct EditModeView: View {
    @ObservedObject var doc: EditorDocument
    @State private var showCaptions = CommandLine.arguments.contains("--captions")

    var body: some View {
        VSplitView {
            HSplitView {
                LibraryPane(doc: doc).frame(minWidth: 170, idealWidth: 220, maxWidth: 320)
                PreviewPane(doc: doc).frame(minWidth: 320)
                // Captions get their own pane rather than a section of the
                // inspector: correcting a transcript is a sustained reading
                // task, and it needs the height.
                if showCaptions {
                    CaptionsPane(doc: doc, captions: doc.captions)
                        .frame(minWidth: 250, idealWidth: 290, maxWidth: 380)
                } else {
                    InspectorPane(doc: doc)
                        .frame(minWidth: 230, idealWidth: 250, maxWidth: 320)
                }
            }
            .frame(minHeight: 220)

            VStack(spacing: 0) {
                TransportBar(doc: doc)
                TimelineView(doc: doc)
            }
            .frame(minHeight: 200)
        }
        .background(Color(white: 0.12))
        .overlay(alignment: .top) { NoticeBanner(doc: doc) }
        .overlay(alignment: .center) { ExportOverlay(doc: doc) }
    }
}

/// O-3 requires HDR conversion to be DISCLOSED, never silent, and §23 requires
/// human wording. Import problems surface here rather than in a modal.
struct NoticeBanner: View {
    @ObservedObject var doc: EditorDocument
    var body: some View {
        if !doc.notices.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(doc.notices, id: \.self) { n in
                    Text(n).font(.system(size: 11)).foregroundColor(.white)
                }
                Button("Dismiss") { doc.dismissNotices() }
                    .font(.system(size: 10))
            }
            .padding(10)
            .background(Color(red: 0.45, green: 0.30, blue: 0.10))
            .cornerRadius(6)
            .padding(8)
        }
    }
}

struct LibraryPane: View {
    @ObservedObject var doc: EditorDocument
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Media").font(.system(size: 11, weight: .semibold))
                .foregroundColor(.secondary)
            Button(action: doc.importMedia) {
                Label("Import…", systemImage: "plus")
            }
            .keyboardShortcut("i", modifiers: [.command])
            Divider()
            Text("\(doc.clips.count) clip\(doc.clips.count == 1 ? "" : "s") on the timeline")
                .font(.system(size: 11)).foregroundColor(.secondary)
            Spacer()
        }
        .padding(10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(white: 0.14))
    }
}

struct InspectorPane: View {
    @ObservedObject var doc: EditorDocument
    var body: some View {
        let selected = doc.clips.filter { $0.selected != 0 }
        // The inspector grew past the height of any window once framing, look
        // and sound joined the effects, and an un-scrolled VStack simply
        // overflows — quietly, taking the media pane's width with it.
        ScrollView {
        VStack(alignment: .leading, spacing: 8) {
            Text("Inspector").font(.system(size: 11, weight: .semibold))
                .foregroundColor(.secondary)
            if selected.isEmpty {
                Text("No clip selected").font(.system(size: 11)).foregroundColor(.secondary)
            } else if selected.count > 1 {
                Text("\(selected.count) clips selected").font(.system(size: 11))
            } else if let c = selected.first {
                row("Clip", "\(c.clip_id)")
                row("Start", String(format: "%.2fs", c.start_ticks.asSeconds))
                row("Duration", String(format: "%.2fs", c.duration_ticks.asSeconds))
                row("Speed", String(format: "%.2f×", c.speed))
                row("Volume", String(format: "%.0f%%", c.gain * 100))
                Divider()
                SoundControls(doc: doc, clip: c)
                if c.track_kind != 2 {   // framing and colour are picture-only
                    Divider()
                    FramingControls(doc: doc, clip: c.clip_id)
                    Divider()
                    LookControls(doc: doc, clip: c.clip_id)
                    Divider()
                    EffectControls(doc: doc, clip: c.clip_id)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(white: 0.14))
    }

    private func row(_ k: String, _ v: String) -> some View {
        HStack {
            Text(k).font(.system(size: 11)).foregroundColor(.secondary)
            Spacer()
            Text(v).font(.system(size: 11, design: .monospaced))
        }
    }
}

/// §16: play/pause, frame step, current time, duration.
struct TransportBar: View {
    @ObservedObject var doc: EditorDocument
    var body: some View {
        HStack(spacing: 12) {
            Button { doc.goToStart() } label: { Image(systemName: "backward.end.fill") }
            Button { doc.togglePlay() } label: {
                Image(systemName: doc.isPlaying ? "pause.fill" : "play.fill")
            }
            Button { doc.step(-1) } label: { Image(systemName: "backward.frame.fill") }
            Button { doc.step(1) } label: { Image(systemName: "forward.frame.fill") }
            Button { doc.goToEnd() } label: { Image(systemName: "forward.end.fill") }

            Text(doc.timecode)
                .font(.system(size: 12, design: .monospaced))
                .foregroundColor(.white)
            Text("/ " + String(format: "%.2fs", doc.duration.asSeconds))
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(.secondary)

            // While recording, the meter and the elapsed time replace nothing
            // else — a take with no level showing is a take you find out was
            // silent afterwards.
            if doc.recorder.state.isRecording {
                HStack(spacing: 6) {
                    Circle().fill(.red).frame(width: 8, height: 8)
                    Text(String(format: "%.1fs", doc.recorder.seconds))
                        .font(.system(size: 11, design: .monospaced))
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            Capsule().fill(.white.opacity(0.15))
                            Capsule()
                                .fill(doc.recorder.level > 0.95 ? Color.red : Color.green)
                                .frame(width: geo.size.width * CGFloat(doc.recorder.level))
                        }
                    }
                    .frame(width: 70, height: 6)
                    Button("Stop") { doc.stopVoiceover() }.font(.system(size: 11))
                }
            }

            Spacer()

            if !doc.status.isEmpty {
                Text(doc.status).font(.system(size: 11)).foregroundColor(.secondary)
            }
            Button { doc.zoom(1/1.4) } label: { Image(systemName: "minus.magnifyingglass") }
            Button { doc.zoom(1.4) } label: { Image(systemName: "plus.magnifyingglass") }
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(Color(white: 0.16))
    }
}


/// §18's effect set — and no more. "Do not build a giant effects marketplace."
/// §3's audio-as-ingredient controls: level, fades, and the one-click
/// operations that are otherwise fiddly by hand.
///
/// "Match Loudness" and "Remove Silence" sit next to the sliders rather than in
/// a separate audio mode, because they are things you do to a clip in the
/// middle of editing — PRODUCT_DIRECTION §4's argument for deleting the
/// separate AUDIO mode applies to their placement too.
struct SoundControls: View {
    @ObservedObject var doc: EditorDocument
    let clip: MCClipView

    var body: some View {
        let l = doc.levels(for: clip.clip_id)
        VStack(alignment: .leading, spacing: 6) {
            Text("Sound").font(.system(size: 11, weight: .semibold))
                .foregroundColor(.secondary)

            VStack(alignment: .leading, spacing: 1) {
                HStack {
                    Text("Volume").font(.system(size: 10)).foregroundColor(.secondary)
                    Spacer()
                    // Decibels, not percent: doubling a percentage does not
                    // double what you hear, and everyone who edits audio
                    // already thinks in dB.
                    Text(l.gain <= 0.0001 ? "silent"
                         : String(format: "%+.1f dB", 20 * log10(l.gain)))
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)
                }
                Slider(value: Binding(get: { l.gain }, set: { doc.setGain(clip.clip_id, $0) }),
                       in: 0...2)
                    .controlSize(.mini)
            }

            fade("Fade in", l.fadeIn) {
                doc.setFades(clip.clip_id, inSeconds: $0,
                             outSeconds: l.fadeOut.asSeconds)
            }
            fade("Fade out", l.fadeOut) {
                doc.setFades(clip.clip_id, inSeconds: l.fadeIn.asSeconds,
                             outSeconds: $0)
            }

            HStack(spacing: 6) {
                if clip.track_kind == 0 {
                    Button("Detach") { doc.detachAudio(clip.clip_id) }
                        .help("Move this clip's sound onto its own track")
                }
                Button("Match Loudness") { doc.normaliseLoudness(clip.clip_id) }
                    .help("Set the level to -14 LUFS, what streaming platforms use")
            }
            .font(.system(size: 11))

            HStack(spacing: 6) {
                Button("Remove Silence") { doc.removeSilence(clip.clip_id) }
                    .help("Cut the gaps out of this take and close them up")
                if clip.track_kind == 2 {
                    Button("Fit to Video") { doc.fitToLength(clip.clip_id) }
                        .help("Trim this music to the length of the video, with a fade")
                }
            }
            .font(.system(size: 11))

            if clip.track_kind == 2 {
                HStack(spacing: 6) {
                    if doc.isDucked(clip.clip_id) {
                        Button("Remove Ducking") { doc.clearDucking(clip.clip_id) }
                    } else {
                        Button("Duck Under Voice") { doc.duck(clip.clip_id) }
                            .help("Lower this music while someone is speaking")
                    }
                }
                .font(.system(size: 11))
            }
        }
    }

    private func fade(_ label: String, _ ticks: Int64,
                      _ set: @escaping (Double) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack {
                Text(label).font(.system(size: 10)).foregroundColor(.secondary)
                Spacer()
                Text(String(format: "%.2fs", ticks.asSeconds))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
            }
            Slider(value: Binding(get: { ticks.asSeconds }, set: { set($0) }), in: 0...5)
                .controlSize(.mini)
        }
    }
}

/// Crop, rotate, flip and reframe (PRODUCT_DIRECTION.md §6).
///
/// Reframe sits at the top because it is the operation creators actually
/// perform — "make this vertical" — and it writes the crop the manual sliders
/// would otherwise have to be nudged into. The sliders stay for the times a
/// centred crop is the wrong crop.
struct FramingControls: View {
    @ObservedObject var doc: EditorDocument
    let clip: UInt64

    var body: some View {
        let g = doc.geometry(for: clip)
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Framing").font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                Spacer()
                Button("Reset") { doc.resetGeometry(clip) }.font(.system(size: 10))
            }

            HStack(spacing: 4) {
                Button { doc.rotate(clip, quarterTurns: -1) } label: {
                    Image(systemName: "rotate.left")
                }.help("Rotate left")
                Button { doc.rotate(clip, quarterTurns: 1) } label: {
                    Image(systemName: "rotate.right")
                }.help("Rotate right")
                Button {
                    var n = g; n.flipH.toggle(); doc.setGeometry(clip, n)
                } label: { Image(systemName: "arrow.left.and.right.righttriangle.left.righttriangle.right") }
                    .help("Flip horizontally")
                Button {
                    var n = g; n.flipV.toggle(); doc.setGeometry(clip, n)
                } label: { Image(systemName: "arrow.up.and.down.righttriangle.up.righttriangle.down") }
                    .help("Flip vertically")
                Spacer()
            }
            .buttonStyle(.bordered).controlSize(.small)

            Text("Reframe").font(.system(size: 10)).foregroundColor(.secondary)
            Picker("", selection: Binding(
                get: { Float(0) },
                set: { doc.reframe(clip, aspect: $0) })) {
                ForEach(EditorDocument.reframeTargets, id: \.name) {
                    Text($0.name).tag($0.aspect)
                }
            }
            .labelsHidden().controlSize(.small)

            crop("Left", g.cropX, 0...0.9) { var n = g; n.cropX = $0; doc.setGeometry(clip, n) }
            crop("Top", g.cropY, 0...0.9) { var n = g; n.cropY = $0; doc.setGeometry(clip, n) }
            crop("Width", g.cropW, 0.05...1) { var n = g; n.cropW = $0; doc.setGeometry(clip, n) }
            crop("Height", g.cropH, 0.05...1) { var n = g; n.cropH = $0; doc.setGeometry(clip, n) }
        }
    }

    private func crop(_ label: String, _ value: Float, _ range: ClosedRange<Double>,
                      _ set: @escaping (Float) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack {
                Text(label).font(.system(size: 10)).foregroundColor(.secondary)
                Spacer()
                Text(String(format: "%.0f%%", value * 100))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
            }
            Slider(value: Binding(get: { Double(value) }, set: { set(Float($0)) }),
                   in: range)
                .controlSize(.mini)
        }
    }
}

/// LUTs — PRODUCT_DIRECTION.md §7's headline colour feature.
///
/// Above the sliders, deliberately. "One good LUT beats twenty minutes of
/// slider-nudging, and creators already own LUTs": the order of the inspector
/// is an argument about how to grade, and putting the look first makes it.
///
/// The amount slider is what makes a LUT usable rather than a switch. Most
/// looks are too strong at 100 %, and being able to sit at 60 % is the
/// difference between a grade and a filter.
struct LookControls: View {
    @ObservedObject var doc: EditorDocument
    let clip: UInt64
    @State private var targeted = false

    var body: some View {
        let spec = doc.lut(for: clip)
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Look").font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                Spacer()
                if spec != nil {
                    Button("Remove") { doc.removeLut(clip) }.font(.system(size: 10))
                }
            }

            if let spec {
                Text((spec.path as NSString).lastPathComponent)
                    .font(.system(size: 10)).lineLimit(1).truncationMode(.middle)
                VStack(alignment: .leading, spacing: 1) {
                    HStack {
                        Text("Amount").font(.system(size: 10)).foregroundColor(.secondary)
                        Spacer()
                        Text(String(format: "%.0f%%", spec.amount * 100))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                    }
                    Slider(value: Binding(get: { Double(spec.amount) },
                                          set: { doc.setLutAmount(clip, Float($0)) }),
                           in: 0...1)
                        .controlSize(.mini)
                }
            } else {
                Button("Choose LUT…") { doc.chooseLut(clip) }.font(.system(size: 11))
                Text("Drop a .cube file here")
                    .font(.system(size: 10)).foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 2)
        .background(targeted ? Color.white.opacity(0.06) : .clear)
        .onDrop(of: ["public.file-url"], isTargeted: $targeted) { providers in
            guard let p = providers.first(where: {
                $0.hasItemConformingToTypeIdentifier("public.file-url") }) else { return false }
            p.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                var url: URL?
                if let d = item as? Data { url = URL(dataRepresentation: d, relativeTo: nil) }
                else if let u = item as? URL { url = u }
                guard let url, url.pathExtension.lowercased() == "cube" else { return }
                Task { @MainActor in doc.applyLut(clip, path: url.path) }
            }
            return true
        }
    }
}

struct EffectControls: View {
    @ObservedObject var doc: EditorDocument
    let clip: UInt64

    var body: some View {
        let e = doc.effects(for: clip)
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Effects").font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                Spacer()
                Button("Reset") { doc.resetEffects(clip) }.font(.system(size: 10))
            }
            slider("Brightness", e.brightness) { var n = e; n.brightness = $0; doc.setEffects(clip, n) }
            slider("Contrast",   e.contrast)   { var n = e; n.contrast   = $0; doc.setEffects(clip, n) }
            slider("Saturation", e.saturation) { var n = e; n.saturation = $0; doc.setEffects(clip, n) }
            slider("Temperature", e.temperature) { var n = e; n.temperature = $0; doc.setEffects(clip, n) }
            slider("Tint",       e.tint)       { var n = e; n.tint       = $0; doc.setEffects(clip, n) }
            unipolar("Blur",    e.blur)    { var n = e; n.blur    = $0; doc.setEffects(clip, n) }
            unipolar("Sharpen", e.sharpen) { var n = e; n.sharpen = $0; doc.setEffects(clip, n) }
            Toggle("Grayscale", isOn: Binding(
                get: { e.grayscale },
                set: { var n = e; n.grayscale = $0; doc.setEffects(clip, n) }))
                .font(.system(size: 11)).toggleStyle(.checkbox)
        }
    }

    /// Blur and sharpen are 0..1, not -1..1 — a negative blur is meaningless.
    private func unipolar(_ label: String, _ value: Float,
                          _ set: @escaping (Float) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack {
                Text(label).font(.system(size: 10)).foregroundColor(.secondary)
                Spacer()
                Text(String(format: "%.2f", value))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
            }
            Slider(value: Binding(get: { Double(value) }, set: { set(Float($0)) }), in: 0...1)
                .controlSize(.mini)
        }
    }

    private func slider(_ label: String, _ value: Float,
                        _ set: @escaping (Float) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack {
                Text(label).font(.system(size: 10)).foregroundColor(.secondary)
                Spacer()
                Text(String(format: "%+.2f", value))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
            }
            Slider(value: Binding(get: { Double(value) }, set: { set(Float($0)) }),
                   in: -1...1)
                .controlSize(.mini)
        }
    }
}


/// §21-style progress with a working Cancel — an export you cannot stop is a
/// worse experience than a slow one.
struct ExportOverlay: View {
    @ObservedObject var doc: EditorDocument
    var body: some View {
        if let p = doc.exportProgress {
            VStack(spacing: 10) {
                Text("Exporting").font(.system(size: 13, weight: .semibold))
                ProgressView(value: p).frame(width: 260)
                Text(doc.exportMessage)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.secondary)
                Button("Cancel") { doc.cancelExport() }
            }
            .padding(20)
            .background(Color(white: 0.16))
            .cornerRadius(10)
            .shadow(radius: 20)
        }
    }
}
