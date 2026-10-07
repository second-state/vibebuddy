import Foundation
import Sparkle

/// The App's own updates through Sparkle (ADR-0010): it reads the appcast, verifies the download's EdDSA signature
/// against the key in Info.plist and swaps the bundle, always behind its own dialog, never silently. It only starts
/// when the build knows both the appcast and the key; otherwise the General tab's download link is all there is.
@MainActor
final class Updater: NSObject, SPUUpdaterDelegate {
    /// Where the appcast lives, next to the update manifest; `VIBEBUDDY_APPCAST_URL` overrides it for testing.
    private nonisolated static let appcastURL: String? = "https://updates.korekore.ai/vibebuddy/appcast.xml"

    private nonisolated static var feed: String? {
        ProcessInfo.processInfo.environment["VIBEBUDDY_APPCAST_URL"].flatMap { $0.isEmpty ? nil : $0 } ?? appcastURL
    }

    private var controller: SPUStandardUpdaterController?

    /// Whether Sparkle runs in this build.
    var available: Bool { controller != nil }

    override init() {
        super.init()
        let key = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String
        guard Self.feed != nil, let key, !key.isEmpty else { return }
        controller = SPUStandardUpdaterController(startingUpdater: false, updaterDelegate: self, userDriverDelegate: nil)
        // The schedule follows the daemon's switch (follow(checks:)); until the first status says, nothing is checked.
        controller?.updater.automaticallyChecksForUpdates = false
        controller?.startUpdater()
    }

    nonisolated func feedURLString(for updater: SPUUpdater) -> String? { Self.feed }

    /// One switch for both: when the daemon stops checking the manifest, Sparkle stops checking the appcast.
    func follow(checks enabled: Bool) {
        guard let updater = controller?.updater, updater.automaticallyChecksForUpdates != enabled else { return }
        updater.automaticallyChecksForUpdates = enabled
    }

    /// Sparkle's own window: what's new, then install, remind me later or skip.
    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}
