import Foundation

/// The app's bundle file was `Vibe Buddy.app` until 2026-10-09 and is `VibeBuddy.app` since, matching the product
/// name. Sparkle updates a bundle in place and keeps its file name, and a new DMG brings the new name next to an old
/// copy, so an installed copy sorts this out itself when it starts. Only copies in an Applications folder are
/// touched; a development build elsewhere is left alone.
public enum BundleName {
    public static let current = "VibeBuddy.app"
    public static let old = "Vibe Buddy.app"

    public enum Step: Equatable {
        case none
        /// This copy has the old name and nothing is in the way: rename it, then start it again from there.
        case rename(to: URL)
        /// This copy has the old name and a copy with the new one is already there: that one wins.
        case replaceWith(URL)
        /// This copy has the new name and an old one is still next to it.
        case removeOld(URL)
    }

    public static func step(for bundle: URL, exists: (URL) -> Bool) -> Step {
        let folder = bundle.deletingLastPathComponent()
        guard folder.lastPathComponent == "Applications" else { return .none }
        switch bundle.lastPathComponent {
        case old:
            let target = folder.appendingPathComponent(current)
            return exists(target) ? .replaceWith(target) : .rename(to: target)
        case current:
            let stale = folder.appendingPathComponent(old)
            return exists(stale) ? .removeOld(stale) : .none
        default:
            return .none
        }
    }
}
