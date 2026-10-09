// Diagnostic parent for a locally signed App Sandbox bundle, not a Tauri integration.
import Foundation

@main struct Main {
    static func main() throws {
        let args = CommandLine.arguments
        guard args.count == 2, let resources = Bundle.main.resourceURL else {
            fputs("Usage: sandbox-worker KNOWN_OUTSIDE_FIXTURE\n", stderr); exit(1)
        }
        // The caller creates this harmless fixture before launch. Its denial is a
        // negative control; do not claim sandbox enforcement merely from entitlements.
        do {
            _ = try Data(contentsOf: URL(fileURLWithPath: args[1]))
            fputs("FAIL: outside fixture readable; sandbox control failed\n", stderr); exit(2)
        } catch {
            print("Outside fixture read rejected: \(error)")
        }
        let fm = FileManager.default
        let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let output = support.appendingPathComponent("slopslide-probe-\(UUID().uuidString).wav")
        let log = support.appendingPathComponent("slopslide-probe-\(UUID().uuidString).log")
        fm.createFile(atPath: log.path, contents: nil)
        let handle = try FileHandle(forWritingTo: log)
        defer { try? handle.close() }
        let worker = Process()
        worker.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/qwen_tts")
        worker.arguments = ["-d", resources.appendingPathComponent("cv").path, "--text", "Good work needs space.", "-s", "ryan", "-l", "English", "--seed", "42", "--max-duration", "15", "-o", output.path]
        worker.environment = ["QWEN_NO_KLEIDI": "1", "QWEN_DISPATCH_MAP": "1", "QWEN_SHAPE_CENSUS": "1"]
        worker.standardOutput = handle
        worker.standardError = handle
        try worker.run()
        let deadline = Date().addingTimeInterval(90)
        while worker.isRunning && Date() < deadline { Thread.sleep(forTimeInterval: 0.1) }
        if worker.isRunning {
            worker.terminate()
            fputs("Worker timed out\n", stderr); exit(3)
        }
        guard worker.terminationStatus == 0, fm.fileExists(atPath: output.path) else {
            fputs("Worker failed: \(worker.terminationStatus); log \(log.path)\n", stderr); exit(4)
        }
        let data = try Data(contentsOf: output)
        guard data.count > 44, String(data: data.prefix(4), encoding: .ascii) == "RIFF" else { exit(5) }
        print("PASS: bundled worker synthesized local WAV and exited")
        print("WAV: \(output.path)")
        print("LOG: \(log.path)")
    }
}
