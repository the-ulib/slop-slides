// Bundled macOS renderer/encoder. No HTTP server, Python or FFmpeg at runtime.
import AppKit
import WebKit
import AVFoundation
import CoreVideo

struct Slide: Decodable { let id: String; let startSample: Int64; let endSample: Int64; let startFrame: Int64; let endFrame: Int64 }
struct Job: Decodable { let slides: [Slide]; let totalSamples: Int64; let totalFrames: Int64 }
func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw NSError(domain: "SlopSlideVideo", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
}
func progress(_ stage: String, _ completed: Int64, _ total: Int64) {
    let data = try! JSONSerialization.data(withJSONObject: ["stage": stage, "completed": completed, "total": total])
    FileHandle.standardOutput.write(data + Data([10]))
}

// Freeze ordinary HTML/CSS resource references before WebKit loads. Remote stylesheets
// are rewritten recursively, including relative font/image URLs. CSP then denies network.
final class Resources {
    var cache: [URL: String] = [:]
    var bytes = 0
    func matches(_ pattern: String, _ text: String) throws -> [NSTextCheckingResult] {
        try NSRegularExpression(pattern: pattern, options: [.caseInsensitive]).matches(in: text, range: NSRange(text.startIndex..., in: text))
    }
    func rewrite(_ text: String, base: URL?, css: Bool, depth: Int = 0) async throws -> String {
        try require(depth < 8, "Stylesheet imports are too deeply nested or cyclic.")
        let pattern = css ? #"(?:url\(\s*["']?)([^\s"')]+)|(?:@import\s+["'])([^"']+)"# : #"(?:\bsrc\s*=\s*["'])([^"']+)|(?:<(?:link|image|use)\b[^>]*?\bhref\s*=\s*["'])([^"']+)|(?:url\(\s*["']?)([^\s"')]+)|(?:@import\s+["'])([^"']+)"#
        var result = text
        for match in try matches(pattern, text).reversed() {
            let capture = (1..<match.numberOfRanges).first { match.range(at: $0).location != NSNotFound }!
            guard let range = Range(match.range(at: capture), in: text) else { continue }
            let raw = String(text[range]).replacingOccurrences(of: "&amp;", with: "&")
            if raw.hasPrefix("data:") || raw.hasPrefix("#") { continue }
            let resolved = URL(string: raw.hasPrefix("//") ? "https:" + raw : raw, relativeTo: base)?.absoluteURL
            guard let url = resolved, ["https", "http"].contains(url.scheme?.lowercased() ?? "") else { continue }
            let replacement = try await download(url, depth: depth + 1)
            if let replaceRange = Range(match.range(at: capture), in: result) { result.replaceSubrange(replaceRange, with: replacement) }
        }
        return result
    }
    func download(_ url: URL, depth: Int) async throws -> String {
        if let hit = cache[url] { return hit }
        try require(cache.count < 256 && bytes < 100_000_000, "Too many or too large external resources.")
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 30
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel() }
        let (data, response) = try await session.data(from: url)
        try require((response as? HTTPURLResponse)?.statusCode == 200, "Could not freeze resource: \(url)")
        bytes += data.count
        try require(data.count <= 20_000_000 && bytes <= 100_000_000, "External resource exceeds the export limit: \(url)")
        let mime = response.mimeType ?? "application/octet-stream"
        try require(!mime.contains("javascript"), "External scripts are unsupported for static video export. Inline the script first.")
        var body = data
        if mime == "text/css" {
            guard let css = String(data: data, encoding: .utf8) else { throw NSError(domain: "Invalid CSS", code: 1) }
            body = Data(try await rewrite(css, base: response.url ?? url, css: true, depth: depth).utf8)
        }
        let value = "data:\(mime);base64,\(body.base64EncodedString())"
        cache[url] = value
        return value
    }
}

@MainActor final class Renderer: NSObject, WKNavigationDelegate {
    let view: WKWebView
    let window: NSWindow
    var navigation: CheckedContinuation<Void, Error>?
    override init() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        view = WKWebView(frame: NSRect(x: 0, y: 0, width: 1920, height: 1080), configuration: config)
        window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        super.init()
        window.contentView = view // Never shown; independent of editor geometry/display scale.
        view.navigationDelegate = self
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { self.navigation?.resume(); self.navigation = nil }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { fail(error) }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { fail(error) }
    func fail(_ error: Error) { navigation?.resume(throwing: error); navigation = nil }
    func js(_ source: String, arguments: [String: Any] = [:]) async throws -> Any {
        try await withCheckedThrowingContinuation { continuation in
            view.callAsyncJavaScript(source, arguments: arguments, in: nil, in: .page) { continuation.resume(with: $0) }
        }
    }
    func render(_ root: URL, _ job: Job) async throws {
        let html = try String(contentsOf: root.appendingPathComponent("deck.html"), encoding: .utf8)
        try require(try Resources().matches(#"<script\b[^>]*\bsrc\s*="#, html).isEmpty, "External scripts are unsupported for static video export. Inline the script first.")
        let resources = Resources()
        let assets = root.appendingPathComponent("assets")
        let files = FileManager.default.enumerator(at: assets, includingPropertiesForKeys: nil)?.allObjects.compactMap { $0 as? URL } ?? []
        for file in files where file.pathExtension.lowercased() == "css" {
                let css = try String(contentsOf: file, encoding: .utf8)
                try Data(try await resources.rewrite(css, base: file, css: true).utf8).write(to: file)
            }
        let frozen = try await resources.rewrite(html, base: nil, css: false)
        let policy = #"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: file:; style-src 'unsafe-inline' data: file:; font-src data: file:; script-src 'unsafe-inline' file:; connect-src 'none'; media-src 'none'; frame-src 'none'; base-uri 'none'">"#
        // Inject before any resource loads; file access is limited to the job copy.
        guard let head = try resources.matches(#"<head(?:\s[^>]*)?>"#, frozen).first,
              let headRange = Range(head.range, in: frozen) else { throw NSError(domain: "Video export requires a head element", code: 1) }
        var secured = frozen
        secured.insert(contentsOf: policy, at: headRange.upperBound)
        try Data(secured.utf8).write(to: root.appendingPathComponent("frozen.html"))
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            navigation = continuation
            view.loadFileURL(root.appendingPathComponent("frozen.html"), allowingReadAccessTo: root)
            DispatchQueue.main.asyncAfter(deadline: .now() + 30) {
                self.fail(NSError(domain: "Render timed out loading deck resources", code: 1))
            }
        }
        _ = try await js("""
            const bounded = (p) => Promise.race([p, new Promise((_, reject) => setTimeout(() => reject(new Error('Resource readiness timed out')), 20000))]);
            if (Array.from(document.querySelectorAll('link[rel=stylesheet]')).some(e => !e.sheet)) throw new Error('Missing stylesheet; save it in assets before exporting.');
            await bounded(document.fonts.ready);
            await bounded(Promise.all(Array.from(document.fonts).map(f => f.load())));
            await bounded(Promise.all(Array.from(document.images).map(i => i.decode().catch(() => { throw new Error('Missing image: ' + i.src); }))));
            if (document.querySelector('[srcset]')) throw new Error('Responsive srcset images are unsupported; use a single image source for export.');
            const urls = new Set();
            for (const el of document.querySelectorAll('*')) {
                const s = getComputedStyle(el);
                for (const value of [s.backgroundImage, s.borderImageSource, s.listStyleImage])
                    for (const m of value.matchAll(/url\\(["']?([^"')]+)["']?\\)/g)) urls.add(m[1]);
            }
            await bounded(Promise.all(Array.from(urls).map(src => { const i = new Image(); i.src = src; return i.decode().catch(() => { throw new Error('Missing background image: ' + src); }); })));
            document.documentElement.setAttribute('data-slop-static', '');
            document.querySelectorAll('.slop-review').forEach(e => e.remove());
            return true;
            """)
        for (index, slide) in job.slides.enumerated() {
            _ = try await js("""
                const slide = document.getElementById(id);
                if (!slide || !slide.classList.contains('slide')) throw new Error('Slide missing: ' + id);
                document.querySelectorAll('.deck > .slide').forEach(e => e.classList.toggle('active', e === slide));
                // Freeze finite animations at their final state; no animated capture in v1.
                for (const a of document.getAnimations()) { try { a.finish(); } catch { a.pause(); } }
                slide.getBoundingClientRect();
                // Hidden views may suspend RAF. Readiness above plus a bounded paint turn
                // is the validated fallback; it is not a blind resource-settle delay.
                await Promise.race([new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))), new Promise(r => setTimeout(r, 200))]);
                return {width: innerWidth, height: innerHeight};
                """, arguments: ["id": slide.id])
            let image: NSImage = try await withCheckedThrowingContinuation { continuation in
                let config = WKSnapshotConfiguration()
                config.rect = NSRect(x: 0, y: 0, width: 1920, height: 1080)
                config.snapshotWidth = 1920
                view.takeSnapshot(with: config) { image, error in
                    if let image { continuation.resume(returning: image) }
                    else { continuation.resume(throwing: error ?? NSError(domain: "Snapshot failed", code: 1)) }
                }
            }
            guard let source = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
                  let context = CGContext(data: nil, width: 1920, height: 1080, bitsPerComponent: 8, bytesPerRow: 1920 * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw NSError(domain: "Snapshot allocation failed", code: 1) }
            context.interpolationQuality = .high
            context.draw(source, in: CGRect(x: 0, y: 0, width: 1920, height: 1080))
            let bitmap = NSBitmapImageRep(cgImage: context.makeImage()!)
            guard let data = bitmap.representation(using: .png, properties: [:]) else { throw NSError(domain: "PNG encoding failed", code: 1) }
            try data.write(to: root.appendingPathComponent("frame-\(index).png"))
            progress("rendering", Int64(index + 1), Int64(job.slides.count))
        }
    }
}

func buffer(_ path: URL) throws -> CVPixelBuffer {
    guard let image = NSImage(contentsOf: path)?.cgImage(forProposedRect: nil, context: nil, hints: nil) else { throw NSError(domain: "Missing rendered frame", code: 1) }
    try require(image.width == 1920 && image.height == 1080, "Rendered frame is not 1080p.")
    var result: CVPixelBuffer?
    let attributes = [kCVPixelBufferCGImageCompatibilityKey: true, kCVPixelBufferCGBitmapContextCompatibilityKey: true] as CFDictionary
    try require(CVPixelBufferCreate(kCFAllocatorDefault, 1920, 1080, kCVPixelFormatType_32ARGB, attributes, &result) == kCVReturnSuccess, "Pixel buffer allocation failed.")
    let pixel = result!
    CVPixelBufferLockBaseAddress(pixel, [])
    defer { CVPixelBufferUnlockBaseAddress(pixel, []) }
    guard let ctx = CGContext(data: CVPixelBufferGetBaseAddress(pixel), width: 1920, height: 1080, bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(pixel), space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue) else { throw NSError(domain: "Pixel context failed", code: 1) }
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    return pixel
}

func encode(_ root: URL, _ job: Job, _ destination: URL) async throws {
    let scratch = destination.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".video.mp4")
    defer { try? FileManager.default.removeItem(at: scratch) }
    let writer = try AVAssetWriter(outputURL: scratch, fileType: .mp4)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 1920, AVVideoHeightKey: 1080, AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 4_000_000, AVVideoMaxKeyFrameIntervalKey: 30]])
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 1920, kCVPixelBufferHeightKey as String: 1080])
    writer.add(input)
    try require(writer.startWriting(), "Could not start encoder: \(String(describing: writer.error))")
    writer.startSession(atSourceTime: .zero)
    var slide = 0
    var pixel = try buffer(root.appendingPathComponent("frame-0.png"))
    for frame in 0..<job.totalFrames {
        while slide + 1 < job.slides.count && frame >= job.slides[slide + 1].startFrame {
            slide += 1
            pixel = try buffer(root.appendingPathComponent("frame-\(slide).png"))
        }
        let deadline = Date().addingTimeInterval(30)
        while !input.isReadyForMoreMediaData {
            try require(writer.status == .writing && Date() < deadline, "Encoder stalled: \(String(describing: writer.error))")
            try await Task.sleep(nanoseconds: 1_000_000)
        }
        try require(adaptor.append(pixel, withPresentationTime: CMTime(value: frame, timescale: 30)), "Frame append failed: \(String(describing: writer.error))")
        if frame % 30 == 0 { progress("encoding", frame, job.totalFrames) }
    }
    writer.endSession(atSourceTime: CMTime(value: job.totalFrames, timescale: 30))
    input.markAsFinished()
    await writer.finishWriting()
    try require(writer.status == .completed, "Video encode failed: \(String(describing: writer.error))")
    let video = AVURLAsset(url: scratch)
    let audio = AVURLAsset(url: root.appendingPathComponent("timeline.wav"))
    let vtracks = try await video.loadTracks(withMediaType: .video)
    let atracks = try await audio.loadTracks(withMediaType: .audio)
    try require(!vtracks.isEmpty && !atracks.isEmpty, "Encoded movie is missing video or audio.")
    let composition = AVMutableComposition()
    let v = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    let a = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    let duration = CMTime(value: job.totalFrames, timescale: 30)
    try v.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: vtracks[0], at: .zero)
    try a.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: atracks[0], at: .zero)
    guard let exporter = AVAssetExportSession(asset: composition, presetName: AVAssetExportPreset1920x1080) else { throw NSError(domain: "MP4 export unavailable", code: 1) }
    exporter.outputURL = destination
    exporter.outputFileType = .mp4
    exporter.shouldOptimizeForNetworkUse = true
    progress("finalizing", job.totalFrames, job.totalFrames)
    await withCheckedContinuation { continuation in exporter.exportAsynchronously { continuation.resume() } }
    try require(exporter.status == .completed, "MP4 finalization failed: \(String(describing: exporter.error))")
    let result = AVURLAsset(url: destination)
    let tracks = try await result.loadTracks(withMediaType: .video)
    let resultDuration = try await result.load(.duration)
    let dimensions = try await tracks.first?.load(.naturalSize)
    try require(dimensions == CGSize(width: 1920, height: 1080) && abs(resultDuration.seconds - duration.seconds) <= 1.0 / 30, "Encoded dimensions or duration failed validation.")
    progress("complete", job.totalFrames, job.totalFrames)
}

@main @MainActor struct Main {
    static func main() {
        let args = CommandLine.arguments
        guard args.count >= 3 else { fputs("Usage: slopslide-video render ROOT | encode ROOT OUTPUT\n", stderr); exit(1) }
        NSApplication.shared.setActivationPolicy(.prohibited)
        let parent = getppid()
        Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in
            if getppid() != parent { exit(3) }
        }
        Task {
            do {
                let root = URL(fileURLWithPath: args[2], isDirectory: true)
                let job = try JSONDecoder().decode(Job.self, from: Data(contentsOf: root.appendingPathComponent("job.json")))
                try require(!job.slides.isEmpty && job.totalFrames > 0, "Empty video timeline.")
                if args[1] == "render" { try await Renderer().render(root, job) }
                else if args[1] == "encode", args.count == 4 { try await encode(root, job, URL(fileURLWithPath: args[3])) }
                else { throw NSError(domain: "Unknown video operation", code: 1) }
                exit(0)
            } catch { fputs("\(error.localizedDescription)\n", stderr); exit(2) }
        }
        NSApplication.shared.run()
    }
}
