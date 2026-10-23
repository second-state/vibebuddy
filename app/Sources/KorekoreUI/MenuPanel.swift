import AppKit
import SwiftUI
import VibeBuddyCore

/// What the menu bar panel's rows do; the app fills these in.
public struct KorekoreMenuActions {
    public var settings: () -> Void
    public var checkForUpdates: () -> Void
    public var reportProblem: () -> Void
    public var quit: () -> Void
    /// The daemon isn't answering: start it again.
    public var restartDaemon: () -> Void

    public init(settings: @escaping () -> Void, checkForUpdates: @escaping () -> Void, reportProblem: @escaping () -> Void,
                quit: @escaping () -> Void, restartDaemon: @escaping () -> Void) {
        self.settings = settings
        self.checkForUpdates = checkForUpdates
        self.reportProblem = reportProblem
        self.quit = quit
        self.restartDaemon = restartDaemon
    }
}

/// The panel the menu bar item opens (canvas: "Mac · Menu bar"), 360 px wide.
public struct KorekoreMenuPanel: View {
    let picture: BoxPicture
    let actions: KorekoreMenuActions
    @State private var hovered: Int?

    public init(picture: BoxPicture, actions: KorekoreMenuActions) {
        self.picture = picture
        self.actions = actions
    }

    public static let width: CGFloat = 360

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .bottom, spacing: 8) {
                Text(verbatim: "korekore").font(.rounded(18))
                PixelTag()
            }
            strip
            modes
            stats
            items
        }
        .foregroundStyle(Theme.Colors.ink)
        .padding(14)
        .frame(width: Self.width - 2 * Theme.Stroke.bold, alignment: .leading)
        .sticker(radius: 18, fill: Theme.Colors.cream, stroke: Theme.Stroke.bold, shadow: Theme.Sticker.md)
        .padding(.bottom, Theme.Sticker.md)
    }

    /// The box at a glance, on Ink like the box itself. Without the daemon it offers to start it again.
    private var strip: some View {
        let mood = picture.mood
        return HStack(spacing: 12) {
            Sprite(name: mood.expression, scale: 3, gray: mood == .noLink)
            VStack(alignment: .leading, spacing: 3) {
                stripTitle.font(.pixel(16)).foregroundStyle(mood.color)
                stripDetail.font(.pixel(12)).foregroundStyle(Theme.Colors.boxTextMuted)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .background(Theme.Colors.ink, in: RoundedRectangle(cornerRadius: 12))
        .contentShape(Rectangle())
        .onTapGesture { if !picture.daemonAlive { actions.restartDaemon() } }
    }

    @ViewBuilder private var stripTitle: some View {
        let word = picture.mood.word
        switch picture.mood {
        case .working, .input:
            if picture.activityCount == 1 {
                Text(verbatim: "\(word) · ") + Text("1 AGENT")
            } else {
                Text(verbatim: "\(word) · ") + Text("\(picture.activityCount) AGENTS")
            }
        default:
            Text(verbatim: word)
        }
    }

    @ViewBuilder private var stripDetail: some View {
        if !picture.daemonAlive {
            Text("daemon not responding · click to restart")
        } else if let link = picture.link {
            if let firmware = picture.firmwareVersion {
                Text("box online · \(link.rawValue) · fw \(firmware)")
            } else {
                Text("box online · \(link.rawValue)")
            }
        } else {
            Text("no box linked")
        }
    }

    /// The box's mode, as chips; the box changes it (K0), so here they only show it.
    private var modes: some View {
        HStack(spacing: 6) {
            // Without a link the last mode heard may be stale, so none is lit.
            chip(String(localized: "On duty"), on: picture.link != nil && picture.mode == "duty")
            chip(String(localized: "Pomodoro"), on: picture.link != nil && picture.mode == "pomodoro")
            chip(String(localized: "Leisure"), on: picture.link != nil && picture.mode == "leisure")
        }
    }

    private func chip(_ label: String, on: Bool) -> some View {
        Text(verbatim: label)
            .font(.rounded(12))
            .padding(.vertical, 3)
            .padding(.horizontal, 10)
            .foregroundStyle(on ? Theme.Colors.cream : Theme.Colors.ink)
            .background(on ? Theme.Colors.ink : Theme.Colors.paper, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
    }

    private var stats: some View {
        HStack(spacing: 8) {
            stat(Text("\(Text(verbatim: "\(picture.today.done)").foregroundColor(Theme.Colors.doneText)) done"))
            stat(Text("\(Text(verbatim: "\(picture.today.asks)").foregroundColor(Theme.Colors.waitText)) asks"))
            stat(Text("\(Text(verbatim: BoxPicture.busy(picture.today.busySeconds)).foregroundColor(Theme.Colors.workText)) busy"))
        }
        .font(.pixel(13))
    }

    private func stat(_ label: Text) -> some View {
        label
            .lineLimit(1)
            .padding(6)
            .frame(maxWidth: .infinity)
            .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
    }

    private var items: some View {
        VStack(spacing: 0) {
            item(0, String(localized: "Settings…"), key: "⌘,", action: actions.settings)
            item(1, String(localized: "Check for updates…"), action: actions.checkForUpdates)
            item(2, String(localized: "Report a problem…"), action: actions.reportProblem)
            item(3, String(localized: "Quit korekore (the box goes offline)"), key: "⌘Q", action: actions.quit)
        }
        .padding(.top, 6)
        .overlay(alignment: .top) { Rectangle().fill(Theme.Colors.line).frame(height: Theme.Stroke.base) }
    }

    private func item(_ index: Int, _ label: String, key: String = "", action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                Text(verbatim: label).font(.rounded(14, weight: 700))
                Spacer()
                Text(verbatim: key).font(.pixel(12)).foregroundStyle(Theme.Colors.muted2)
            }
            .padding(.vertical, 7)
            .padding(.horizontal, 8)
            .background(hovered == index ? Theme.Colors.paper : .clear, in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 ? index : (hovered == index ? nil : hovered) }
    }
}

/// The menu bar item: Kore at 1×, in colour online, with a `sun` dot when an agent needs you, gray and asleep
/// without a link (canvas: "Mac · Menu bar", bottom row).
public enum KorekoreMenuIcon {
    public static func image(for picture: BoxPicture) -> NSImage {
        let mood = picture.mood
        let needsYou = picture.tickets.contains(where: \.needsInput)
        let awake = KorekoreSprites.all[needsYou ? "kore-input" : "kore-normal"]!
        let kore = mood == .noLink ? KorekoreSprites.koreSleep.grayed(brightness: 0.75) : awake
        // Two image pixels per point, so each of Kore's pixels is one point, sharp on Retina.
        let pixels = kore.rgba(scale: 2)
        let dot: CGFloat = needsYou ? 7 + 4 : 0
        let size = NSSize(width: CGFloat(kore.width) + dot, height: CGFloat(kore.height))
        let image = NSImage(size: size, flipped: false) { _ in
            guard let context = NSGraphicsContext.current?.cgContext,
                  let provider = CGDataProvider(data: Data(pixels) as CFData),
                  let sprite = CGImage(width: kore.width * 2, height: kore.height * 2, bitsPerComponent: 8, bitsPerPixel: 32,
                                       bytesPerRow: kore.width * 8, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue), provider: provider,
                                       decode: nil, shouldInterpolate: false, intent: .defaultIntent)
            else { return false }
            context.interpolationQuality = .none
            context.draw(sprite, in: CGRect(x: 0, y: 0, width: kore.width, height: kore.height))
            if needsYou {
                let rect = CGRect(x: CGFloat(kore.width) + 4 + 0.75, y: (CGFloat(kore.height) - 7) / 2 + 0.75, width: 5.5, height: 5.5)
                context.setFillColor(NSColor(Theme.Colors.sun).cgColor)
                context.fillEllipse(in: rect)
                context.setStrokeColor(NSColor(Theme.Colors.ink).cgColor)
                context.setLineWidth(1.5)
                context.strokeEllipse(in: rect)
            }
            return true
        }
        return image
    }
}
