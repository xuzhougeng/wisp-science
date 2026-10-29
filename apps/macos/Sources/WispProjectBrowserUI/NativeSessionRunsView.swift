import SwiftUI
import WispProjectBrowser

/// The conversation's runs, split like the WebView 运行 tab (`session_run_groups`).
enum NativeSessionRuns {
    /// `is_active_run_status` in ui/src/app_support/runtime.rs.
    static func isActive(_ status: String) -> Bool { ["submitted", "running", "cancelling", "paused"].contains(status) }
    /// Active newest-submitted first; everything else most recently ended first.
    static func group(_ runs: [NativeRun]) -> (active: [NativeRun], completed: [NativeRun]) {
        let active = runs.filter { isActive($0.status) }.sorted { ($0.created_at, $0.id) > ($1.created_at, $1.id) }
        let completed = runs.filter { !isActive($0.status) }.sorted {
            ($0.ended_at ?? $0.created_at, $0.created_at, $0.id) > ($1.ended_at ?? $1.created_at, $1.created_at, $1.id)
        }
        return (active, completed)
    }
    /// `runs.status.*` in ui/src/i18n.rs.
    static func statusLabel(_ status: String) -> String {
        localized(["submitted": "排队中", "draft": "等待审批", "running": "运行中", "paused": "已暂停", "cancelling": "正在取消",
                   "succeeded": "已完成", "failed": "失败", "timed_out": "已超时", "cancelled": "已取消", "lost": "已失联"][status] ?? "状态不可用")
    }
    /// The tab title counts only runs still working, like `tab_count(right.runs)`.
    static func tabTitle(_ runs: [NativeRun]) -> String {
        let active = runs.filter { isActive($0.status) }.count
        return active == 0 ? localized("运行") : localized("运行 ({n})").replacingOccurrences(of: "{n}", with: String(active))
    }
}

/// Right panel 运行: this conversation's background runs and transfers, with the
/// existing detail, cancel and re-harvest actions. Polls every 2 s while shown.
struct NativeSessionRunsView: View {
    @ObservedObject var model: NativeContextActivityModel
    @State private var cancelID: String?
    @State private var completedOpen = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let groups = NativeSessionRuns.group(model.runs)
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                if model.runs.isEmpty && !model.loading {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(localized("本会话暂无运行")).font(.headline)
                        Text(localized("本会话启动的后台运行和文件传输会显示在这里，无需翻看对话即可查看状态。")).font(.caption).foregroundStyle(.secondary)
                    }.padding(.vertical, 20)
                } else {
                    HStack {
                        Text(localized("进行中")).font(.headline)
                        Text("\(groups.active.count)").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button { Task { await model.refresh() } } label: { WispIcon(name: "refresh", size: 14) }
                            .buttonStyle(.plain).help(localized("刷新运行记录")).accessibilityLabel(localized("刷新运行记录"))
                    }
                    if groups.active.isEmpty { Text(localized("本会话当前没有进行中的运行。")).font(.caption).foregroundStyle(.secondary) }
                    ForEach(groups.active) { row($0) }
                    if !groups.completed.isEmpty {
                        DisclosureGroup(isExpanded: $completedOpen) {
                            VStack(alignment: .leading, spacing: 8) { ForEach(groups.completed) { row($0) } }.padding(.top, 6)
                        } label: {
                            Text(localized("已完成 ({n})").replacingOccurrences(of: "{n}", with: String(groups.completed.count))).font(.headline)
                        }
                    }
                }
            }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
        }
        .task {
            await model.refresh()
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if !Task.isCancelled { await model.refresh() }
            }
        }
        .confirmationDialog(localized("取消运行"), isPresented: Binding(get: { cancelID != nil }, set: { if !$0 { cancelID = nil } })) {
            if let id = cancelID { Button(localized("取消运行"), role: .destructive) { cancelID = nil; Task { await model.mutateRun(id, harvest: false) } } }
            Button(localized("继续运行"), role: .cancel) { cancelID = nil }
        }
    }

    private func row(_ run: NativeRun) -> some View {
        let open = model.selectedRun == run.id
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(run.title.isEmpty ? run.id : run.title).fontWeight(.semibold).lineLimit(2)
                Spacer()
                Text(NativeSessionRuns.statusLabel(run.status)).font(.caption)
                    .padding(.horizontal, 6).padding(.vertical, 2).background(Color.primary.opacity(0.06), in: Capsule())
            }
            Text([run.kind, run.context_id, date(run.started_at ?? run.created_at)].joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
            if let error = run.last_poll_error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                Button(localized(open ? "收起" : "查看详情")) { if open { model.dismissDetail() } else { Task { await model.readRun(run.id) } } }
                if run.cancellable { Button(localized("取消运行")) { cancelID = run.id }.disabled(model.busy || model.snapshot?.read_only != false) }
                if run.harvestable { Button(localized("重新收集产物")) { Task { await model.mutateRun(run.id, harvest: true) } }.disabled(model.busy || model.snapshot?.read_only != false) }
            }.font(.caption)
            if open, let detail = model.detail, detail.id == run.id { self.detail(detail) }
        }
        .padding(10).frame(maxWidth: .infinity, alignment: .leading)
        .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityIdentifier("session-run-\(run.id)")
    }

    private func detail(_ run: NativeRun) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let code = run.exit_code { Text(localized("退出码") + " \(code)").font(.caption) }
            if let path = run.remote_workdir { Text(path).font(.caption).textSelection(.enabled) }
            if let command = run.command { output("命令", command) }
            if let stdout = run.stdout_tail, !stdout.isEmpty { output("标准输出", stdout) }
            if let stderr = run.stderr_tail, !stderr.isEmpty { output("错误输出", stderr) }
            if let error = run.cleanup_error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
        }.padding(.top, 4)
    }
    private func output(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(localized(title)).font(.caption.weight(.semibold))
            Text(text).font(.system(size: 11, design: .monospaced)).textSelection(.enabled).lineLimit(12).frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    private func date(_ seconds: Int64) -> String { Date(timeIntervalSince1970: Double(seconds)).formatted(date: .abbreviated, time: .shortened) }
}
