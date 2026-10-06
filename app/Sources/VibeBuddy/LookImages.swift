import AppKit
import VibeBuddyCore

/// Between AppKit images and the plain pixels the look building works on.
enum LookImages {
    /// Decodes an image file into RGBA pixels at its own size; nil for a file AppKit can't read.
    static func load(_ url: URL) -> RGBAImage? {
        guard let image = NSImage(contentsOf: url),
              let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return nil }
        let width = cgImage.width, height = cgImage.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        return drawn ? RGBAImage(width: width, height: height, pixels: pixels) : nil
    }

    /// A look frame as an image, each look pixel drawn `scale` screen pixels wide, unsmoothed.
    static func image(_ frame: RGBAImage, scale: Int) -> NSImage {
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: frame.width * scale, pixelsHigh: frame.height * scale, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        for y in 0..<(frame.height * scale) {
            for x in 0..<(frame.width * scale) {
                let (r, g, b, a) = frame.pixel(x / scale, y / scale)
                var color = [Int(r), Int(g), Int(b), Int(a)]
                rep.setPixel(&color, atX: x, y: y)
            }
        }
        let image = NSImage(size: NSSize(width: frame.width * scale, height: frame.height * scale))
        image.addRepresentation(rep)
        return image
    }
}
