import SwiftUI
import VibeBuddyCore

/// The settings window's sections, in the sidebar's order (docs/design/korekore/README.md).
public enum KorekoreSection: String, CaseIterable, Identifiable, Sendable {
    case character, agents, conversation, device, general, advanced

    public var id: String { rawValue }

    var title: String {
        switch self {
        case .character: String(localized: "Character")
        case .agents: String(localized: "Agents")
        case .conversation: String(localized: "Conversation")
        case .device: String(localized: "Device")
        case .general: String(localized: "General")
        case .advanced: String(localized: "Advanced")
        }
    }

    var subtitle: String {
        switch self {
        case .character: String(localized: "Who lives in the box")
        case .agents: String(localized: "Who the box watches")
        case .conversation: String(localized: "Hold K0 on the box and ask")
        case .device: String(localized: "The box, its firmware and its network")
        case .general: String(localized: "How korekore behaves on this Mac")
        case .advanced: String(localized: "For when something's off")
        }
    }

    var tile: String {
        switch self {
        case .character: "icon-nav-char"
        case .agents: "icon-nav-agents"
        case .conversation: "icon-nav-chat"
        case .device: "icon-nav-box"
        case .general: "icon-nav-gen"
        case .advanced: "icon-nav-adv"
        }
    }
}

/// Versions for the foot of the sidebar.
public struct KorekoreVersions: Equatable, Sendable {
    public var app: String
    public var firmware: String?

    public init(app: String, firmware: String?) {
        self.app = app
        self.firmware = firmware
    }
}

/// The left side of the settings window (canvas: "App sidebar component").
struct KorekoreSidebar: View {
    let picture: BoxPicture
    let versions: KorekoreVersions
    let sections: [KorekoreSection]
    @Binding var selection: KorekoreSection

    /// The real window buttons sit here; the window moves them into this row.
    static let windowButtonsRow: CGFloat = 12

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Color.clear.frame(height: Self.windowButtonsRow)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: "korekore").font(.rounded(26)).tracking(-0.52).foregroundStyle(Theme.Colors.ink)
                PixelTag().padding(.top, 4)
            }
            .padding(.horizontal, 2)
            MiniBox(picture: picture).frame(maxWidth: .infinity)
            status
            VStack(spacing: 4) {
                ForEach(sections) { section in
                    row(section)
                }
            }
            Spacer(minLength: 0)
            footer
        }
        .padding(EdgeInsets(top: 14, leading: 14, bottom: 16, trailing: 14))
        .frame(width: 240 - Theme.Stroke.bold, alignment: .leading)
        .frame(maxHeight: .infinity)
        .background(Theme.Colors.paper)
        .overlay(alignment: .trailing) {
            Rectangle().fill(Theme.Colors.ink).frame(width: Theme.Stroke.bold).offset(x: Theme.Stroke.bold)
        }
        .padding(.trailing, Theme.Stroke.bold)
    }

    private var status: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Rectangle()
                    .fill(picture.link == nil ? Theme.Colors.mist : Theme.Colors.mint)
                    .frame(width: 8, height: 8)
                    .overlay(Rectangle().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.thin))
                linkLine
            }
            .foregroundStyle(Theme.Colors.ink)
            Text("\(picture.today.done) done · \(picture.today.asks) asks today").foregroundStyle(Theme.Colors.muted)
        }
        .font(.pixel(12))
        .padding(.horizontal, 4)
    }

    @ViewBuilder private var linkLine: some View {
        if !picture.daemonAlive {
            Text("DAEMON NOT RUNNING")
        } else if let link = picture.link {
            Text("BOX ONLINE · \(link.rawValue)")
        } else {
            Text("NO BOX LINKED")
        }
    }

    private func row(_ section: KorekoreSection) -> some View {
        let current = section == selection
        return Button {
            selection = section
        } label: {
            HStack(spacing: 10) {
                Sprite(name: section.tile, scale: 2)
                Text(verbatim: section.title).font(.rounded(15))
            }
            .padding(.vertical, 6)
            .padding(.horizontal, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .foregroundStyle(current ? Theme.Colors.cream : Theme.Colors.ink)
            .background(current ? Theme.Colors.ink : .clear, in: RoundedRectangle(cornerRadius: 12))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(current ? .isSelected : [])
    }

    private var footer: some View {
        Group {
            if let firmware = versions.firmware {
                Text("app \(versions.app) · box fw \(firmware)")
            } else {
                Text("app \(versions.app)")
            }
        }
        .font(.pixel(12))
        .foregroundStyle(Theme.Colors.muted2)
        .padding(.horizontal, 4)
    }
}

/// The whole settings window (canvas: "Mac · Character (sidebar)" and its siblings): cream, a 3 px ink outline,
/// radius 20 and the large sticker shadow, with the sidebar on the left and the section's page on the right.
public struct KorekoreSettings<Page: View>: View {
    let picture: BoxPicture
    let versions: KorekoreVersions
    let sections: [KorekoreSection]
    @Binding var selection: KorekoreSection
    let page: (KorekoreSection) -> Page

    public init(picture: BoxPicture, versions: KorekoreVersions, sections: [KorekoreSection],
                selection: Binding<KorekoreSection>, @ViewBuilder page: @escaping (KorekoreSection) -> Page) {
        self.picture = picture
        self.versions = versions
        self.sections = sections
        _selection = selection
        self.page = page
    }

    /// Room below the window's outline for its shadow.
    public static var shadow: CGFloat { Theme.Sticker.lg }

    public var body: some View {
        HStack(spacing: 0) {
            KorekoreSidebar(picture: picture, versions: versions, sections: sections, selection: $selection)
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .firstTextBaseline) {
                    Text(verbatim: selection.title).font(.korekore(Theme.Typography.title))
                    Spacer()
                    Text(verbatim: selection.subtitle).font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.muted)
                }
                page(selection)
                Spacer(minLength: 0)
            }
            .foregroundStyle(Theme.Colors.ink)
            .padding(EdgeInsets(top: 22, leading: 22, bottom: 20, trailing: 22))
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .sticker(radius: Theme.Radius.lg, fill: Theme.Colors.cream, stroke: Theme.Stroke.bold, shadow: Self.shadow)
        .padding(.bottom, Self.shadow)
    }
}

/// Stands in for a page that comes with a later step of the korekore UI.
public struct KorekorePagePending: View {
    public init() {}

    public var body: some View {
        Text("This page comes with the next step of korekore.")
            .font(.rounded(14, weight: 400))
            .foregroundStyle(Theme.Colors.muted)
            .frame(maxWidth: .infinity, minHeight: 120)
            .overlay(RoundedRectangle(cornerRadius: Theme.Radius.md).strokeBorder(Theme.Colors.ink, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
    }
}
