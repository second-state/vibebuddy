import SwiftUI
import VibeBuddyCore

/// The box's screen redrawn at the head of the sidebar (canvas: "App sidebar component"): laid out at the box's
/// 320 × 240 and shown at 0.65, 208 × 156, as the design does.
public struct MiniBox: View {
    let picture: BoxPicture
    let scale: CGFloat
    let framed: Bool

    /// At `scale` of the box's size; `framed` adds the sidebar's outline and shadow.
    public init(picture: BoxPicture, scale: CGFloat = 0.65, framed: Bool = true) {
        self.picture = picture
        self.scale = scale
        self.framed = framed
    }

    public var body: some View {
        let screen = screen
            .scaleEffect(scale, anchor: .topLeading)
            .frame(width: 320 * scale, height: 240 * scale, alignment: .topLeading)
        if framed {
            screen
                // The outline sits outside the screen, as the canvas's content-box does.
                .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.md - Theme.Stroke.bold, style: .continuous))
                .padding(Theme.Stroke.bold)
                .sticker(radius: Theme.Radius.md, fill: Theme.Colors.ink, stroke: Theme.Stroke.bold)
        } else {
            screen
        }
    }

    @ViewBuilder private var screen: some View {
        if picture.mood == .noLink {
            asleep
        } else {
            onDuty
        }
    }

    /// No link: the whole screen gray, Kore asleep and grayed at scale 6 (README, "No link").
    private var asleep: some View {
        VStack(spacing: 6) {
            Text(verbatim: "Z z").font(.pixel(14)).foregroundStyle(Theme.Colors.mist)
            Sprite(name: "kore-sleep", scale: 6, gray: true)
            Text(verbatim: "NO LINK").font(.korekore(Theme.Typography.boxState)).foregroundStyle(Theme.Colors.mist)
            Text(verbatim: "waiting for the Mac").font(.pixel(11)).foregroundStyle(Theme.Colors.mistText)
        }
        .frame(width: 320, height: 240)
        .background(Theme.Colors.boxGrayGround)
    }

    private var onDuty: some View {
        let mood = picture.mood
        return HStack(spacing: 0) {
            VStack(spacing: 8) {
                Sprite(name: mood.expression, scale: 4)
                Text(verbatim: mood.word).font(.pixel(16)).foregroundStyle(mood.color)
            }
            .frame(width: 96, height: 240)
            .background(Theme.Colors.boxCard)
            VStack(alignment: .leading, spacing: 8) {
                if picture.tickets.isEmpty {
                    quiet
                } else {
                    ForEach(Array(picture.tickets.enumerated()), id: \.offset) { _, ticket in
                        HStack(spacing: 8) {
                            Sprite(name: "icon-\(ticket.icon)", scale: 2)
                            Text(verbatim: ticket.title).font(.pixel(13)).lineLimit(1)
                            Spacer(minLength: 0)
                        }
                        .padding(7)
                        .background(Theme.Colors.boxCard, in: RoundedRectangle(cornerRadius: 8))
                    }
                    Spacer(minLength: 0)
                }
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 8)
        }
        .foregroundStyle(Theme.Colors.boxText)
        .frame(width: 320, height: 240)
        .background(Theme.Colors.boxGround)
    }

    /// No cards: the box's ALL QUIET with today's numbers.
    private var quiet: some View {
        VStack(spacing: 6) {
            Text(verbatim: "ALL QUIET").font(.pixel(12))
            Text(verbatim: "\(picture.today.done) done · \(picture.today.asks) asks · \(BoxPicture.busy(picture.today.busySeconds).uppercased()) busy")
                .font(.pixel(10))
                .foregroundStyle(Theme.Colors.boxTextMuted)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
