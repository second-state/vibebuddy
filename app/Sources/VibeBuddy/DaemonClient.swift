import Foundation
import VibeBuddyCore

/// Talks to the daemon over local HTTP only; the status stream is SSE.
struct DaemonClient {
    let base = URL(string: "http://127.0.0.1:7331")!

    private func request(_ path: String, method: String = "GET", body: Data? = nil, contentType: String? = nil) -> URLRequest {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = method
        request.httpBody = body
        request.timeoutInterval = 10
        if let contentType { request.setValue(contentType, forHTTPHeaderField: "Content-Type") }
        return request
    }

    func status() async throws -> Status {
        let (data, _) = try await URLSession.shared.data(for: request("/v1/status"))
        return try StatusCoding.decoder().decode(Status.self, from: data)
    }

    /// Reads the status stream, calling back with each snapshot; throws when it drops, and the caller decides when to reconnect.
    func stream(onStatus: @escaping (Status) -> Void) async throws {
        var request = self.request("/v1/status/stream")
        request.timeoutInterval = 3600 * 24
        let (bytes, response) = try await URLSession.shared.bytes(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
        let decoder = StatusCoding.decoder()
        for try await line in bytes.lines {
            guard line.hasPrefix("data:") else { continue }
            let payload = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
            if let status = try? decoder.decode(Status.self, from: Data(payload.utf8)) {
                onStatus(status)
            }
        }
        throw URLError(.networkConnectionLost)
    }

    func putConfig(_ config: DaemonConfig) async throws -> DaemonConfig {
        let body = try StatusCoding.encoder().encode(config)
        let (data, _) = try await URLSession.shared.data(for: request("/v1/config", method: "PUT", body: body, contentType: "application/json"))
        return try StatusCoding.decoder().decode(DaemonConfig.self, from: data)
    }

    @discardableResult
    private func post(_ path: String, body: Data? = nil, contentType: String? = nil) async throws -> (Int, Data) {
        let (data, response) = try await URLSession.shared.data(for: request(path, method: "POST", body: body, contentType: contentType))
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
    }

    func identify() async throws { try await post("/v1/device/identify") }

    /// The box reports a VOLUME line once applied, which is when the volume in the status stream updates; preview makes it play a line at the new volume.
    func setVolume(_ level: Int, preview: Bool) async throws {
        let body = try JSONSerialization.data(withJSONObject: ["level": level, "preview": preview])
        let (code, data) = try await post("/v1/device/volume", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Volume change was rejected (\(code))")) }
    }

    /// The daemon remembers the choice and connects to that device on its next look around.
    func chooseBox(usbSerial: String) async throws {
        let body = try JSONSerialization.data(withJSONObject: ["usb_serial": usbSerial])
        let (code, data) = try await post("/v1/device/choice", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Choosing the box was rejected (\(code))")) }
    }

    /// Forgets another computer on the box; the daemon refuses this computer, which pairs again whenever it's plugged in.
    func unpair(key: String) async throws {
        let body = try JSONSerialization.data(withJSONObject: ["key": key])
        let (code, data) = try await post("/v1/device/unpair", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Removing the computer was rejected (\(code))")) }
    }

    /// Takes the box from the computer that has it; the daemon's next try over the network or the Relay asks outright.
    func takeOver() async throws {
        let (code, data) = try await post("/v1/device/takeover", body: Data(), contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Taking the box over was rejected (\(code))")) }
    }

    /// The password goes to the box over the cable and is kept nowhere else.
    func setWifi(ssid: String, password: String) async throws {
        let body = try JSONSerialization.data(withJSONObject: ["ssid": ssid, "password": password])
        let (code, data) = try await post("/v1/device/wifi", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Setting Wi-Fi was rejected (\(code))")) }
    }

    /// The box scans; what it hears arrives in the status stream as `wifiSeen`.
    func scanWifi() async throws {
        let (code, data) = try await post("/v1/device/wifi/scan", body: Data(), contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Scanning for networks was rejected (\(code))")) }
    }

    func forgetWifi() async throws {
        let (code, data) = try await post("/v1/device/wifi/forget", body: Data(), contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Forgetting Wi-Fi was rejected (\(code))")) }
    }

    /// `lender` (the Character lending a custom one its voice, persona and lines) and `address` (the form of
    /// address its lines carry) aren't in the pack; the daemon keeps them for the buddy's spoken replies.
    func writeVoicePack(_ pack: Data, lender: String? = nil, address: String? = nil) async throws {
        var request = self.request("/v1/device/voice-pack", method: "POST", body: pack, contentType: "application/octet-stream")
        let query = [("lender", lender), ("address", address)].compactMap { name, value in value.map { URLQueryItem(name: name, value: $0) } }
        if !query.isEmpty, let url = request.url, var components = URLComponents(url: url, resolvingAgainstBaseURL: false) {
            components.queryItems = query
            request.url = components.url
        }
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Voice pack write was rejected (\(code))")) }
    }

    func flashFirmware(bootloader: URL, partitionTable: URL, app: URL) async throws {
        let body = try JSONSerialization.data(withJSONObject: [
            "bootloader": bootloader.path, "partition_table": partitionTable.path, "app": app.path,
        ])
        let (code, data) = try await post("/v1/device/firmware", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Firmware flash was rejected (\(code))")) }
    }

    func screenshot() async throws -> Data {
        let (code, data) = try await post("/v1/device/screenshot")
        guard code == 200 else { throw DaemonError(message: String(data: data, encoding: .utf8) ?? String(localized: "Screenshot failed (\(code))")) }
        return data
    }

    func restart() async throws { try await post("/v1/daemon/restart") }

    /// Asks the daemon to read the update manifest now instead of at the next daily check.
    func checkForUpdates() async throws {
        let (code, data) = try await post("/v1/updates/check")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "Update check was rejected (\(code))")) }
    }

    /// Starts signing in to the VibeBuddy account (ADR-0018): the App opens the URL, and the browser comes back to the
    /// daemon, whose next status says signed in.
    func signIn() async throws -> URL {
        let (code, data) = try await post("/v1/account/sign-in")
        guard code == 200 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "The daemon answered \(code)")) }
        guard let text = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["url"] as? String, let url = URL(string: text) else {
            throw DaemonError(message: String(localized: "The daemon answered \(code)"))
        }
        return url
    }

    func signOut() async throws { try await accountCall("/v1/account/sign-out", method: "POST") }

    /// Has the daemon read today's voice use from the service; it comes back in the status.
    func refreshAccount() async throws { try await accountCall("/v1/account/refresh", method: "POST") }

    /// Deletes the account on the service; when the service doesn't, the daemon stays signed in.
    func deleteAccount() async throws { try await accountCall("/v1/account", method: "DELETE") }

    /// The daemon asks the service, which it gives 20 s, so this waits longer than the usual 10 s.
    private func accountCall(_ path: String, method: String) async throws {
        var request = request(path, method: method)
        request.timeoutInterval = 30
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "The daemon answered \(code)")) }
    }

    func resident() async throws -> ResidentInfo {
        let (data, _) = try await URLSession.shared.data(for: request("/v1/resident"))
        return try StatusCoding.decoder().decode(ResidentInfo.self, from: data)
    }

    /// The last `limit` voice commands and their spoken replies, oldest first. Slow the first time: the daemon may have to
    /// start Codex's app-server to read the thread.
    func residentHistory(limit: Int) async throws -> [ResidentExchange] {
        var request = self.request("/v1/resident/history")
        request.url = URL(string: "\(base.absoluteString)/v1/resident/history?limit=\(limit)")
        request.timeoutInterval = 60
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "The daemon answered \(code)")) }
        return try StatusCoding.decoder().decode(ResidentHistory.self, from: data).exchanges
    }

    /// Which Agent answers voice commands. Its own endpoint, not part of the config.
    func setResidentBrain(_ choice: ResidentBrainChoice) async throws -> ResidentInfo {
        let body = try JSONSerialization.data(withJSONObject: ["brain": choice.rawValue])
        return try await residentSetting("/v1/resident/brain", method: "PUT", body: body)
    }

    /// Stores the user's Anthropic API key in the daemon (nil removes it). The daemon never sends it back.
    func setAnthropicKey(_ key: String?) async throws -> ResidentInfo {
        guard let key else { return try await residentSetting("/v1/resident/anthropic-key", method: "DELETE", body: nil) }
        let body = try JSONSerialization.data(withJSONObject: ["api_key": key])
        return try await residentSetting("/v1/resident/anthropic-key", method: "PUT", body: body)
    }

    private func residentSetting(_ path: String, method: String, body: Data?) async throws -> ResidentInfo {
        let request = request(path, method: method, body: body, contentType: body == nil ? nil : "application/json")
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw DaemonError(message: DaemonClient.message(in: data) ?? String(localized: "The daemon answered \(code)")) }
        return try StatusCoding.decoder().decode(ResidentInfo.self, from: data)
    }

    private static func message(in data: Data) -> String? {
        (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["message"] as? String
    }
}

struct DaemonError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}
