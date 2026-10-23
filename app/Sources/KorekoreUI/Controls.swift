import SwiftUI

// The korekore controls (canvas: the app boards and their "every state" boards). Every page uses these, so a page
// only says what goes where.

/// A white card: 2 px ink outline, radius 16, the small sticker shadow.
public struct KCard<Content: View>: View {
    var padding: EdgeInsets
    var fill: Color
    var outline: Color
    var stroke: CGFloat
    var dashed: Bool
    let content: Content

    public init(padding: EdgeInsets = EdgeInsets(top: 14, leading: 14, bottom: 14, trailing: 14), fill: Color = Theme.Colors.paper,
                outline: Color = Theme.Colors.ink, stroke: CGFloat = Theme.Stroke.base, dashed: Bool = false,
                @ViewBuilder content: () -> Content) {
        self.padding = padding
        self.fill = fill
        self.outline = outline
        self.stroke = stroke
        self.dashed = dashed
        self.content = content()
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: Theme.Radius.md, style: .continuous)
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(shape.fill(fill))
            .clipShape(shape)
            .overlay(shape.strokeBorder(outline, style: StrokeStyle(lineWidth: stroke, dash: dashed ? [6, 4] : [])))
            .background(shape.fill(Theme.Colors.ink).offset(y: dashed ? 0 : Theme.Sticker.sm))
    }
}

/// A card made of rows; put a `KDivider` between them.
public struct KRows<Content: View>: View {
    let content: Content

    public init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    public var body: some View {
        KCard(padding: EdgeInsets()) {
            VStack(spacing: 0) { content }
        }
    }
}

/// The 2 px `line` between rows inside a card.
public struct KDivider: View {
    public init() {}

    public var body: some View {
        Rectangle().fill(Theme.Colors.line).frame(height: Theme.Stroke.base)
    }
}

/// The fills a pill button comes in: `sun` for the one primary action, `kore-pink` to commit a change.
public enum KFill {
    case paper, sun, pink, ink

    var background: Color {
        switch self {
        case .paper: Theme.Colors.paper
        case .sun: Theme.Colors.sun
        case .pink: Theme.Colors.korePink
        case .ink: Theme.Colors.ink
        }
    }

    var text: Color { self == .ink ? Theme.Colors.cream : Theme.Colors.ink }
}

/// Everything you press: a pill with a 2 px ink outline, 12/800 (13/800 for the larger ones).
public struct KButtonStyle: ButtonStyle {
    var fill: KFill
    var size: CGFloat
    var outline: Color

    public init(_ fill: KFill = .paper, size: CGFloat = 12, outline: Color = Theme.Colors.ink) {
        self.fill = fill
        self.size = size
        self.outline = outline
    }

    public func makeBody(configuration: Configuration) -> some View {
        PillLabel(configuration: configuration, fill: fill, size: size, outline: outline)
    }

    private struct PillLabel: View {
        let configuration: Configuration
        let fill: KFill
        let size: CGFloat
        let outline: Color
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            configuration.label
                .font(.rounded(size))
                .lineLimit(1)
                .padding(.vertical, size > 12 ? 6 : 4)
                .padding(.horizontal, size > 12 ? 14 : 12)
                .foregroundStyle(fill.text)
                .background(fill.background, in: Capsule())
                .overlay(Capsule().strokeBorder(outline, lineWidth: Theme.Stroke.base))
                .opacity(enabled ? (configuration.isPressed ? 0.8 : 1) : 0.45)
                .contentShape(Capsule())
        }
    }
}

extension View {
    /// The korekore pill for a button.
    public func kButton(_ fill: KFill = .paper, size: CGFloat = 12) -> some View {
        buttonStyle(KButtonStyle(fill, size: size))
    }
}

/// The switch: a 48 × 28 pill track, `kore-pink` on, `backdrop` off, a white knob with an ink outline.
public struct KToggle: View {
    @Binding var isOn: Bool
    let label: String
    @Environment(\.isEnabled) private var enabled

    public init(_ label: String, isOn: Binding<Bool>) {
        self.label = label
        _isOn = isOn
    }

    public var body: some View {
        Button { isOn.toggle() } label: {
            Capsule()
                .fill(isOn ? Theme.Colors.korePink : Theme.Colors.backdrop)
                .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                .overlay(alignment: isOn ? .trailing : .leading) {
                    Circle().fill(Theme.Colors.paper)
                        .overlay(Circle().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                        .frame(width: 20, height: 20)
                        .padding(4)
                }
                .frame(width: 48, height: 28)
                .opacity(enabled ? 1 : 0.45)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityValue(isOn ? Text(verbatim: "1") : Text(verbatim: "0"))
        .accessibilityAddTraits(.isToggle)
    }
}

/// A row of a settings card: a 14/700 label, then whatever controls it.
public struct KRow<Trailing: View>: View {
    let label: String
    let detail: String?
    let trailing: Trailing

    public init(_ label: String, detail: String? = nil, @ViewBuilder trailing: () -> Trailing) {
        self.label = label
        self.detail = detail
        self.trailing = trailing()
    }

    public var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: label).font(.rounded(14, weight: 700))
                if let detail {
                    Text(verbatim: detail).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 16)
    }
}

/// A state in pixel type, in a readable shade of its colour, no background: WATCHING, NOT FOUND, UP TO DATE.
public struct KTag: View {
    let text: String
    let color: Color
    var size: CGFloat

    public init(_ text: String, color: Color = Theme.Colors.muted2, size: CGFloat = 12) {
        self.text = text
        self.color = color
        self.size = size
    }

    public var body: some View {
        Text(verbatim: text.uppercased()).font(.pixel(size)).foregroundStyle(color)
    }
}

/// Progress: a 12 px pill track in cream with a 2 px ink outline, filled in the given colour.
public struct KBar: View {
    let progress: Double
    var fill: Color

    public init(_ progress: Double, fill: Color = Theme.Colors.korePink) {
        self.progress = progress
        self.fill = fill
    }

    public var body: some View {
        GeometryReader { geometry in
            Rectangle().fill(fill).frame(width: geometry.size.width * min(1, max(0, progress)))
        }
        .frame(height: 12)
        .background(Theme.Colors.cream)
        .clipShape(Capsule())
        .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
    }
}

/// A card heading, 15/800.
public struct KHeading: View {
    let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        Text(verbatim: text).font(.rounded(15))
    }
}

/// Body copy, 13 px `muted`, wrapping.
public struct KNote: View {
    let text: String
    var color: Color

    public init(_ text: String, color: Color = Theme.Colors.muted) {
        self.text = text
        self.color = color
    }

    public var body: some View {
        Text(verbatim: text).font(.rounded(13, weight: 400)).foregroundStyle(color).lineSpacing(3)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A pick from a short list, shown as a pill with a chevron.
public struct KMenu<Value: Hashable>: View {
    let selection: Value
    let options: [(value: Value, label: String)]
    let pick: (Value) -> Void

    public init(selection: Value, options: [(value: Value, label: String)], pick: @escaping (Value) -> Void) {
        self.selection = selection
        self.options = options
        self.pick = pick
    }

    public var body: some View {
        Menu {
            ForEach(options, id: \.value) { option in
                Button { pick(option.value) } label: { Text(verbatim: option.label) }
            }
        } label: {
            HStack(spacing: 8) {
                Text(verbatim: options.first { $0.value == selection }?.label ?? "")
                Image(systemName: "chevron.down").font(.system(size: 9, weight: .heavy))
            }
            .font(.rounded(13))
            .foregroundStyle(Theme.Colors.ink)
            .padding(.vertical, 4)
            .padding(.horizontal, 12)
            .background(Theme.Colors.paper, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
        }
        // Drawn as given: the borderless style would put AppKit's own popup in its place.
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
    }
}

/// A text field: DotGothic16 14 px in a white box with a 2 px ink outline, radius 10.
public struct KField: View {
    let prompt: String
    @Binding var text: String
    var secure: Bool

    public init(_ prompt: String, text: Binding<String>, secure: Bool = false) {
        self.prompt = prompt
        _text = text
        self.secure = secure
    }

    public var body: some View {
        Group {
            if secure {
                SecureField(text: $text, prompt: Text(verbatim: prompt)) { Text(verbatim: prompt) }
            } else {
                TextField(text: $text, prompt: Text(verbatim: prompt)) { Text(verbatim: prompt) }
            }
        }
        .textFieldStyle(.plain)
        .font(.pixel(14))
        .padding(.vertical, 6)
        .padding(.horizontal, 10)
        .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
        .overlay(RoundedRectangle(cornerRadius: Theme.Radius.sm).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
    }
}

/// Kore on a small Ink stage, as the pages put it beside a note.
public struct KoreBadge: View {
    let expression: String
    var scale: Int
    var gray: Bool
    var fill: Color

    public init(_ expression: String = "kore-normal", scale: Int = 2, gray: Bool = false, fill: Color = Theme.Colors.ink) {
        self.expression = expression
        self.scale = scale
        self.gray = gray
        self.fill = fill
    }

    public var body: some View {
        Sprite(name: expression, scale: scale, gray: gray)
            .padding(.vertical, 5)
            .padding(.horizontal, 6)
            .background(fill, in: RoundedRectangle(cornerRadius: 12))
    }
}
