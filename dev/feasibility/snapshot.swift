// Standalone feasibility probe. Compile with swiftc; no application changes.
import AppKit
import WebKit

@MainActor final class SnapshotProbe: NSObject, WKNavigationDelegate {
    let view: WKWebView = {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        return WKWebView(frame: NSRect(x: 0, y: 0, width: 1920, height: 1080), configuration: config)
    }()
    var window: NSWindow?
    let output: URL
    init(html: URL, output: URL, attached: Bool) throws {
        self.output = output
        super.init()
        view.navigationDelegate = self
        if attached {
            let w = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            w.contentView = view
            window = w // Deliberately never shown: test export without opening a user window.
        }
        fputs("Loading HTML\n", stderr)
        view.loadHTMLString(try String(contentsOf: html, encoding: .utf8), baseURL: html.deletingLastPathComponent())
        DispatchQueue.main.asyncAfter(deadline: .now() + 25) {
            fputs("Snapshot timed out\n", stderr)
            exit(2)
        }
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        fputs("Navigation failed: \(error)\n", stderr); exit(3)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        fputs("Navigation complete\n", stderr)
        webView.callAsyncJavaScript("""
            await document.fonts.ready;
            await Promise.all(Array.from(document.images).map(i => i.decode()));
            // Hidden WebKit views may suspend animation frames indefinitely.
            await Promise.race([
                new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))),
                new Promise(r => setTimeout(r, 200))
            ]);
            return {width: innerWidth, height: innerHeight, images: document.images.length, visibility: document.visibilityState};
            """, arguments: [:], in: nil, in: .page) { result in
            switch result {
            case .failure(let error): fputs("Readiness failed: \(error)\n", stderr); exit(4)
            case .success(let dimensions):
                print("Ready: \(String(describing: dimensions))")
                let config = WKSnapshotConfiguration()
                config.rect = NSRect(x: 0, y: 0, width: 1920, height: 1080)
                config.snapshotWidth = 1920
                webView.takeSnapshot(with: config) { image, error in
                    guard let image, error == nil,
                          let source = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
                          let context = CGContext(data: nil, width: 1920, height: 1080, bitsPerComponent: 8, bytesPerRow: 1920 * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
                        fputs("Snapshot failed: \(String(describing: error))\n", stderr); exit(5)
                    }
                    // WKSnapshotConfiguration uses points; normalize Retina pixels explicitly.
                    context.interpolationQuality = .high
                    context.draw(source, in: CGRect(x: 0, y: 0, width: 1920, height: 1080))
                    let bitmap = NSBitmapImageRep(cgImage: context.makeImage()!)
                    guard let png = bitmap.representation(using: .png, properties: [:]) else { exit(5) }
                    do {
                        try png.write(to: self.output)
                        print("PNG: \(bitmap.pixelsWide)x\(bitmap.pixelsHigh), \(png.count) bytes")
                        exit(0)
                    } catch { fputs("Write failed: \(error)\n", stderr); exit(6) }
                }
            }
        }
    }
}
@main @MainActor struct Main {
 static func main() throws {
let args = CommandLine.arguments
if args.count < 3 { fputs("Usage: snapshot HTML PNG [attached]\n", stderr); exit(1) }
let application = NSApplication.shared
application.setActivationPolicy(.prohibited)
let probe = try SnapshotProbe(html: URL(fileURLWithPath: args[1]), output: URL(fileURLWithPath: args[2]), attached: args.count > 3)
fputs("Starting run loop\n", stderr)
withExtendedLifetime(probe) {
    while true { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
}

 }
}
