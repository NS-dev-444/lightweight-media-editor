import SwiftUI

/// §31's TOOLS mode.
///
/// PRODUCT_DIRECTION §4: "presets over the converter engine, NOT separate
/// code." Each tool here is a one-click configuration of the same queue that
/// CONVERT uses — which is why they inherit its progress, cancel, retry and
/// overwrite protection for free, and why there is no second implementation to
/// keep in step.
struct ToolsView: View {
    @ObservedObject var queue: ConversionQueue
    @StateObject private var frames = FrameExtractor()
    @StateObject private var merger = Merger()

    struct Tool: Identifiable {
        let id: String
        let name: String
        let detail: String
        let icon: String
        let format: ConversionQueue.OutputFormat
        /// Set for the tools that must re-encode. Absent keeps the lossless
        /// copy path available.
        var video: ConversionQueue.VideoSettings? = nil
        /// Appended to the file name, so a resized copy sits beside the
        /// original instead of replacing it.
        var suffix: String = ""
    }

    private var tools: [Tool] {
        let f = { (id: String) in
            ConversionQueue.OutputFormat.all.first { $0.id == id }!
        }
        return [
            Tool(id: "extract-mp3", name: "Extract Audio as MP3",
                 detail: "Pull the soundtrack out of a video.",
                 icon: "waveform", format: f("mp3")),
            Tool(id: "extract-m4a", name: "Extract Audio as M4A",
                 detail: "Better quality than MP3 at the same size.",
                 icon: "waveform.circle", format: f("m4a")),
            Tool(id: "extract-wav", name: "Extract Audio as WAV",
                 detail: "Uncompressed, for further editing.",
                 icon: "waveform.path", format: f("wav")),
            Tool(id: "to-mp4", name: "Change Format to MP4",
                 detail: "Instant when the video is already compatible — no quality loss.",
                 icon: "film", format: f("mp4")),
            Tool(id: "to-mov", name: "Change Format to MOV",
                 detail: "For editing on Apple tools.",
                 icon: "film.stack", format: f("mov")),
            Tool(id: "to-flac", name: "Convert Audio to FLAC",
                 detail: "Lossless compression for archiving.",
                 icon: "square.stack.3d.up", format: f("flac")),
            Tool(id: "resize-1080", name: "Resize to 1080p",
                 detail: "Scales down to 1920 across. The shape is kept.",
                 icon: "rectangle.compress.vertical", format: f("mp4"),
                 video: .init(height: 1080), suffix: "-1080p"),
            Tool(id: "resize-720", name: "Resize to 720p",
                 detail: "Smaller again — good for email and messaging.",
                 icon: "rectangle.compress.vertical", format: f("mp4"),
                 video: .init(height: 720), suffix: "-720p"),
            Tool(id: "compress", name: "Compress for Sharing",
                 detail: "1080p at a modest bitrate. Much smaller, still looks right.",
                 icon: "arrow.down.circle", format: f("mp4"),
                 video: .init(height: 1080, bitrateKbps: 4000), suffix: "-small"),
        ]
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Tools")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.secondary)
                Text("Pick a tool, then drop files on it. Everything runs through the same queue as Convert.")
                    .font(.system(size: 11)).foregroundColor(.secondary)

                LazyVGrid(columns: [GridItem(.adaptive(minimum: 250), spacing: 10)],
                          spacing: 10) {
                    ForEach(tools) { tool in
                        ToolCard(tool: tool, queue: queue)
                    }
                    ExtractFramesCard(frames: frames)
                    MergeCard(merger: merger)
                }
            }
            .padding(14)
        }
        .background(Color(white: 0.12))
        .onAppear {
            // --mode tools --frames <video> --into <folder> runs the extractor
            // headlessly, so this path is testable without driving a file
            // panel — the same arrangement --export and --queue already use.
            let args = CommandLine.arguments
            guard let i = args.firstIndex(of: "--frames"), i + 1 < args.count else { return }
            let into = args.firstIndex(of: "--into")
                .flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil } ?? "/tmp/mcframes"
            frames.run(input: args[i + 1], directory: URL(fileURLWithPath: into),
                       interval: 1.0, format: .png)
        }
        .onAppear {
            // --mode tools --merge <a> --merge <b> --joined <out> exercises the
            // merge path headlessly, like --frames does for extraction.
            let args = CommandLine.arguments
            var files: [URL] = []
            for (i, a) in args.enumerated() where a == "--merge" && i + 1 < args.count {
                files.append(URL(fileURLWithPath: args[i + 1]))
            }
            guard !files.isEmpty,
                  let j = args.firstIndex(of: "--joined"), j + 1 < args.count else { return }
            merger.add(files)
            merger.run(to: URL(fileURLWithPath: args[j + 1]))
        }
    }
}

struct ToolCard: View {
    let tool: ToolsView.Tool
    @ObservedObject var queue: ConversionQueue
    @State private var targeted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: tool.icon).font(.system(size: 15))
                Text(tool.name).font(.system(size: 12, weight: .medium))
            }
            Text(tool.detail).font(.system(size: 10)).foregroundColor(.secondary)
            Button("Choose Files…") { choose() }.font(.system(size: 11))
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(white: targeted ? 0.22 : 0.16))
        .cornerRadius(8)
        .onDrop(of: ["public.file-url"], isTargeted: $targeted) { providers in
            var paths: [String] = []
            let group = DispatchGroup()
            for p in providers where p.hasItemConformingToTypeIdentifier("public.file-url") {
                group.enter()
                p.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                    defer { group.leave() }
                    if let d = item as? Data, let u = URL(dataRepresentation: d, relativeTo: nil) {
                        paths.append(u.path)
                    } else if let u = item as? URL { paths.append(u.path) }
                }
            }
            group.notify(queue: .main) {
                guard !paths.isEmpty else { return }
                queue.format = tool.format
                queue.add(paths: paths.sorted(), video: tool.video, suffix: tool.suffix)
                queue.start()
            }
            return true
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        queue.format = tool.format
        queue.add(paths: panel.urls.map(\.path), video: tool.video, suffix: tool.suffix)
        queue.start()
    }
}


/// Extract Frames sits apart from the other cards because it is the one tool
/// that turns a file into a folder rather than into another file — see
/// `FrameExtractor` for why it does not run through the conversion queue.
struct ExtractFramesCard: View {
    @ObservedObject var frames: FrameExtractor
    @State private var everySeconds = 1.0
    @State private var format: FrameExtractor.Format = .png
    @State private var targeted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: "photo.on.rectangle").font(.system(size: 15))
                Text("Extract Frames").font(.system(size: 12, weight: .medium))
            }
            Text("Save stills from a video as image files.")
                .font(.system(size: 10)).foregroundColor(.secondary)

            HStack(spacing: 6) {
                Text("Every").font(.system(size: 10)).foregroundColor(.secondary)
                // A step, not a free number: "one frame every N seconds" is the
                // question people actually have, and every-frame on a long clip
                // is tens of thousands of files.
                Picker("", selection: $everySeconds) {
                    Text("0.5s").tag(0.5)
                    Text("1s").tag(1.0)
                    Text("5s").tag(5.0)
                    Text("10s").tag(10.0)
                }
                .labelsHidden().frame(width: 76)
                Picker("", selection: $format) {
                    ForEach(FrameExtractor.Format.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden().frame(width: 78)
            }

            if frames.isRunning {
                ProgressView(value: frames.progress).controlSize(.small)
                HStack {
                    Text("\(frames.written) saved").font(.system(size: 10))
                        .foregroundColor(.secondary)
                    Spacer()
                    Button("Stop") { frames.cancel() }.font(.system(size: 11))
                }
            } else {
                Button("Choose Video…") { choose() }.font(.system(size: 11))
                if !frames.status.isEmpty {
                    Text(frames.status).font(.system(size: 10)).foregroundColor(.secondary)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(white: targeted ? 0.22 : 0.16))
        .cornerRadius(8)
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Choose a video to take frames from"
        guard panel.runModal() == .OK, let url = panel.url else { return }

        // The frames land in a folder named after the video, beside it — so a
        // few hundred images never scatter into the user's Movies folder.
        let dir = url.deletingLastPathComponent()
            .appendingPathComponent(url.deletingPathExtension().lastPathComponent + " frames")
        frames.run(input: url.path, directory: dir,
                   interval: everySeconds, format: format)
    }
}


/// §31's merge tool. Many files in, one out — see `Merger` for why it sits
/// outside the conversion queue.
struct MergeCard: View {
    @ObservedObject var merger: Merger
    @State private var targeted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: "rectangle.stack").font(.system(size: 15))
                Text("Merge Videos").font(.system(size: 12, weight: .medium))
            }
            Text(merger.inputs.count > 1 && merger.wouldCopy
                 ? "These join instantly, with no quality loss."
                 : "Join clips end to end, in the order below.")
                .font(.system(size: 10)).foregroundColor(.secondary)

            if merger.inputs.isEmpty {
                Text("No files chosen").font(.system(size: 10)).foregroundColor(.secondary)
            } else {
                // The order is the whole point, so it is shown and editable.
                ForEach(Array(merger.inputs.enumerated()), id: \.offset) { i, url in
                    HStack(spacing: 4) {
                        Text("\(i + 1).").font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                        Text(url.lastPathComponent).font(.system(size: 10)).lineLimit(1)
                        Spacer()
                        Button { merger.move(from: i, to: i - 1) } label: {
                            Image(systemName: "chevron.up")
                        }.disabled(i == 0)
                        Button { merger.move(from: i, to: i + 1) } label: {
                            Image(systemName: "chevron.down")
                        }.disabled(i == merger.inputs.count - 1)
                        Button { merger.remove(at: i) } label: {
                            Image(systemName: "xmark")
                        }
                    }
                    .buttonStyle(.borderless).controlSize(.mini)
                }
            }

            if merger.isRunning {
                ProgressView(value: merger.progress).controlSize(.small)
                Button("Stop") { merger.cancel() }.font(.system(size: 11))
            } else {
                HStack(spacing: 6) {
                    Button("Add Files…") { choose() }.font(.system(size: 11))
                    Button("Join…") { save() }
                        .font(.system(size: 11))
                        .disabled(merger.inputs.isEmpty)
                    if !merger.inputs.isEmpty {
                        Button("Clear") { merger.clear() }.font(.system(size: 11))
                    }
                }
                if !merger.status.isEmpty {
                    Text(merger.status).font(.system(size: 10)).foregroundColor(.secondary)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(white: targeted ? 0.22 : 0.16))
        .cornerRadius(8)
        .onDrop(of: ["public.file-url"], isTargeted: $targeted) { providers in
            var urls: [URL] = []
            let group = DispatchGroup()
            for p in providers where p.hasItemConformingToTypeIdentifier("public.file-url") {
                group.enter()
                p.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                    defer { group.leave() }
                    if let d = item as? Data, let u = URL(dataRepresentation: d, relativeTo: nil) {
                        urls.append(u)
                    } else if let u = item as? URL { urls.append(u) }
                }
            }
            group.notify(queue: .main) {
                // Dropped files have no inherent order, so they are sorted by
                // name — which is what "part1, part2, part3" needs.
                merger.add(urls.sorted { $0.lastPathComponent < $1.lastPathComponent })
            }
            return true
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.message = "Choose videos to join"
        guard panel.runModal() == .OK else { return }
        merger.add(panel.urls.sorted { $0.lastPathComponent < $1.lastPathComponent })
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Joined.mp4"
        panel.message = "Where should the joined video go?"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        merger.run(to: url)
    }
}
