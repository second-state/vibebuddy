import AppKit
import KorekoreUI
import VibeBuddyCore

/// Asking before flashing, in the korekore look (canvas: "CONFIRMATIONS"): before an update, and before
/// flashing over firmware that isn't ours.
@MainActor
enum KorekoreFlash {
    static func update(_ offer: FirmwareOffer, then flash: @escaping @MainActor () -> Void) {
        var note = String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        if let notes = Firmware.notes(offer.notes, chinese: Resources.uiLanguage == .zh) { note = notes + "\n\n" + note }
        KorekoreAlert.ask(title: String(localized: "Update the box firmware to \(offer.version)?"), message: note,
                          buttons: [String(localized: "Update"), String(localized: "Not now")]) { answer in
            if answer.button == 0 { flash() }
        }
    }

    static func foreign(then flash: @escaping @MainActor () -> Void) {
        KorekoreAlert.ask(
            title: String(localized: "Flash the box with korekore?"),
            message: String(localized: "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes."),
            buttons: [String(localized: "Flash"), String(localized: "Cancel")], warning: true) { answer in
            if answer.button == 0 { flash() }
        }
    }
}
