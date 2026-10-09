// Native H.264/AAC smoke test. Inputs: snapshot PNG, narration WAV, output MP4.
import Foundation
import AVFoundation
import AppKit
import CoreVideo

func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw NSError(domain: "SlopSlideProbe", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
}
func makeBuffer(_ image: CGImage) throws -> CVPixelBuffer {
    var result: CVPixelBuffer?
    let attributes = [kCVPixelBufferCGImageCompatibilityKey: true, kCVPixelBufferCGBitmapContextCompatibilityKey: true] as CFDictionary
    try require(CVPixelBufferCreate(kCFAllocatorDefault, 1920, 1080, kCVPixelFormatType_32ARGB, attributes, &result) == kCVReturnSuccess, "Pixel buffer allocation failed")
    let buffer = result!
    CVPixelBufferLockBaseAddress(buffer, [])
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
    let ctx = CGContext(data: CVPixelBufferGetBaseAddress(buffer), width: 1920, height: 1080, bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(buffer), space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue)!
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    return buffer
}
@main struct Main {
 static func main() async throws {
    let args = CommandLine.arguments
    try require(args.count == 4, "Usage: encode PNG WAV OUTPUT.mp4")
    let destination = URL(fileURLWithPath: args[3])
    try require(!FileManager.default.fileExists(atPath: destination.path), "Output already exists")
    guard let image = NSImage(contentsOfFile: args[1])?.cgImage(forProposedRect: nil, context: nil, hints: nil) else { throw NSError(domain: "Image", code: 1) }
    try require(image.width == 1920 && image.height == 1080, "PNG is not 1080p")
    let audio = AVURLAsset(url: URL(fileURLWithPath: args[2]))
    let audioTracks = try await audio.loadTracks(withMediaType: .audio)
    let duration = try await audio.load(.duration)
    try require(!audioTracks.isEmpty && duration.seconds > 0, "Missing audio")
    let frames = Int(ceil(duration.seconds * 30))
    let videoURL = destination.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".video.mp4")
    defer { try? FileManager.default.removeItem(at: videoURL) }
    let writer = try AVAssetWriter(outputURL: videoURL, fileType: .mp4)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 1920, AVVideoHeightKey: 1080, AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 4_000_000]])
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 1920, kCVPixelBufferHeightKey as String: 1080])
    writer.add(input)
    try require(writer.startWriting(), "Writer start failed: \(String(describing: writer.error))")
    writer.startSession(atSourceTime: .zero)
    let buffer = try makeBuffer(image)
    for frame in 0..<frames {
        let deadline = Date().addingTimeInterval(30)
        while !input.isReadyForMoreMediaData {
            try require(writer.status == .writing && Date() < deadline, "Encoder stalled: \(String(describing: writer.error))")
            try await Task.sleep(nanoseconds: 1_000_000)
        }
        try require(adaptor.append(buffer, withPresentationTime: CMTime(value: Int64(frame), timescale: 30)), "Append failed")
    }
    writer.endSession(atSourceTime: CMTime(value: Int64(frames), timescale: 30))
    input.markAsFinished()
    await writer.finishWriting()
    try require(writer.status == .completed, "Video failed: \(String(describing: writer.error))")
    let video = AVURLAsset(url: videoURL)
    let tracks = try await video.loadTracks(withMediaType: .video)
    let composition = AVMutableComposition()
    let vtrack = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    let atrack = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    try vtrack.insertTimeRange(CMTimeRange(start: .zero, duration: CMTime(value: Int64(frames), timescale: 30)), of: tracks[0], at: .zero)
    try atrack.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: audioTracks[0], at: .zero)
    guard let exporter = AVAssetExportSession(asset: composition, presetName: AVAssetExportPreset1920x1080) else { throw NSError(domain: "Export", code: 1) }
    try await exporter.export(to: destination, as: .mp4)
    let result = AVURLAsset(url: destination)
    let resultTracks = try await result.load(.tracks)
    let finalDuration = try await result.load(.duration)
    print("Exported H.264/AAC MP4: \(resultTracks.count) tracks, \(finalDuration.seconds) seconds, \(frames) frames")
 }
}
