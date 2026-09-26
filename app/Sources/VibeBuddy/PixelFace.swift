import AppKit

/// The buddy in the menu bar: an 18-point pixel face as a template image, so it adapts to light and dark. The closed-eye version means the link is down.
enum PixelFace {
    static func image(eyesClosed: Bool) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let image = NSImage(size: size, flipped: true) { _ in
            NSColor.black.setFill()
            func px(_ x: Int, _ y: Int, _ w: Int = 1, _ h: Int = 1) {
                NSRect(x: x, y: y, width: w, height: h).fill()
            }
            // Antenna
            px(8, 0, 2, 2); px(8, 2, 2, 1)
            // Head (rounded rectangle outline)
            px(3, 4, 12, 1); px(2, 5, 1, 9); px(15, 5, 1, 9); px(3, 14, 12, 1)
            // Eyes
            if eyesClosed {
                px(5, 9, 3, 1); px(10, 9, 3, 1)
            } else {
                px(5, 7, 3, 3); px(10, 7, 3, 3)
            }
            // Mouth
            if eyesClosed { px(7, 12, 4, 1) } else { px(6, 12, 1, 1); px(7, 13, 4, 1); px(11, 12, 1, 1) }
            // Legs
            px(5, 15, 2, 2); px(11, 15, 2, 2)
            return true
        }
        image.isTemplate = true
        return image
    }

    /// The large version for onboarding and About.
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
