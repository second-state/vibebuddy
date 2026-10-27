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

    /// Asks for one to four drawings (normal, eyes closed, happy, sad, in file name order) and makes a look of them:
    /// nil when the user cancels, else the look or what is wrong with the images. The panel is a sheet on the key
    /// window: on its own, it can open on another display, out of sight, and hold the whole App.
    static func chooseLook(then done: @escaping (Result<Data, LookProblem>?) -> Void) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = [.png, .jpeg]
        let finish = { (response: NSApplication.ModalResponse) in
            done(response == .OK ? look(of: panel.urls) : nil)
        }
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    private static func look(of chosen: [URL]) -> Result<Data, LookProblem> {
        // In name order, so normal.png, closed.png, ... or 1.png, 2.png, ... come in as meant.
        let urls = chosen.sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
        let images = urls.compactMap(load)
        guard images.count == urls.count, (1...4).contains(images.count) else {
            return .failure(LookProblem(message: String(localized: "Pick one to four PNG or JPEG images.")))
        }
        guard let look = try? LookBuilder.build(images) else {
            return .failure(LookProblem(message: String(localized: "No figure found: use a plain white or transparent background.")))
        }
        return .success(look)
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

struct LookProblem: Error {
    let message: String
}
