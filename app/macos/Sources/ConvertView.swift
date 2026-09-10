import SwiftUI

/// §31's CONVERT mode.
///
/// PRODUCT_DIRECTION §4 keeps this as one of three modes and §9 makes it free
/// forever — it is the acquisition channel, and it is what people actually
/// search for. So it has to be genuinely good, not a token feature.
struct ConvertView: View {
    @ObservedObject var queue: ConversionQueue

    var body: some View {
        VStack(spacing: 0) {
            controls
            Divider()
            if queue.jobs.isEmpty { dropHint } else { jobList }
        }
        .background(Color(white: 0.12))
        .onDrop(of: ["public.file-url"], isTargeted: nil) { providers in
            handleDrop(providers)
        }
    }

    private var controls: some View {
        HStack(spacing: 10) {
            Picker("Convert to", selection: $queue.format) {
                ForEach(ConversionQueue.OutputFormat.all) { f in
                    Text(f.name).tag(f)
                }
            }
            .frame(width: 210)

            Button("Add Files…") { chooseFiles() }
            Button(queue.outputDirectory == nil ? "Output: same folder" : "Output: chosen") {
                chooseOutputFolder()
            }
            Toggle("Overwrite", isOn: $queue.overwrite)
                .toggleStyle(.checkbox).font(.system(size: 11))

            Spacer()

            if queue.isRunning {
                // §21 pause is BETWEEN items; the label says so rather than
                // implying the current encode can be frozen.
                Button("Pause after current") { queue.requestPause() }
                Button("Cancel All") { queue.cancelAll() }
            } else {
                Button("Convert") { queue.start() }
                    .disabled(!queue.jobs.contains { $0.state == .waiting })
                    .keyboardShortcut(.return, modifiers: [])
            }
            Button("Clear Finished") { queue.clearFinished() }
                .disabled(!queue.jobs.contains { $0.state.isTerminal })
        }
        .padding(10)
        .background(Color(white: 0.16))
    }

    private var dropHint: some View {
        VStack(spacing: 8) {
            Image(systemName: "arrow.down.doc").font(.system(size: 28))
                .foregroundColor(.secondary)
            Text("Drop files here").font(.system(size: 13)).foregroundColor(.secondary)
            Text("Batch conversion — video and audio")
                .font(.system(size: 11)).foregroundColor(.secondary.opacity(0.7))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var jobList: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(queue.jobs) { job in
                    JobRow(job: job, queue: queue)
                    Divider()
                }
            }
        }
    }

    private func chooseFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        queue.add(paths: panel.urls.map(\.path))
    }

    private func chooseOutputFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.message = "Where should converted files go?"
        guard panel.runModal() == .OK else { return }
        queue.outputDirectory = panel.url
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
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
            if !paths.isEmpty { queue.add(paths: paths.sorted()) }
        }
        return true
    }
}

struct JobRow: View {
    let job: ConversionQueue.Job
    @ObservedObject var queue: ConversionQueue

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(job.inputName).font(.system(size: 12))
                    Image(systemName: "arrow.right")
                        .font(.system(size: 9)).foregroundColor(.secondary)
                    Text(job.outputName).font(.system(size: 12)).foregroundColor(.secondary)
                    // Worth surfacing: a remux is instant and lossless, and
                    // that is a materially different promise from re-encoding.
                    if job.willRemux {
                        Text("instant")
                            .font(.system(size: 9, weight: .medium))
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(Color.green.opacity(0.25))
                            .cornerRadius(3)
                    }
                }
                statusLine
            }
            Spacer()
            actions
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
    }

    @ViewBuilder private var statusLine: some View {
        switch job.state {
        case .running:
            ProgressView(value: job.progress).frame(width: 220)
        case .failed(let message):
            // §23: a sentence the user can act on.
            Text(message).font(.system(size: 10)).foregroundColor(.orange)
        default:
            Text(job.state.label).font(.system(size: 10)).foregroundColor(.secondary)
        }
    }

    @ViewBuilder private var actions: some View {
        switch job.state {
        case .running:
            Button("Cancel") { queue.cancel(job.id) }.font(.system(size: 11))
        case .failed, .cancelled:
            Button("Retry") { queue.retry(job.id) }.font(.system(size: 11))
            Button("Remove") { queue.remove(job.id) }.font(.system(size: 11))
        case .done:
            Button("Show") {
                NSWorkspace.shared.activateFileViewerSelecting(
                    [URL(fileURLWithPath: job.output)])
            }.font(.system(size: 11))
        case .waiting:
            Button("Remove") { queue.remove(job.id) }.font(.system(size: 11))
        }
    }
}
