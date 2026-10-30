import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The Advanced page (canvas: "Mac · Advanced").
struct KorekoreAdvancedPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                daemon
                HStack(alignment: .top, spacing: 14) {
                    card(String(localized: "Logs"), String(localized: "The daemon's and the app's logs, as files."),
                         button: String(localized: "Open logs folder")) {
                        // A menu bar app has no Dock icon, so Finder could open behind; selecting the file brings it forward.
                        NSWorkspace.shared.activateFileViewerSelecting([Resources.logsDirectory.appendingPathComponent("vibebuddyd.log")])
                    }
                    card(String(localized: "Diagnostics"), String(localized: "Logs, config and both sides' build IDs in one file, for a bug report."),
                         button: String(localized: "Export diagnostics…"), action: exportDiagnostics)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text("Config file").font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    Text(verbatim: Resources.applicationSupport.appendingPathComponent("config.json").path
                        .replacingOccurrences(of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~"))
                        .font(.pixel(13))
                        .textSelection(.enabled)
                }
                .padding(.vertical, 10)
                .padding(.horizontal, 14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(Theme.Colors.mistText, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
            }
            .padding(.bottom, Theme.Sticker.sm + 4)
        }
        .scrollIndicators(.never)
    }

    /// The daemon on an Ink card, as the box would show it.
    private var daemon: some View {
        KCard(padding: EdgeInsets(top: 16, leading: 16, bottom: 16, trailing: 16), fill: Theme.Colors.ink) {
            HStack(spacing: 16) {
                Sprite(name: model.daemonAlive ? "kore-working" : "kore-sad", scale: 3)
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: "DAEMON").font(.pixel(13)).foregroundStyle(Theme.Colors.boxTextMuted)
                    if model.daemonAlive {
                        (Text(String(localized: "Running").uppercased()).foregroundColor(Theme.Colors.mint)
                            + Text(verbatim: " · \(model.status?.daemon.shortBuild ?? "")"))
                            .font(.pixel(16))
                    } else {
                        Text(String(localized: "Not running").uppercased()).font(.pixel(16)).foregroundStyle(Theme.Colors.peach)
                    }
                    Text(verbatim: "127.0.0.1:7331").font(.pixel(12)).foregroundStyle(Theme.Colors.boxTextMuted)
                }
                .foregroundStyle(Theme.Colors.cream)
                .frame(maxWidth: .infinity, alignment: .leading)
                Button(String(localized: "Restart daemon")) { model.restartDaemon() }
                    .buttonStyle(KButtonStyle(.sun, size: 13, outline: Theme.Colors.cream))
            }
        }
    }

    private func card(_ title: String, _ note: String, button: String, action: @escaping () -> Void) -> some View {
        KCard {
            VStack(alignment: .leading, spacing: 8) {
                KHeading(title)
                Text(verbatim: note).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                Button(button, action: action).kButton()
            }
            .frame(minHeight: 96, alignment: .topLeading)
        }
    }

    /// Logs, config and both sides' build IDs in one folder, with no hook payloads.
    private func exportDiagnostics() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "vibe-buddy-diagnostics"
        panel.canCreateDirectories = true
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window) { response in
                guard response == .OK, let target = panel.url else { return }
                writeDiagnostics(to: target)
            }
        } else {
            NSApp.activate(ignoringOtherApps: true)
            guard panel.runModal() == .OK, let target = panel.url else { return }
            writeDiagnostics(to: target)
        }
    }

    private func writeDiagnostics(to target: URL) {
        let manager = FileManager.default
        try? manager.createDirectory(at: target, withIntermediateDirectories: true)
        for name in ["vibebuddyd.log", "codex-hooks.log"] {
            let source = Resources.logsDirectory.appendingPathComponent(name)
            if manager.fileExists(atPath: source.path) { try? manager.copyItem(at: source, to: target.appendingPathComponent(name)) }
        }
        if let config = model.status?.config, let data = try? StatusCoding.encoder().encode(config) {
            try? data.write(to: target.appendingPathComponent("config.json"))
        }
        try? model.diagnosticsSummary.write(to: target.appendingPathComponent("summary.txt"), atomically: true, encoding: .utf8)
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }
}
