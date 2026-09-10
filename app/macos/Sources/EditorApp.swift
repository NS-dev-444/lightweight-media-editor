import SwiftUI
import AppKit

/// §37: a genuinely native macOS app — real menus, real shortcuts, real
/// window behaviour. Not a web app in a window (§47).
@main
struct EditorApp: App {
    /// AppKit needs a delegate to answer open-file requests — see
    /// `AppDelegate` for why launch itself depends on it.
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var doc = EditorDocument()!
    @StateObject private var queue = ConversionQueue()

    var body: some Scene {
        Window("Editor", id: "main") {
            ContentView(doc: doc, queue: queue)
                .task {
                    // --queue <format-id> --import <file> … populates and runs
                    // the queue, so batch conversion is testable end to end.
                    // The files are named with --import for the reason
                    // LaunchArguments documents.
                    let args = CommandLine.arguments
                    guard let i = args.firstIndex(of: "--queue"), i + 1 < args.count else { return }
                    if let f = ConversionQueue.OutputFormat.all.first(where: { $0.id == args[i + 1] }) {
                        queue.format = f
                    }
                    let files = LaunchArguments.allPaths
                    guard !files.isEmpty else { return }
                    let out = args.firstIndex(of: "--out")
                        .flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil } ?? "/tmp/convout"
                    queue.outputDirectory = URL(fileURLWithPath: out)
                    try? FileManager.default.createDirectory(
                        at: URL(fileURLWithPath: out), withIntermediateDirectories: true)
                    queue.overwrite = true
                    queue.add(paths: files)
                    queue.start()
                }
                .frame(minWidth: 900, minHeight: 560)
                // §29 keyboard shortcuts. Attached here so they work wherever
                // focus sits, which is what users expect of an editor.
                .background(ShortcutCatcher(doc: doc))
        }
        .commands { EditorCommands(doc: doc) }

        // A separate window rather than a sheet: licence texts are long, and
        // people reading them want to keep them open beside their work.
        Window("Acknowledgements", id: "acknowledgements") {
            AcknowledgementsView()
        }
        .defaultSize(width: 720, height: 560)
    }
}

/// §29's shortcut set, using platform-native conventions.
struct EditorCommands: Commands {
    @ObservedObject var doc: EditorDocument
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        // Under the app menu, next to About, which is where macOS users look.
        CommandGroup(after: .appInfo) {
            Button("Acknowledgements…") { openWindow(id: "acknowledgements") }
        }
        CommandGroup(replacing: .newItem) {
            Button("Open…") { doc.open() }.keyboardShortcut("o")
            Button("Import Media…") { doc.importMedia() }.keyboardShortcut("i")
        }
        CommandGroup(replacing: .saveItem) {
            Button("Save") { doc.save() }.keyboardShortcut("s")
            Button("Save As…") { doc.saveAs() }.keyboardShortcut("s", modifiers: [.command, .shift])
        }
        CommandGroup(replacing: .undoRedo) {
            Button("Undo") { doc.undo() }
                .keyboardShortcut("z").disabled(!doc.canUndo)
            // Cmd+Shift+Z on macOS, per §29 and platform convention.
            Button("Redo") { doc.redo() }
                .keyboardShortcut("z", modifiers: [.command, .shift]).disabled(!doc.canRedo)
        }
        CommandMenu("Export") {
            // §12: presets first; codec and bitrate stay out of the way.
            ForEach(doc.exportPresets) { preset in
                Button(preset.name) { doc.export(preset: preset) }
                    .help(preset.detail)
            }
        }
        CommandMenu("Captions") {
            Button("Transcribe Clip") { doc.transcribe() }
                .keyboardShortcut("t", modifiers: [.command, .option])
            Button("Import Captions…") { doc.importSubtitles() }
            Button("Export Captions…") { doc.exportSubtitles() }
                .disabled(doc.captions.entries.isEmpty)
            Divider()
            Button(doc.captions.burnIn ? "Do Not Burn In" : "Burn Into Picture") {
                doc.setCaptionBurnIn(!doc.captions.burnIn)
            }
            .disabled(doc.captions.entries.isEmpty)
        }
        CommandMenu("Record") {
            // §3's voiceover. One shortcut, because starting and stopping a
            // take is one decision made twice.
            Button(doc.recorder.state.isRecording ? "Stop Recording" : "Record Voiceover") {
                if doc.recorder.state.isRecording { doc.stopVoiceover() }
                else { doc.startVoiceover() }
            }
            .keyboardShortcut("r")
        }
        CommandMenu("Playback") {
            Button(doc.isPlaying ? "Pause" : "Play") { doc.togglePlay() }
                .keyboardShortcut(.space, modifiers: [])
        }
        CommandMenu("Title") {
            // Presets are finished looks, so "Add Title" alone gives something
            // that already looks right (PRODUCT_DIRECTION §7).
            Button("Add Title") { doc.addText() }.keyboardShortcut("t")
            Button("Add Image…") { doc.addImageOverlay() }
                .keyboardShortcut("t", modifiers: [.command, .shift])
            ForEach(Array(doc.textPresets.enumerated()), id: \.offset) { i, name in
                Button("Add \(name) Title") { doc.addText(preset: i) }
            }
        }
        CommandMenu("Timeline") {
            // Reframing every clip at once is the version of this people
            // actually want: "make the whole thing vertical", not clip by clip.
            Menu("Reframe All Clips") {
                ForEach(EditorDocument.reframeTargets, id: \.name) { target in
                    Button(target.name) { doc.reframeAll(aspect: target.aspect) }
                }
            }
            Button("Rotate Left") { doc.rotateSelection(-1) }
            Button("Rotate Right") { doc.rotateSelection(1) }
            Divider()
            Button("Split at Playhead") { doc.splitAtPlayhead() }.keyboardShortcut("s", modifiers: [])
            Button("Delete Selected") { doc.deleteSelection() }
                .keyboardShortcut(.delete, modifiers: [])
            Button("Ripple Delete") { doc.rippleDeleteSelection() }
                .keyboardShortcut(.delete, modifiers: [.shift])
            Divider()
            Button("Zoom In") { doc.zoom(1.4) }.keyboardShortcut("=")
            Button("Zoom Out") { doc.zoom(1/1.4) }.keyboardShortcut("-")
            Divider()
            Button("Go to Start") { doc.goToStart() }.keyboardShortcut(.home, modifiers: [])
            Button("Go to End") { doc.goToEnd() }.keyboardShortcut(.end, modifiers: [])
        }
    }
}

/// Arrow-key frame stepping and Space.
///
/// SwiftUI's `keyboardShortcut` does not cover bare arrows well, so a small
/// AppKit view takes the key events — the AppKit-where-SwiftUI-falls-short
/// pattern from AD-2, used sparingly.
struct ShortcutCatcher: NSViewRepresentable {
    let doc: EditorDocument
    func makeNSView(context: Context) -> NSView {
        let v = KeyView()
        v.doc = doc
        DispatchQueue.main.async { v.window?.makeFirstResponder(v) }
        return v
    }
    func updateNSView(_ v: NSView, context: Context) {}
}

final class KeyView: NSView {
    var doc: EditorDocument?
    override var acceptsFirstResponder: Bool { true }
    override func keyDown(with event: NSEvent) {
        guard let doc else { return super.keyDown(with: event) }
        // Step by 1 frame, or 10 with Shift — the standard NLE behaviour.
        let stride: Int64 = event.modifierFlags.contains(.shift) ? 10 : 1
        switch event.keyCode {
        case 123: Task { @MainActor in doc.step(-stride) }   // left arrow
        case 124: Task { @MainActor in doc.step(stride) }    // right arrow
        case 49:  Task { @MainActor in doc.togglePlay() }    // space
        default: super.keyDown(with: event)
        }
    }
}
