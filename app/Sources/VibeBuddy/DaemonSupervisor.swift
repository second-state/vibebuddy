import Foundation

/// 看管包内的 vibebuddyd：拉起、崩了按 1、2、5 秒退避重启，连续三次失败停手。
@MainActor
final class DaemonSupervisor {
    enum State: Equatable { case stopped, running, givenUp }

    private(set) var state: State = .stopped { didSet { onStateChange?(state) } }
    var onStateChange: ((State) -> Void)?
    var onGiveUp: (() -> Void)?

    private var process: Process?
    private var failures = 0
    private var stopping = false
    private let backoff: [TimeInterval] = [1, 2, 5]

    func start() {
        stopping = false
        failures = 0
        launch()
    }

    /// 用户点了「重启」：不算失败，重新计数。
    func restart() {
        failures = 0
        if let process, process.isRunning {
            stopping = true
            process.terminate()
            process.waitUntilExit()
        }
        stopping = false
        launch()
    }

    func stop() {
        stopping = true
        if let process, process.isRunning {
            process.terminate()
            process.waitUntilExit()
        }
        process = nil
        state = .stopped
    }

    private func launch() {
        let process = Process()
        process.executableURL = Resources.daemonBinary
        var environment = ProcessInfo.processInfo.environment
        environment["VIBEBUDDY_APP_VERSION"] = Resources.bundleVersion
        // daemon 看着这个 pid：App 被强杀（SIGKILL）时它也能在两秒内退出，
        // 不会变成孤儿占着端口。
        environment["VIBEBUDDY_PARENT_PID"] = String(ProcessInfo.processInfo.processIdentifier)
        process.environment = environment
        // 日志与 LaunchAgent 时代同一个位置，「打开日志」就能找到。
        try? FileManager.default.createDirectory(at: Resources.logsDirectory, withIntermediateDirectories: true)
        let logURL = Resources.logsDirectory.appendingPathComponent("vibebuddyd.log")
        if !FileManager.default.fileExists(atPath: logURL.path) {
            FileManager.default.createFile(atPath: logURL.path, contents: nil)
        }
        if let handle = try? FileHandle(forWritingTo: logURL) {
            handle.seekToEndOfFile()
            process.standardOutput = handle
            process.standardError = handle
        }
        process.terminationHandler = { [weak self] finished in
            Task { @MainActor in self?.handleExit(status: finished.terminationStatus) }
        }
        do {
            try process.run()
            self.process = process
            state = .running
        } catch {
            handleExit(status: -1)
        }
    }

    private func handleExit(status: Int32) {
        process = nil
        if stopping { return }
        // 退出码 0 是 daemon 应 App 的要求主动退出（POST /v1/daemon/restart），
        // 立刻拉起，不算失败。
        if status == 0 {
            launch()
            return
        }
        failures += 1
        if failures > backoff.count {
            state = .givenUp
            onGiveUp?()
            return
        }
        let delay = backoff[failures - 1]
        state = .stopped
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, !self.stopping, self.process == nil else { return }
            self.launch()
        }
    }
}
