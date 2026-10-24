import AppKit
import KorekoreUI
import VibeBuddyCore

/// Asking before flashing, in the korekore look (canvas: "CONFIRMATIONS"). `FlashConfirm`'s two questions.
@MainActor
enum KorekoreFlash {
    static func update(_ offer: FirmwareOffer, then flash: () -> Void) {
        var note = String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        if let notes = Firmware.notes(offer.notes, chinese: Resources.uiLanguage == .zh) { note = notes + "\n\n" + note }
        let answer = KorekoreAlert.run(title: String(localized: "Update the box firmware to \(offer.version)?"), message: note,
                                       buttons: [String(localized: "Update"), String(localized: "Not now")])
        if answer.button == 0 { flash() }
    }

    static func foreign(then flash: () -> Void) {
        let answer = KorekoreAlert.run(
            title: String(localized: "Flash the box with korekore?"),
            message: String(localized: "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes."),
            buttons: [String(localized: "Flash"), String(localized: "Cancel")], warning: true)
        if answer.button == 0 { flash() }
    }
}
