import Foundation
import AppKit

/// §8's captions: transcribe, correct, burn in or export as a sidecar.
///
/// The transcription itself runs entirely on this machine — see
/// `mediacore-media/src/transcribe.rs` for why whisper.cpp rather than the
/// system recogniser. Nothing here touches the network, and there is no code
/// path by which it could.
@MainActor
final class CaptionsModel: ObservableObject {

    struct Entry: Identifiable, Codable, Equatable {
        var start: Int64
        var end: Int64
        var text: String
        var id: Int64 { start }
        var startSeconds: Double { Double(start) / Double(TICKS_PER_SECOND) }
        var endSeconds: Double { Double(end) / Double(TICKS_PER_SECOND) }
    }

    @Published private(set) var entries: [Entry] = []
    @Published private(set) var isWorking = false
    @Published private(set) var progress: Double = 0
    @Published private(set) var status = ""
    @Published var burnIn = false
    @Published var preset: Int32 = 1        // Subtitle: outlined, made for this

    /// Is on-device transcription compiled into this build?
    var canTranscribe: Bool { mc_transcribe_available() != 0 }

    /// The bundled model, or nil if the app was built without one.
    static var bundledModel: String? {
        Bundle.main.path(forResource: "ggml-base-q5_1", ofType: "bin")
    }

    final class CancelFlag: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var isSet: Bool { lock.lock(); defer { lock.unlock() }; return value }
        func set() { lock.lock(); value = true; lock.unlock() }
    }
    private var flag = CancelFlag()
    func cancel() { flag.set() }
}

extension CaptionsModel {
    /// Pull the current captions across from the core.
    func refresh(from core: Core) {
        entries = core.allCaptions().map {
            Entry(start: $0.start, end: $0.end, text: $0.text)
        }
        burnIn = core.captionBurnIn
        preset = core.captionPreset
    }

    /// Transcribe a media file and replace the captions with the result.
    ///
    /// The whole transcript arrives as **one** edit, so undo takes back the
    /// transcription rather than four hundred separate captions.
    func transcribe(path: String, model: String, language: String,
                    into core: Core, durationSeconds: Double,
                    done: @escaping () -> Void) {
        guard !isWorking else { return }
        guard canTranscribe else {
            status = "This build has no transcription."
            return
        }
        isWorking = true
        flag = CancelFlag()
        progress = 0
        status = "Listening…"

        let flag = self.flag
        let report: @Sendable (Double) -> Void = { [weak self] p in
            Task { @MainActor in self?.progress = p }
        }
        let finish: @Sendable ([Entry], String) -> Void = { [weak self] items, message in
            Task { @MainActor in
                guard let self else { return }
                self.isWorking = false
                self.progress = 1
                self.status = message
                if !items.isEmpty {
                    core.setCaptions(items.map {
                        Core.CaptionEntry(start: $0.start, end: $0.end, text: $0.text)
                    })
                    self.refresh(from: core)
                }
                done()
            }
        }

        Task.detached(priority: .userInitiated) {
            let (items, message) = Self.run(path: path, model: model, language: language,
                                            durationSeconds: durationSeconds,
                                            cancelled: flag, report: report)
            finish(items, message)
        }
    }

    private nonisolated static func run(path: String, model: String, language: String,
                                        durationSeconds: Double,
                                        cancelled: CancelFlag,
                                        report: @Sendable @escaping (Double) -> Void)
        -> ([Entry], String)
    {
        guard let t = mc_transcribe_open(path, model, language, 0) else {
            return ([], "That file could not be transcribed. It may have no sound.")
        }
        defer { _ = mc_transcribe_close(t) }

        while true {
            let rc = mc_transcribe_step(t)
            if rc == 0 { break }
            if rc < 0 {
                let msg = mc_transcribe_error(t).flatMap { String(cString: $0) } ?? ""
                return ([], msg.isEmpty ? "This audio could not be transcribed." : msg)
            }
            if cancelled.isSet {
                // Keep what has been transcribed so far: half a transcript is
                // useful, and throwing it away wastes the time already spent.
                break
            }
            if durationSeconds > 0 {
                report(min(0.99, mc_transcribe_seconds_done(t) / durationSeconds))
            }
        }

        let n = Int(mc_transcribe_count(t))
        var items: [Entry] = []
        items.reserveCapacity(n)
        for i in 0..<n {
            let start = mc_transcribe_start_ns(t, Int32(i))
            let end = mc_transcribe_end_ns(t, Int32(i))
            let text = mc_transcribe_text(t, Int32(i)).map { String(cString: $0) } ?? ""
            guard !text.isEmpty, end > start else { continue }
            let toTicks = { (ns: Int64) in
                Int64(Double(ns) / 1e9 * Double(TICKS_PER_SECOND))
            }
            items.append(Entry(start: toTicks(start), end: toTicks(end), text: text))
        }
        if items.isEmpty { return ([], "No speech was found in this clip.") }
        let stopped = cancelled.isSet ? " (stopped early)" : ""
        return (items, "Wrote \(items.count) caption\(items.count == 1 ? "" : "s")\(stopped)")
    }
}

import SwiftUI

/// §8's caption panel.
///
/// A **list of editable lines**, not markers on the timeline. Correcting a
/// transcript is a text-editing job — you read down it, fix the three words the
/// model got wrong, and move on — and doing that by clicking tiny blocks on a
/// timeline would be miserable. The timing controls are there for the one thing
/// that is not text: a transcript that is uniformly early or late.
struct CaptionsPane: View {
    @ObservedObject var doc: EditorDocument
    @ObservedObject var captions: CaptionsModel
    @State private var language = "auto"

    /// The languages worth listing. "Auto" first, because it is right almost
    /// always and choosing wrongly is worse than not choosing.
    private static let languages: [(String, String)] = [
        ("auto", "Detect"), ("en", "English"), ("es", "Spanish"), ("fr", "French"),
        ("de", "German"), ("it", "Italian"), ("pt", "Portuguese"), ("nl", "Dutch"),
        ("ru", "Russian"), ("ar", "Arabic"), ("hi", "Hindi"), ("zh", "Chinese"),
        ("ja", "Japanese"), ("ko", "Korean"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Captions").font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                Spacer()
                if !captions.entries.isEmpty {
                    Text("\(captions.entries.count)")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)
                }
            }

            if captions.isWorking {
                ProgressView(value: captions.progress).controlSize(.small)
                HStack {
                    Text(captions.status).font(.system(size: 10)).foregroundColor(.secondary)
                    Spacer()
                    Button("Stop") { captions.cancel() }.font(.system(size: 11))
                }
            } else {
                HStack(spacing: 6) {
                    Picker("", selection: $language) {
                        ForEach(Self.languages, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .labelsHidden().controlSize(.small).frame(width: 92)
                    Button("Transcribe") { doc.transcribe(language: language) }
                        .font(.system(size: 11))
                        .disabled(!captions.canTranscribe)
                }
                if !captions.canTranscribe {
                    Text("This build has no transcription.")
                        .font(.system(size: 10)).foregroundColor(.secondary)
                }
                HStack(spacing: 6) {
                    Button("Import…") { doc.importSubtitles() }.font(.system(size: 11))
                    Button("Export…") { doc.exportSubtitles() }
                        .font(.system(size: 11))
                        .disabled(captions.entries.isEmpty)
                }
            }

            if !captions.entries.isEmpty {
                Divider()
                HStack(spacing: 6) {
                    Toggle("Burn in", isOn: Binding(
                        get: { captions.burnIn },
                        set: { doc.setCaptionBurnIn($0) }))
                        .toggleStyle(.checkbox).font(.system(size: 11))
                    if captions.burnIn {
                        Picker("", selection: Binding(
                            get: { captions.preset },
                            set: { doc.setCaptionPreset($0) })) {
                            ForEach(Array(doc.textPresets.enumerated()), id: \.offset) { i, name in
                                Text(name).tag(Int32(i))
                            }
                        }
                        .labelsHidden().controlSize(.small).frame(width: 90)
                    }
                }
                if captions.burnIn {
                    Text("Burned-in captions cannot be switched off by the viewer.")
                        .font(.system(size: 9)).foregroundColor(.secondary)
                }

                HStack(spacing: 4) {
                    Text("Nudge").font(.system(size: 10)).foregroundColor(.secondary)
                    Button("−0.5s") { doc.shiftCaptions(seconds: -0.5) }
                    Button("+0.5s") { doc.shiftCaptions(seconds: 0.5) }
                    Spacer()
                }
                .buttonStyle(.bordered).controlSize(.mini)

                Divider()
                ScrollView {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(captions.entries.enumerated()), id: \.element.id) { i, e in
                            CaptionRow(doc: doc, index: i, entry: e)
                        }
                    }
                }
            } else if !captions.isWorking {
                Text("Transcribe the clip under the playhead, or import a transcript you already have.")
                    .font(.system(size: 10)).foregroundColor(.secondary)
            }

            if !captions.status.isEmpty && !captions.isWorking {
                Text(captions.status).font(.system(size: 10)).foregroundColor(.secondary)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(white: 0.14))
        .onAppear { doc.refreshCaptions() }
    }
}

/// One editable line. Clicking its timestamp moves the playhead there, which is
/// how you check a correction against what was actually said.
struct CaptionRow: View {
    @ObservedObject var doc: EditorDocument
    let index: Int
    let entry: CaptionsModel.Entry
    @State private var draft: String = ""
    @State private var editing = false

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Button(String(format: "%d:%05.2f", Int(entry.startSeconds) / 60,
                          entry.startSeconds.truncatingRemainder(dividingBy: 60))) {
                doc.scrub(to: entry.start)
            }
            .buttonStyle(.plain)
            .font(.system(size: 9, design: .monospaced))
            .foregroundColor(.accentColor)
            .frame(width: 48, alignment: .leading)

            if editing {
                TextField("", text: $draft, onCommit: {
                    doc.setCaptionText(index, draft)
                    editing = false
                })
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11))
            } else {
                Text(entry.text)
                    .font(.system(size: 11))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                    .onTapGesture { draft = entry.text; editing = true }
            }

            Button { doc.removeCaption(index) } label: { Image(systemName: "xmark") }
                .buttonStyle(.borderless).controlSize(.mini)
        }
        .padding(.vertical, 1)
    }
}
