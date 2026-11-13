import Foundation

/// The company edition a box is of (ADR-0025), as `/v1/status` reports it in `device.edition`.
public struct DeviceEdition: Codable, Equatable {
    public var id: String
    public init(id: String) { self.id = id }
}

/// The company Character's card, from `GET /v1/device/edition` (docs/protocol.md). The App shows it first in the
/// Character list.
public struct EditionCard: Codable, Equatable {
    public var id: String
    public var name: String
    public var summary: String
    /// The look (docs/characters.md) as base64; nil for a Character without one.
    public var look: String?

    public init(id: String, name: String, summary: String, look: String? = nil) {
        self.id = id; self.name = name; self.summary = summary; self.look = look
    }

    /// The look's bytes; nil when there is none or it isn't base64.
    public var lookData: Data? { look.flatMap { Data(base64Encoded: $0) } }

    /// The edition whose card to ask the daemon for now: the connected box's, when no card of it is kept and none is
    /// being asked for. Nil otherwise.
    public static func toAsk(device: DeviceState?, kept: EditionCard?, asking: String?) -> String? {
        guard let device, device.connected, let id = device.edition?.id, kept?.id != id, asking != id else { return nil }
        return id
    }

    /// The card to show for this box: the kept one when it is of the box's edition, else none. An ordinary box shows
    /// none.
    public static func shown(_ kept: EditionCard?, device: DeviceState?) -> EditionCard? {
        guard let kept, device?.connected == true, device?.edition?.id == kept.id else { return nil }
        return kept
    }
}
