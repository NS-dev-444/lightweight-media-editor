// S8 worker — stands in for the media worker process. Tries to read the file
// it was given and reports whether the sandbox let it.
import Foundation
let args = CommandLine.arguments
if args.count > 2, args[1] == "--fd" {
    let d = FileHandle.standardInput.readData(ofLength: 16)
    print(d.count > 0 ? "read \(d.count) bytes from inherited descriptor" : "descriptor empty")
    exit(d.count > 0 ? 0 : 1)
}
guard args.count > 1 else { print("no path"); exit(2) }
do {
    let fh = try FileHandle(forReadingFrom: URL(fileURLWithPath: args[1]))
    let d = fh.readData(ofLength: 16)
    try? fh.close()
    print("read \(d.count) bytes by path")
    exit(0)
} catch {
    print("DENIED: \(error.localizedDescription)")
    exit(1)
}
