import SwiftUI
import VibeBuddyCore

public typealias Theme = KorekoreTheme

extension Font {
    /// A type style from tokens.json.
    public static func korekore(_ style: KorekoreTextStyle) -> Font {
        .custom(style.font, fixedSize: style.size)
    }

    /// M PLUS Rounded 1c at a size the type styles don't name (the sidebar's 26 px wordmark, say).
    public static func rounded(_ size: CGFloat, weight: Int = 800) -> Font {
        let name = switch weight {
        case ..<550: Theme.FontName.roundedRegular
        case ..<750: Theme.FontName.roundedBold
        default: Theme.FontName.roundedExtraBold
        }
        return .custom(name, fixedSize: size)
    }

    /// Pixel type, for the machine-ish bits: state words, versions, counts. A pixel font is clean only at its own size or
    /// a whole multiple, so the size snaps: below 15 pt Fusion Pixel at its own 12, then DotGothic16 at its own 16, and
    /// at 32 from 24 pt. DotGothic16 at the design's 12 to 14 pt fell between pixels and crowded its capitals.
    public static func pixel(_ size: CGFloat) -> Font {
        switch size {
        case ..<15: .custom(fusionPixel, fixedSize: 12)
        case ..<24: .custom(Theme.FontName.pixelRegular, fixedSize: 16)
        default: .custom(Theme.FontName.pixelRegular, fixedSize: 32)
        }
    }

    /// Fusion Pixel 12 px (fonts/korekore, tools/korekore-fonts.py), by its PostScript name.
    static let fusionPixel = "Fusion-Pixel-12px-Prop-latin-Regular"
}

/// A sprite from docs/design/korekore at a whole-number scale, every pixel a crisp square.
public struct Sprite: View {
    let name: String
    var scale = 1
    var tint: UInt32?
    /// Kore asleep without a link: CSS `grayscale(1) brightness(0.75)`, as the canvas draws it.
    var gray = false

    public init(name: String, scale: Int = 1, tint: UInt32? = nil, gray: Bool = false) {
        self.name = name
        self.scale = scale
        self.tint = tint
        self.gray = gray
    }

    public var body: some View {
        if let sprite = KorekoreSprites.all[name].map({ gray ? $0.grayed(brightness: 0.75) : $0 }),
           let image = sprite.image(scale: scale, tint: tint) {
            Image(nsImage: image)
                .interpolation(.none)
                .frame(width: CGFloat(sprite.width * scale), height: CGFloat(sprite.height * scale))
        }
    }
}

/// The "your vibebuddy" bitmap tag: 2 px dots, solid, in `kore-pink-text` beside small wordmarks.
public struct PixelTag: View {
    var dot = 2
    var color: UInt32 = 0xC2335B

    public init(dot: Int = 2, color: UInt32 = 0xC2335B) {
        self.dot = dot
        self.color = color
    }

    public var body: some View {
        Sprite(name: "tag-your-vibebuddy", scale: dot, tint: color)
    }
}

/// The sticker look: a fill, an ink outline inside the edge, and a solid ink shadow straight down, no blur.
struct Sticker: ViewModifier {
    var radius: CGFloat
    var fill: Color = Theme.Colors.paper
    var stroke: CGFloat = Theme.Stroke.base
    var outline: Color = Theme.Colors.ink
    var shadow: CGFloat = Theme.Sticker.sm

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        content
            .background(shape.fill(fill))
            .clipShape(shape)
            .overlay(shape.strokeBorder(outline, lineWidth: stroke))
            .background(shape.fill(Theme.Colors.ink).offset(y: shadow))
    }
}

extension View {
    public func sticker(radius: CGFloat, fill: Color = Theme.Colors.paper, stroke: CGFloat = Theme.Stroke.base,
                 outline: Color = Theme.Colors.ink, shadow: CGFloat = Theme.Sticker.sm) -> some View {
        modifier(Sticker(radius: radius, fill: fill, stroke: stroke, outline: outline, shadow: shadow))
    }
}

/// How each state reads on the box: its word, its colour and Kore's face.
extension BoxPicture.Mood {
    var word: String {
        switch self {
        case .ready: "READY"
        case .working: "WORKING"
        case .input: "ASK"
        case .pomodoro: "POMODORO"
        case .leisure: "LEISURE"
        case .noLink: "NO LINK"
        }
    }

    var color: Color {
        switch self {
        case .ready, .leisure: Theme.Colors.lavender
        case .working: Theme.Colors.sky
        case .input: Theme.Colors.sun
        case .pomodoro: Theme.Colors.korePink
        case .noLink: Theme.Colors.mist
        }
    }

    var expression: String {
        switch self {
        case .ready, .pomodoro, .leisure: "kore-normal"
        case .working: "kore-working"
        case .input: "kore-input"
        case .noLink: "kore-sleep"
        }
    }
}
