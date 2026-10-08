import Foundation
import VibeBuddyCore

/// 和 daemon 说话只走本机 HTTP；状态流是 SSE。
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

    /// 一直读状态流，每收到一份快照就回调；断了就抛错，调用方决定何时重连。
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

    /// 盒子应用后回报 VOLUME 行，状态流里的音量才更新；preview 让它用新音量播一句。
    func setVolume(_ level: Int, preview: Bool) async throws {
        let body = try JSONSerialization.data(withJSONObject: ["level": level, "preview": preview])
        let (code, data) = try await post("/v1/device/volume", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? "音量未被接受（\(code)）") }
    }

    func writeVoicePack(_ pack: Data) async throws {
        let (code, data) = try await post("/v1/device/voice-pack", body: pack, contentType: "application/octet-stream")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? "写入未被接受（\(code)）") }
    }

    func flashFirmware(bootloader: URL, partitionTable: URL, app: URL, board: String) async throws {
        let body = try JSONSerialization.data(withJSONObject: [
            "bootloader": bootloader.path, "partition_table": partitionTable.path, "app": app.path, "board": board,
        ])
        let (code, data) = try await post("/v1/device/firmware", body: body, contentType: "application/json")
        guard code == 202 else { throw DaemonError(message: DaemonClient.message(in: data) ?? "烧录未被接受（\(code)）") }
    }

    func screenshot() async throws -> Data {
        let (code, data) = try await post("/v1/device/screenshot")
        guard code == 200 else { throw DaemonError(message: String(data: data, encoding: .utf8) ?? "截图失败（\(code)）") }
        return data
    }

    func restart() async throws { try await post("/v1/daemon/restart") }

    private static func message(in data: Data) -> String? {
        (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["message"] as? String
    }
}

struct DaemonError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}
