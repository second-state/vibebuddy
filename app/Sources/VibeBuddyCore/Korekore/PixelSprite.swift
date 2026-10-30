import Foundation

/// A palette-indexed sprite from docs/design/korekore (KorekoreSprites): one index per pixel, row by row; 0 is
/// transparent and n is `palette[n - 1]`, as 0xRRGGBB.
public struct PixelSprite: Sendable {
    public let width: Int
    public let height: Int
    public let palette: [UInt32]
    public let pixels: [UInt8]

    public init(width: Int, height: Int, palette: [UInt32], pixels: [UInt8]) {
        self.width = width
        self.height = height
        self.palette = palette
        self.pixels = pixels
    }

    /// RGBA bytes at `scale` whole times the size, nearest-neighbour. `tint` (0xRRGGBB) replaces every colour,
    /// for the one-colour tag.
    public func rgba(scale: Int, tint: UInt32? = nil) -> [UInt8] {
        let (outWidth, outHeight) = (width * scale, height * scale)
        var bytes = [UInt8](repeating: 0, count: outWidth * outHeight * 4)
        for y in 0..<outHeight {
            for x in 0..<outWidth {
                let index = pixels[(y / scale) * width + x / scale]
                guard index > 0 else { continue }
                let rgb = tint ?? palette[Int(index) - 1]
                let at = (y * outWidth + x) * 4
                bytes[at] = UInt8(rgb >> 16 & 0xFF)
                bytes[at + 1] = UInt8(rgb >> 8 & 0xFF)
                bytes[at + 2] = UInt8(rgb & 0xFF)
                bytes[at + 3] = 0xFF
            }
        }
        return bytes
    }

    /// The sprite in gray, as CSS `grayscale(1) brightness(b)` draws it: each colour's luminance (Rec. 709
    /// weights), scaled by `brightness`. The canvas grays Kore this way when the box has no link.
    public func grayed(brightness: Double) -> PixelSprite {
        // Typed step by step: as one expression it is too slow for the type checker on CI's Xcode.
        let palette = palette.map { (rgb: UInt32) -> UInt32 in
            let r: Double = Double((rgb >> 16) & 0xFF)
            let g: Double = Double((rgb >> 8) & 0xFF)
            let b: Double = Double(rgb & 0xFF)
            let luminance: Double = 0.2126 * r + 0.7152 * g + 0.0722 * b
            let level: UInt32 = UInt32(Swift.min(255.0, Swift.max(0.0, (luminance * brightness).rounded())))
            return (level << 16) | (level << 8) | level
        }
        return PixelSprite(width: width, height: height, palette: palette, pixels: pixels)
    }
}
