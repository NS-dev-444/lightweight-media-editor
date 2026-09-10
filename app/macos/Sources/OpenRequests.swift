import AppKit
import SwiftUI

/// Files handed to the app by Finder, the Dock, or `open`.
///
/// Two things make a mailbox the right shape here rather than a direct call.
/// Files can arrive **before** the window exists (double-clicking a project
/// launches the app) or **long after** it does (dropping one on the Dock icon),
/// so there is no single moment to handle them in. And the delegate is created
/// by AppKit, not by the SwiftUI scene, so it has no reference to the document.
@MainActor
final class OpenRequests: ObservableObject {
    static let shared = OpenRequests()
    @Published private(set) var urls: [URL] = []

    /// Files named on the command line are imported synchronously at launch, so
    /// that `--export` and friends see a populated timeline. AppKit delivers
    /// those same paths here as well, so the first delivery of each is dropped
    /// rather than importing the file twice.
    private var pendingDuplicates = Set(LaunchArguments.allPaths.map(OpenRequests.canonical))

    func post(_ new: [URL]) {
        for u in new {
            if pendingDuplicates.remove(OpenRequests.canonical(u.path)) != nil { continue }
            urls.append(u)
        }
    }

    /// argv may hold a relative path where AppKit delivers an absolute one,
    /// so both sides are compared in the same form.
    private static func canonical(_ path: String) -> String {
        URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath().path
    }

    /// Take everything waiting. Draining rather than observing-and-clearing
    /// keeps a file from being opened twice if the view body re-runs.
    func drain() -> [URL] {
        guard !urls.isEmpty else { return [] }
        defer { urls = [] }
        return urls
    }
}

/// §37: opening a file from Finder is table stakes for a native app, and
/// without a delegate to receive it the file is simply dropped on the floor.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The modern callback — Finder, the Dock, `open -a`, and Services.
    func application(_ app: NSApplication, open urls: [URL]) {
        Task { @MainActor in OpenRequests.shared.post(urls) }
    }

    /// The legacy callbacks. AppKit picks one of the three depending on how the
    /// request arrived, so all three have to answer.
    func application(_ sender: NSApplication, openFile filename: String) -> Bool {
        Task { @MainActor in OpenRequests.shared.post([URL(fileURLWithPath: filename)]) }
        return true
    }

    func application(_ sender: NSApplication, openFiles filenames: [String]) {
        Task { @MainActor in OpenRequests.shared.post(filenames.map { URL(fileURLWithPath: $0) }) }
        sender.reply(toOpenOrPrint: .success)
    }

    /// A single-window editor with no document browser has nothing left to do
    /// once its window is gone.
    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

/// Launch-argument parsing, in one place.
///
/// ## Why files are named with `--import`, not as bare arguments
///
/// AppKit treats every bare (non-`-flag`) command-line argument as a file to
/// open, exactly as NSUserDefaults' argument domain does — the token after a
/// `-flag` is that flag's value, anything else is a file. Having decided the
/// app was launched to open a document, it then **skips the "open untitled
/// window" step**, and a SwiftUI `Window` scene has no other trigger. The
/// result is an app that launches, runs, and shows nothing at all.
///
/// This was measured rather than guessed, and every plausible escape was tried:
/// answering `openFile`/`openFiles` with success, failure, or cancel; declaring
/// `applicationShouldOpenUntitledFile`; and `.defaultLaunchBehavior(.presented)`
/// on the scene. **None of them restores the window** — the decision is made
/// inside `finishLaunching` before any of those run. It is not something the
/// app can answer its way out of.
///
/// It costs nothing, because it is not the path users take. Finder, the Dock,
/// `open`, and `open -a Editor.app file.mp4` all deliver through an Apple
/// event instead, which does *not* suppress the window — verified working, and
/// handled by `AppDelegate`. Only a direct `exec` of the binary with a bare
/// path is affected, which is a test harness, so the test harness says
/// `--import <path>` and the problem disappears. Repeat the flag for several
/// files: each path is then a flag's value rather than a bare argument.
///
/// The rule is stricter than it first looks, and worth stating exactly: **every
/// argument must be a `-flag` followed by exactly one value.** `--import=<path>`
/// is *not* equivalent and does not work — AppKit reads it as a key with no
/// value, pairs it with whatever flag comes next, and orphans that flag's value
/// into a bare argument, suppressing the window again. Hence one spelling only.
enum LaunchArguments {
    /// Flags whose next argument is a value, not a file to open.
    private static let takesValue: Set<String> = ["--export", "--image", "--mode", "--queue", "--out",
                                                 "--frames", "--into", "--rotate", "--reframe", "--preset",
                                                 "--merge", "--joined", "--guide", "--lut", "--lang"]

    private static let parsed: (imports: [String], bare: [String]) = {
        var imports: [String] = []
        var bare: [String] = []
        var pendingImport = false
        var skip = false
        for a in CommandLine.arguments.dropFirst() {
            if pendingImport { pendingImport = false; imports.append(a); continue }
            if skip { skip = false; continue }
            if a == "--import" { pendingImport = true; continue }
            if takesValue.contains(a) { skip = true; continue }
            if a.hasPrefix("-") { continue }
            bare.append(a)
        }
        return (imports, bare)
    }()

    /// Files named with `--import`, in order.
    static var importFlagPaths: [String] { parsed.imports }

    /// Every path on the command line, however it was written. Used to
    /// suppress AppKit's duplicate delivery of the same files.
    static var allPaths: [String] { parsed.imports + parsed.bare }

    /// The subset to import into the timeline at launch. `--queue` takes over
    /// the named files, so nothing goes to the timeline in that case.
    static var timelinePaths: [String] {
        CommandLine.arguments.contains("--queue") ? [] : allPaths
    }
}
