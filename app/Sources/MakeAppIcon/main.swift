import AppKit
import KorekoreUI

/// Writes the app icon's iconset from the same image the Dock shows (KorekoreAppIcon), so the bundle, Finder and the
/// DMG show that icon too. build-app.sh turns the iconset into AppIcon.icns.
/// Usage: MakeAppIcon <output .iconset directory>
guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: MakeAppIcon <output.iconset>\n".data(using: .utf8)!)
    exit(2)
}
let out = URL(fileURLWithPath: CommandLine.arguments[1])
try! FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

let master = KorekoreAppIcon.image()
// The iconset's ten sizes: a 1x and a 2x image for each point size.
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let px = points * scale
        let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
        try! png(of: master, size: px).write(to: out.appendingPathComponent(name))
    }
}

func png(of image: NSImage, size: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    let context = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = context
    // Small sizes must resample the pixel art; high-quality interpolation keeps it from breaking into noise.
    context.imageInterpolation = size >= 1024 ? .none : .high
    image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}
