import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

struct NativeTrajectoryView: View {
    @StateObject private var model: NativeTrajectoryModel
    let close: () -> Void
    var running = false
    @State private var query = ""
    @State private var axis = NativeTrajectoryAxis.duration
    @State private var selectedKey: String?
    @State private var inspectorOpen = true
    @State private var inspectorTab = "摘要"
    @State private var exportError: String?
    @State private var exporting = false
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, running: Bool = false, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: NativeTrajectoryModel(client: client, projectID: projectID, sessionID: sessionID))
        self.close = close; self.running = running
    }
    init(model: NativeTrajectoryModel, running: Bool = false, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: model); self.running = running; self.close = close
    }
    private var rows: [NativeTrajectoryRow] { NativeTrajectoryRow.collect(model.snapshot?.turns ?? [], query: query) }
    private var selected: NativeTrajectoryRow? { inspectorOpen ? (rows.first { $0.id == selectedKey } ?? rows.first) : nil }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                // snapshot.model is a profile id (e.g. "m1"); the WebView header shows no model.
                Text("运行轨迹").font(.title2.bold())
                Spacer()
                if model.loading { ProgressView().controlSize(.small) }
                Button("刷新") { Task { await model.refresh() } }.disabled(model.loading)
                Button(exporting ? "正在导出…" : "导出 HTML") { Task { await export() } }.disabled(exporting || model.snapshot == nil)
                Button("关闭", action: close)
            }
            HStack {
                Picker("时间轴", selection: $axis) { Text("耗时").tag(NativeTrajectoryAxis.duration); Text("轮次").tag(NativeTrajectoryAxis.turns); Text("调用").tag(NativeTrajectoryAxis.calls) }.pickerStyle(.segmented).frame(width: 240)
                TextField("搜索步骤、输入或输出", text: $query)
            }
            if let error = exportError ?? model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if let snapshot = model.snapshot {
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(snapshot.stats.turns) 轮 · \(snapshot.stats.steps) 步 · 模型 \(snapshot.stats.llm_ms) ms · 工具 \(snapshot.stats.tool_ms) ms")
                    Text("输入 \(snapshot.stats.input_tokens) · 输出 \(snapshot.stats.output_tokens) · 缓存 \(snapshot.stats.cached_input_tokens) · \(snapshot.stats.tokens_per_sec.map { String(format: "%.1f", $0) } ?? "—") tok/s · 缓存命中 \(snapshot.stats.cache_hit_pct.map { String(format: "%.0f%%", $0) } ?? "—")")
                }.font(.caption).foregroundStyle(.secondary)
            }
            ScrollViewReader { scroll in
            VStack(spacing: 12) {
                NativeTrajectoryChart(rows: rows, axis: axis, selected: selected?.id) { key in
                    selectedKey = key; inspectorOpen = true; scroll.scrollTo(key, anchor: .top)
                }
            HStack(alignment: .top, spacing: 16) {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                            if index == 0 || rows[index - 1].turn != row.turn {
                                Text("第 \(row.turn) 轮").font(.headline).padding(.top, 8)
                                if let turn = model.snapshot?.turns.first(where: { $0.index == row.turn }), let timing = NativeTrajectoryTiming.collect(turn.cells) {
                                    NativeTrajectoryTurnBar(timing: timing)
                                }
                            }
                            Button { selectedKey = row.id; inspectorOpen = true } label: {
                                VStack(alignment: .leading, spacing: 5) {
                                    HStack {
                                        WispIcon(name: row.cell.kind == "user" ? "user" : row.cell.kind == "tool" ? "wrench" : row.cell.kind == "usage" ? "gauge" : "sparkles", size: 14)
                                        Text(row.cell.kind.uppercased()).font(.caption).foregroundStyle(.secondary)
                                        Spacer()
                                        if let duration = row.cell.duration_ms { Text("\(duration) ms").font(.caption) }
                                        if row.cell.is_error || row.cell.ok == false { Text("失败").foregroundStyle(.orange) }
                                    }
                                    Text(row.cell.displaySummary).lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                                }.padding(10).background(Color.primary.opacity(selected?.id == row.id ? 0.1 : 0.035), in: RoundedRectangle(cornerRadius: 8))
                            }.buttonStyle(.plain).id(row.id)
                        }
                        if rows.isEmpty { Text(model.loading ? "正在读取轨迹…" : "没有匹配的轨迹记录").foregroundStyle(.secondary).padding(30) }
                    }
                }
                if let selected {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack { Text("步骤详情").font(.headline); Spacer(); Button("关闭详情") { inspectorOpen = false } }
                        Picker("详情", selection: $inspectorTab) { ForEach(["摘要", "预览", "原始", "来源"], id: \.self) { Text($0) } }.pickerStyle(.segmented)
                        ScrollView {
                            VStack(alignment: .leading, spacing: 12) {
                                if inspectorTab == "原始" {
                                    Text(selected.cell.rawJSON).font(.system(.caption, design: .monospaced))
                                } else if inspectorTab == "来源" {
                                    Text(selected.cell.source).font(.system(.caption, design: .monospaced))
                                } else {

                                Text(selected.cell.displaySummary).font(.headline)
                                if inspectorTab == "摘要" {
                                    Text("来源：\(localized(selected.cell.sourceLabel)) · \(status(selected))").font(.caption)
                                    Text("耗时：" + (selected.cell.duration_ms.map { "\(max(0, $0)) ms" } ?? "—")).font(.caption)
                                }
                                if let timestamp = selected.cell.ts { Text(Date(timeIntervalSince1970: Double(timestamp) / 1000), style: .time) }
                                if inspectorTab == "预览" && selected.cell.kind == "tool" {
                                    if let input = selected.cell.detail_input { Text("输入").font(.headline); Text(input).font(.system(.caption, design: .monospaced)) }
                                    if let output = selected.cell.detail_output { Text("输出").font(.headline); Text(output).font(.system(.caption, design: .monospaced)) }
                                } else { Text(selected.cell.preview).font(.system(.caption, design: .monospaced)) }
                                if let usage = selected.cell.usage {
                                    Text("模型：\(usage.model ?? "—")\n输入：\(usage.input_tokens)\n输出：\(usage.output_tokens)\n推理：\(usage.reasoning_tokens)\n缓存：\(usage.cached_input_tokens)")
                                }
                                }
                            }.frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
                        }
                    }.frame(width: 300)
                }
            }
            }
            }
        }.padding(20).frame(minWidth: 640, idealWidth: 980, minHeight: 420, idealHeight: 650)
            .background(NativeSettingsEscape { if selected != nil { inspectorOpen = false } else { close() } })
            .task {
                while !Task.isCancelled {
                    await model.refresh()
                    do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
                }
            }
            .onChange(of: rows.map(\.id)) { keys in
                if !keys.contains(selectedKey ?? "") { selectedKey = keys.first }
            }
            .onDisappear { model.close() }
    }
    private func status(_ row: NativeTrajectoryRow) -> String {
        let value = row.cell.status(running: running && row.turn == model.snapshot?.turns.last?.index)
        return ["error": "失败", "pending": "待完成", "running": "运行中", "completed": "已完成"][value] ?? value
    }
    private func export() async {
        exporting = true; exportError = nil
        defer { exporting = false }
        do {
            let html = try await model.exportHTML()
            let panel = NSSavePanel(); panel.allowedContentTypes = [.html]; panel.nameFieldStringValue = "trajectory.html"
            let response = await panel.begin()
            guard response == .OK, let url = panel.url else { return }
            try html.write(to: url, atomically: true, encoding: .utf8)
        } catch { exportError = error.localizedDescription }
    }
}
