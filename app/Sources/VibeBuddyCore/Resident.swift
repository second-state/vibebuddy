import Foundation

/// `GET /v1/resident`: whether the daemon runs the experimental resident session (`VIBEBUDDY_RESIDENT_SESSION=1`).
public struct ResidentInfo: Decodable, Equatable {
    public var enabled: Bool
    public var threadId: String?

    enum CodingKeys: String, CodingKey {
        case enabled
        case threadId = "thread_id"
    }
}

/// One voice command and the buddy's spoken replies to it, as `GET /v1/resident/history` reads them from the thread.
public struct ResidentExchange: Decodable, Equatable {
    public var said: String
    public var replies: [String]

    public init(said: String, replies: [String]) {
        self.said = said
        self.replies = replies
    }
}

/// `GET /v1/resident/history`: oldest first, so the newest exchange is the last one.
public struct ResidentHistory: Decodable, Equatable {
    public var threadId: String?
    public var exchanges: [ResidentExchange]

    enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case exchanges
    }
}
