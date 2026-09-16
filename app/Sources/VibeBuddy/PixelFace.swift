import AppKit

/// 菜单栏里的氛围小助手：18 点的像素脸，模板图适应深浅色。闭眼版给链路断开。
enum PixelFace {
    static func image(eyesClosed: Bool) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let image = NSImage(size: size, flipped: true) { _ in
            NSColor.black.setFill()
            func px(_ x: Int, _ y: Int, _ w: Int = 1, _ h: Int = 1) {
                NSRect(x: x, y: y, width: w, height: h).fill()
            }
            // 天线
            px(8, 0, 2, 2); px(8, 2, 2, 1)
            // 头（圆角矩形轮廓）
            px(3, 4, 12, 1); px(2, 5, 1, 9); px(15, 5, 1, 9); px(3, 14, 12, 1)
            // 眼睛
            if eyesClosed {
                px(5, 9, 3, 1); px(10, 9, 3, 1)
            } else {
                px(5, 7, 3, 3); px(10, 7, 3, 3)
            }
            // 嘴
            if eyesClosed { px(7, 12, 4, 1) } else { px(6, 12, 1, 1); px(7, 13, 4, 1); px(11, 12, 1, 1) }
            // 腿
            px(5, 15, 2, 2); px(11, 15, 2, 2)
            return true
        }
        image.isTemplate = true
        return image
    }

    /// 引导页与关于页里的大号版本。
    static func largeImage() -> NSImage {
        let base = image(eyesClosed: false)
        base.isTemplate = false
        let big = NSImage(size: NSSize(width: 108, height: 108))
        big.lockFocus()
        NSGraphicsContext.current?.imageInterpolation = .none
        base.draw(in: NSRect(x: 0, y: 0, width: 108, height: 108))
        big.unlockFocus()
        return big
    }
}
