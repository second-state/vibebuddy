import AppKit

/// 把菜单栏那张像素脸渲染成 App 图标的 iconset。和 PixelFace.swift 一起编译，
/// 脸只在那一处定义；配色照盒子屏幕：底 0x0841、宠物 0x3c9f。
/// 用法: make-app-icon <输出.iconset 目录>
@main
struct MakeAppIcon {
    static func main() {
        guard CommandLine.arguments.count == 2 else {
            FileHandle.standardError.write("用法: make-app-icon <输出.iconset>\n".data(using: .utf8)!)
            exit(2)
        }
        let out = URL(fileURLWithPath: CommandLine.arguments[1])
        try! FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

        let master = render(canvas: 1024)
        // iconset 的十个尺寸：每个点尺寸各一张 1x 与 2x。
        for points in [16, 32, 128, 256, 512] {
            for scale in [1, 2] {
                let px = points * scale
                let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
                try! png(of: master, size: px).write(to: out.appendingPathComponent(name))
            }
        }
    }

    /// Apple 的 macOS 图标网格：1024 画布，圆角方块 824 见方居中，圆角 185。
    static func render(canvas: Int) -> NSImage {
        let background = NSColor(srgbRed: 8 / 255, green: 8 / 255, blue: 8 / 255, alpha: 1)
        let pet = NSColor(srgbRed: 56 / 255, green: 144 / 255, blue: 248 / 255, alpha: 1)
        let face = PixelFace.image(eyesClosed: false)
        face.isTemplate = false
        let size = CGFloat(canvas)
        return NSImage(size: NSSize(width: size, height: size), flipped: false) { _ in
            let plate = NSRect(x: 100, y: 100, width: 824, height: 824)
            background.setFill()
            NSBezierPath(roundedRect: plate, xRadius: 185, yRadius: 185).fill()
            // 18 格的脸放大 32 倍；脸的像素占第 2..15 列，整体向右挪半格才居中。
            let faceSize: CGFloat = 18 * 32
            let origin = (size - faceSize) / 2
            let faceRect = NSRect(x: origin + 16, y: origin, width: faceSize, height: faceSize)
            let tinted = NSImage(size: faceRect.size, flipped: false) { rect in
                NSGraphicsContext.current?.imageInterpolation = .none
                face.draw(in: rect)
                pet.set()
                rect.fill(using: .sourceAtop)
                return true
            }
            NSGraphicsContext.current?.imageInterpolation = .none
            tinted.draw(in: faceRect)
            return true
        }
    }

    static func png(of image: NSImage, size: Int) -> Data {
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        let context = NSGraphicsContext(bitmapImageRep: rep)!
        NSGraphicsContext.current = context
        // 缩到小尺寸时像素脸必然要重采样，用高质量插值，别让它碎成噪点。
        context.imageInterpolation = size >= 1024 ? .none : .high
        image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
        NSGraphicsContext.restoreGraphicsState()
        return rep.representation(using: .png, properties: [:])!
    }
}
