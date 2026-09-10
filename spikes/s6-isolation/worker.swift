// S6 WORKER PROCESS.
//
// This is the only process that links FFmpeg. A codec bug parsing hostile input
// can crash *this*, and that is the design (AD-3, R-15): a worker crash becomes
// a classified job failure instead of losing the user's project.
import Foundation

guard CommandLine.arguments.count > 1 else { exit(64) }
var probe = MCProbe()
let cls = mc_probe(CommandLine.arguments[1], 30, &probe)
let msg = String(cString: mc_class_message(cls))
print("\(cls)\t\(probe.width)x\(probe.height)\t\(probe.frames_decoded)\t\(probe.av_error)\t\(msg)")
exit(0)
