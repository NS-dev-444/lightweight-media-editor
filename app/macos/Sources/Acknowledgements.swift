import SwiftUI
import AppKit

/// The licences of everything this application is built on.
///
/// **This is a legal obligation, not a courtesy.** MIT requires its copyright
/// and permission notices to accompany the software; LGPL §6 requires the
/// licence text, prominent notice that the libraries are used, and that the
/// user be able to relink with their own build. A menu item nobody clicks still
/// discharges the obligation; its absence does not.
///
/// The text is generated at build time by `tools/gather_licences.py` from the
/// components actually present in the bundle, and the build fails if anything
/// ships without its licence. A hand-kept NOTICE file is wrong the day after
/// somebody adds a dependency.
struct AcknowledgementsView: View {
    @State private var text = "Loading…"

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Acknowledgements").font(.system(size: 13, weight: .semibold))
                Spacer()
                Button("Save a Copy…") { save() }.font(.system(size: 11))
            }
            .padding(10)
            Divider()
            ScrollView {
                Text(text)
                    // Monospaced because licence texts are hard-wrapped at 70-ish
                    // columns and reflowing them in a proportional font makes
                    // them unreadable.
                    .font(.system(size: 11, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
            }
        }
        .frame(minWidth: 620, minHeight: 460)
        .onAppear(perform: load)
    }

    private func load() {
        guard let path = Bundle.main.path(forResource: "Acknowledgements", ofType: "txt"),
              let s = try? String(contentsOfFile: path, encoding: .utf8) else {
            // Says what is wrong rather than showing an empty window: a missing
            // acknowledgements file is a build problem worth reporting, not a
            // blank pane.
            text = "The acknowledgements file is missing from this build.\n\n"
                 + "This is a packaging fault — the licences of the software this "
                 + "application is built on are required to ship with it. Please "
                 + "report it."
            return
        }
        text = s
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Acknowledgements.txt"
        panel.message = "Save the licence notices"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        try? text.write(to: url, atomically: true, encoding: .utf8)
    }
}
