import Foundation

/// Supervises the bundled vibebuddyd: starts it, restarts it after a crash with 1, 2, 5 second backoff, and gives up after three failures in a row.
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

    /// The user clicked Restart: not a failure, so the count starts over.
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
        // The daemon watches this pid, so even if the app is killed (SIGKILL) it exits within two seconds
        // instead of lingering as an orphan holding the port.
        environment["VIBEBUDDY_PARENT_PID"] = String(ProcessInfo.processInfo.processIdentifier)
        process.environment = environment
        // Logs go where they did in the LaunchAgent era, so "Open logs folder" finds them.
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
        // Exit code 0 means the daemon quit at the app's request (POST /v1/daemon/restart);
        // relaunch right away and don't count it as a failure.
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
