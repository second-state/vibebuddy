import AppKit
import SwiftUI

/// The onboarding window's frame (canvas: "Mac · Onboarding welcome" and "Onboarding · find the box, connect, pick,
/// Wi-Fi"): cream, the 3 px outline, a 40 pt strip on top for the window buttons, the page, and the step chips.
public struct KorekoreOnboardingFrame<Page: View>: View {
    let steps: [String]
    let current: Int
    /// Going back to a step already done; nil while it mustn't (a flash in progress).
    let goTo: ((Int) -> Void)?
    let page: Page

    public init(steps: [String], current: Int, goTo: ((Int) -> Void)?, @ViewBuilder page: () -> Page) {
        self.steps = steps
        self.current = current
        self.goTo = goTo
        self.page = page()
    }

    public var body: some View {
        VStack(spacing: 0) {
            Color.clear.frame(height: 40)
            page.frame(maxWidth: .infinity, maxHeight: .infinity)
            HStack(spacing: 8) {
                ForEach(steps.indices, id: \.self) { index in
                    chip(index)
                }
            }
            .padding(.top, 22)
            .padding(.bottom, 24)
        }
        .foregroundStyle(Theme.Colors.ink)
        .sticker(radius: Theme.Radius.lg, fill: Theme.Colors.cream, stroke: Theme.Stroke.bold, shadow: Theme.Sticker.lg)
        .padding(.bottom, Theme.Sticker.lg)
    }

    /// Done in `mint`, current in ink, the rest white. Done ones take you back.
    private func chip(_ index: Int) -> some View {
        let done = index < current, on = index == current
        let label = Text(verbatim: steps[index])
            .font(.rounded(12, weight: 700))
            .padding(.vertical, 3)
            .padding(.horizontal, 10)
            .foregroundStyle(on ? Theme.Colors.cream : Theme.Colors.ink)
            .background(on ? Theme.Colors.ink : done ? Theme.Colors.mint : Theme.Colors.paper, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
        return Group {
            if done, let goTo {
                Button { goTo(index) } label: { label }.buttonStyle(.plain)
            } else {
                label
            }
        }
    }
}

/// The large pill onboarding pages lead with: 15/800 (17 on the welcome page), a 3 px outline and the small shadow
/// when it is the primary action.
public struct KBigButtonStyle: ButtonStyle {
    var fill: KFill
    var primary: Bool
    var size: CGFloat

    public init(_ fill: KFill = .sun, primary: Bool = true, size: CGFloat = 15) {
        self.fill = fill
        self.primary = primary
        self.size = size
    }

    public func makeBody(configuration: Configuration) -> some View {
        BigLabel(configuration: configuration, fill: fill, primary: primary, size: size)
    }

    private struct BigLabel: View {
        let configuration: Configuration
        let fill: KFill
        let primary: Bool
        let size: CGFloat
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            configuration.label
                .font(.rounded(size))
                .lineLimit(1)
                .padding(.vertical, size > 15 ? 12 : 10)
                .padding(.horizontal, primary ? (size > 15 ? 28 : 22) : 18)
                .foregroundStyle(fill.text)
                .background(fill.background, in: Capsule())
                .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: primary ? Theme.Stroke.bold : Theme.Stroke.base))
                .background(Capsule().fill(Theme.Colors.ink).offset(y: primary ? Theme.Sticker.sm : 0))
                .padding(.bottom, primary ? Theme.Sticker.sm : 0)
                .opacity(enabled ? (configuration.isPressed ? 0.85 : 1) : 0.45)
                .contentShape(Capsule())
        }
    }
}

/// The confirmations in the korekore look (canvas: "CONFIRMATIONS"): a cream sheet with the 3 px outline and the
/// medium shadow, a title, a note, an optional checkbox and the buttons, the last one the action. Shown modally like
/// `NSAlert.runModal`, so it drops into the same places.
@MainActor
public enum KorekoreAlert {
    public struct Checkbox {
        public var label: String
        public var isOn: Bool

        public init(label: String, isOn: Bool) {
            self.label = label
            self.isOn = isOn
        }
    }

    /// The index of the button pressed (0 is the first in `buttons`), and the checkbox's final state. `warning`
    /// draws the outline and the action in `kore-pink`, for what can't be undone.
    public static func run(title: String, message: String?, buttons: [String], checkbox: Checkbox? = nil,
                           warning: Bool = false, facts: [(String, String)] = []) -> (button: Int, checked: Bool) {
        let state = AlertState(checked: checkbox?.isOn ?? false)
        let panel = KeyPanel(contentRect: NSRect(x: 0, y: 0, width: 460, height: 200), styleMask: [.borderless],
                             backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isMovableByWindowBackground = true
        let view = AlertView(title: title, message: message, buttons: buttons, checkboxLabel: checkbox?.label,
                             warning: warning, facts: facts, state: state) { index in
            state.result = index
            NSApp.stopModal()
        }
        let hosting = NSHostingView(rootView: view)
        hosting.frame.size = hosting.fittingSize
        panel.setContentSize(hosting.fittingSize)
        panel.contentView = hosting
        panel.center()
        NSApp.activate(ignoringOtherApps: true)
        NSApp.runModal(for: panel)
        panel.orderOut(nil)
        return (state.result, state.checked)
    }

    /// The sheet as `run` shows it, for snapshots.
    public static func preview(title: String, message: String?, buttons: [String], checkbox: Checkbox? = nil,
                               warning: Bool = false, facts: [(String, String)] = []) -> some View {
        AlertView(title: title, message: message, buttons: buttons, checkboxLabel: checkbox?.label, warning: warning, facts: facts,
                  state: AlertState(checked: checkbox?.isOn ?? false)) { _ in }
    }

    /// A borderless panel can't become key by default, and then the modal session drops every click and key press.
    final class KeyPanel: NSPanel {
        override var canBecomeKey: Bool { true }
    }

    final class AlertState: ObservableObject {
        @Published var checked: Bool
        var result = 0

        init(checked: Bool) {
            self.checked = checked
        }
    }

    struct AlertView: View {
        let title: String
        let message: String?
        let buttons: [String]
        let checkboxLabel: String?
        let warning: Bool
        let facts: [(String, String)]
        @ObservedObject var state: AlertState
        let press: (Int) -> Void

        var body: some View {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 10) {
                    Sprite(name: warning ? "kore-sad" : "kore-input")
                        .padding(.vertical, 3)
                        .padding(.horizontal, 5)
                        .background(Theme.Colors.korePink, in: RoundedRectangle(cornerRadius: 10))
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                    Text(verbatim: title).font(.rounded(16)).foregroundStyle(warning ? Theme.Colors.korePinkText : Theme.Colors.ink)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !facts.isEmpty {
                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 4) {
                        ForEach(facts.indices, id: \.self) { index in
                            GridRow {
                                KTag(facts[index].0, color: Theme.Colors.muted2)
                                KTag(facts[index].1, color: Theme.Colors.ink)
                            }
                        }
                    }
                }
                if let message { KNote(message) }
                if let checkboxLabel {
                    Toggle(isOn: $state.checked) {
                        Text(verbatim: checkboxLabel).font(.rounded(13, weight: 400))
                    }
                    .toggleStyle(.checkbox)
                    .tint(Theme.Colors.korePink)
                }
                HStack(spacing: 10) {
                    Spacer()
                    // Buttons come in NSAlert's order (action first); the action goes last, on the right.
                    ForEach(Array(buttons.indices.reversed()), id: \.self) { index in
                        Button(buttons[index]) { press(index) }
                            .buttonStyle(KButtonStyle(index == 0 ? (warning ? .pink : .sun) : .paper, size: 12))
                            .keyboardShortcut(index == 0 ? .defaultAction : index == buttons.count - 1 ? .cancelAction : nil)
                    }
                }
            }
            .foregroundStyle(Theme.Colors.ink)
            .padding(EdgeInsets(top: 16, leading: 18, bottom: 16, trailing: 18))
            .frame(width: 440, alignment: .leading)
            .sticker(radius: 18, fill: Theme.Colors.cream, stroke: Theme.Stroke.bold,
                     outline: warning ? Theme.Colors.korePinkText : Theme.Colors.ink, shadow: Theme.Sticker.md)
            .padding(EdgeInsets(top: 4, leading: 10, bottom: Theme.Sticker.md + 10, trailing: 10))
        }
    }
}
