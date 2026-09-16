import SwiftUI
import VibeBuddyCore

/// 首次引导：欢迎 → 找设备 → 接入 → 选音色 → 登录时启动 → 完成。
struct OnboardingView: View {
    @ObservedObject var model: AppModel
    let finish: () -> Void
    @State private var step = 0
    @State private var pendingPlan: HookInstaller.Plan?

    private let titles = ["欢迎", "找设备", "接入 Agent", "选音色", "登录时启动", "完成"]

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                ForEach(titles.indices, id: \.self) { index in
                    Text(titles[index]).font(.caption).foregroundStyle(index == step ? .primary : .secondary)
                    if index + 1 < titles.count { Text("›").foregroundStyle(.tertiary) }
                }
            }
            Divider()
            Group {
                switch step {
                case 0: welcome
                case 1: findDevice
                case 2: hooks
                case 3: voices
                case 4: loginItem
                default: done
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            Divider()
            HStack {
                if step > 0 && step < 5 { Button("上一步") { step -= 1 } }
                Spacer()
                if step == 1 || step == 3 { Button("跳过") { step += 1 } }
                if step < 5 {
                    Button(step == 2 && !anyHookInstalled ? "先不接" : "下一步") { step += 1 }
                        .keyboardShortcut(.defaultAction)
                } else {
                    Button("开始使用", action: finish).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(24)
        .frame(width: 560, height: 440)
        .sheet(item: Binding(get: { pendingPlan.map(PlanBox.init) }, set: { pendingPlan = $0?.plan })) { box in
            PlanSheet(plan: box.plan, confirm: { model.applyHookPlan(box.plan); pendingPlan = nil }, cancel: { pendingPlan = nil })
        }
    }

    private var anyHookInstalled: Bool { model.hookInstalled.values.contains(true) }

    private var welcome: some View {
        HStack(alignment: .top, spacing: 24) {
            Image(nsImage: PixelFace.largeImage()).interpolation(.none)
            VStack(alignment: .leading, spacing: 10) {
                Text("Vibe Buddy 把 Codex 和 Claude Code 的状态变成盒子上的画面和声音。").font(.title3)
                Text("接下来几步：插上盒子，接入 Agent，挑一个播报音色，决定要不要开机就启动。都能改，都能跳过。")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var findDevice: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("把盒子用 USB 线插到这台 Mac。").font(.title3)
            if let device = model.status?.device, device.connected {
                Label("找到了：\(device.port ?? "")", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                Button("让它眨一下眼") { model.identify() }
                Text("盒子背光闪了就是它。").font(.caption).foregroundStyle(.secondary)
            } else {
                Label(model.daemonAlive ? "还没找到盒子，插上后这里会变。" : "daemon 正在启动…", systemImage: "cable.connector")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var hooks: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("接入 Agent：让它们的进展传到盒子上。").font(.title3)
            ForEach(HookAgent.allCases, id: \.rawValue) { agent in
                HookRow(model: model, agent: agent, pendingPlan: $pendingPlan)
            }
            Text("只转发会话标识、事件名和工作目录。Codex 写入后要在它的 /hooks 页面信任一次。").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var voices: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("挑一个播报音色。").font(.title3)
            ScrollView {
                VStack(spacing: 6) {
                    ForEach(VoiceCatalogEntry.all) { entry in VoiceCard(model: model, entry: entry) }
                }
            }
            if let operation = model.operation, operation.kind == .voicePack { OperationRow(operation: operation) }
            Text("不选就用盒子内置的湾湾小何。").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var loginItem: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("登录时自动启动 Vibe Buddy？").font(.title3)
            Toggle("登录时启动", isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
            Text("系统可能会提示「已添加后台项目」，那是 macOS 的正常提示。").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var done: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("好了。").font(.title3)
            Text("Vibe Buddy 住在菜单栏里：图标是氛围小助手的脸，盒子在线时睁着眼。想改什么，菜单里的「设置…」。")
                .foregroundStyle(.secondary)
        }
    }
}
