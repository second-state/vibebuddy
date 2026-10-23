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
}

/// Which settings window, onboarding and menu bar the app shows (ADR-0021). The korekore UI opens only with
/// `VIBEBUDDY_UI=korekore` while it is being built; read once at launch.
public enum AppUI: Equatable, Sendable {
    case current
    case korekore

    public static func atLaunch(environment: [String: String] = ProcessInfo.processInfo.environment) -> AppUI {
        environment["VIBEBUDDY_UI"] == "korekore" ? .korekore : .current
    }
}
