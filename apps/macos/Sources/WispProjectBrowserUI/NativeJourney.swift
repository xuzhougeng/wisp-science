import SwiftUI
import WispProjectBrowser

struct JourneyPage: Codable, Equatable {
    var entries: [CalendarEntry]
    var truncated: Bool
}

struct NativeJourneyDay: Identifiable, Equatable {
    var id: Int64
    var entries: [CalendarEntry]
    var sessionActivityCounts: [String: Int]

    static func grouped(_ entries: [CalendarEntry], calendar: Calendar) -> [Self] {
        let byDay = Dictionary(grouping: NativeCalendarModel.sorted(entries)) { entry in
            NativeCalendarClock.dayInterval(containing: Date(timeIntervalSince1970: TimeInterval(entry.occurredAt)), calendar: calendar).0
        }
        return byDay.keys.sorted(by: >).map { day in
            var sessions = Set<String>()
            var counts: [String: Int] = [:]
            let rows = (byDay[day] ?? []).filter { entry in
                guard entry.kind == "session", let source = entry.sourceID, !source.isEmpty else { return true }
                counts[source, default: 0] += 1
                return sessions.insert(source).inserted
            }
            return Self(id: day, entries: rows, sessionActivityCounts: counts)
        }
    }
}

struct JourneyArtifact: Decodable {
    var versionID: String
    var filename: String
    var versionNumber: Int64
    var source: JourneyArtifactSource
    var text: String?
    var mime: String
    var base64: String?
    var truncated: Bool
    var contentError: String?
    enum CodingKeys: String, CodingKey {
        case filename, source, text, mime, base64, truncated
        case versionID = "version_id", versionNumber = "version_number", contentError = "content_error"
    }
}
struct JourneyArtifactSource: Decodable {
    var runID: String?
    var runTitle: String
    var runStatus: String
    var contextID: String
    var inputs: [JourneyArtifactInput]
    enum CodingKeys: String, CodingKey {
        case inputs
        case runID = "run_id", runTitle = "run_title", runStatus = "run_status", contextID = "context_id"
    }
}
struct JourneyArtifactInput: Decodable {
    var title: String
    var role: String
    var versionID: String?
    var confidence: String
    enum CodingKeys: String, CodingKey { case title, role, confidence; case versionID = "version_id" }
}

enum NativeJourneyCommand {
    static let read = "native_research_journey"
}

@MainActor
final class NativeJourneyModel: ObservableObject {
    @Published var presented = false
    @Published var query = ""
    @Published private(set) var projectID: String?
    @Published private(set) var day: Int64?
    @Published private(set) var entries: [CalendarEntry] = []
    @Published private(set) var recaps: [SettingsValue] = []
    @Published private(set) var graph: SettingsValue = .null
    @Published private(set) var writing = false
    @Published private(set) var uncertainProjects = Set<String>()
    private var loaded = false
    private var graphGeneration = UUID()
    var uncertain: Bool { projectID.map { uncertainProjects.contains($0) } ?? false }
    var canWrite: Bool { presented && loaded && !busy && !writing && !uncertain }
    @Published private(set) var truncated = false
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published private(set) var selectedEntry: CalendarEntry?
    @Published private(set) var artifact: JourneyArtifact?
    @Published private(set) var detailBusy = false
    @Published private(set) var detailError: String?
    private var detailGeneration = UUID()
    private var generation = UUID()
    var clock = Date()
    var calendar = Calendar.current

    func open(projectID: String, day: Int64?) {
        generation = UUID()
        self.projectID = projectID
        self.day = day
        if let day { clock = Date(timeIntervalSince1970: TimeInterval(day)) }
        entries = []; recaps = []; graph = .null; loaded = false
        graphGeneration = UUID()
        truncated = false
        error = nil
        busy = false
        query = ""
        closeDetail()
        presented = true
    }

    func dismiss() {
        invalidate()
    }

    func invalidate() {
        generation = UUID()
        presented = false
        graphGeneration = UUID(); graph = .null; loaded = false
        busy = false
        closeDetail()
    }

    func closeDetail() {
        detailGeneration = UUID()
        selectedEntry = nil
        artifact = nil
        detailError = nil
        detailBusy = false
    }

    func showDetail(_ entry: CalendarEntry, client: any NativeSettingsQuerying) async {
        guard presented, let projectID, entries.contains(where: { $0.id == entry.id }) else { return }
        closeDetail()
        selectedEntry = entry
        guard entry.kind == "artifact", let versionID = entry.sourceID else { return }
        let current = UUID()
        detailGeneration = current
        detailBusy = true
        defer { if detailGeneration == current { detailBusy = false } }
        do {
            let value = try await client.invoke("native_research_journey_artifact", args: ["version_id": .string(versionID)], projectID: projectID)
            guard detailGeneration == current, presented, self.projectID == projectID else { return }
            let page = try JSONDecoder().decode(JourneyArtifact.self, from: JSONEncoder().encode(value))
            guard page.versionID == versionID else { throw ProjectBrowserError.service("产物版本不匹配。") }
            artifact = page
        } catch {
            guard detailGeneration == current, presented, self.projectID == projectID else { return }
            detailError = error.localizedDescription
        }
    }

    func acknowledge() { guard loaded, !busy, !writing, let projectID else { return }; uncertainProjects.remove(projectID); error = nil }

    func loadGraph(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID else { return }
        let token = UUID(); graphGeneration = token; graph = .null; error = nil
        do {
            let result = try await client.invoke("native_research_journey_graph", args: [:], projectID: projectID)
            guard presented, self.projectID == projectID, graphGeneration == token else { return }
            guard case .array = result["nodes"], case .array = result["edges"] else { throw ProjectBrowserError.invalidResponse }
            graph = result
        } catch { if presented, self.projectID == projectID, graphGeneration == token { self.error = error.localizedDescription } }
    }

    @discardableResult func mutate(_ command: String, args: [String: SettingsValue], client: any NativeSettingsQuerying, savedDay: Int64? = nil, expectedProject: String? = nil) async -> Bool {
        guard canWrite, let projectID, ["native_research_journey_add", "native_research_journey_recap"].contains(command) else { return false }
        guard expectedProject == nil || expectedProject == projectID else { return false }
        let token = generation
        writing = true; uncertainProjects.insert(projectID); loaded = false; error = nil
        if let savedDay { day = savedDay; clock = Date(timeIntervalSince1970: TimeInterval(savedDay)) }
        do {
            let value = try await client.invoke(command, args: args, projectID: projectID)
            guard command == "native_research_journey_add" ? !value.string.isEmpty : value["id"] == args["edit"]?["id"] else { throw ProjectBrowserError.invalidResponse }
            writing = false
            guard presented, self.projectID == projectID, generation == token else { return false }
            uncertainProjects.remove(projectID)
            if let savedDay { day = savedDay; clock = Date(timeIntervalSince1970: TimeInterval(savedDay)) }
            await reload(client)
            return true
        } catch {
            writing = false
            if presented, self.projectID == projectID, generation == token { self.error = localized("操作结果未确认。请刷新核对后继续，不会自动重试。") + "\n" + error.localizedDescription }
            return false
        }
    }

    func shiftMonth(_ amount: Int, client: any NativeSettingsQuerying) async {
        guard presented, let next = calendar.date(byAdding: .month, value: amount, to: clock) else { return }
        clock = next
        day = nil
        entries = []
        truncated = false
        closeDetail()
        await reload(client)
    }

    func showMonth(client: any NativeSettingsQuerying) async {
        guard presented else { return }
        day = nil
        entries = []
        closeDetail()
        await reload(client)
    }

    func showDay(_ value: Int64, client: any NativeSettingsQuerying) async {
        guard presented else { return }
        day = value
        clock = Date(timeIntervalSince1970: TimeInterval(value))
        entries = []
        closeDetail()
        await reload(client)
    }

    func visibleEntries() -> [CalendarEntry] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let rows = NativeCalendarModel.sorted(entries)
        guard !needle.isEmpty else { return rows }
        return rows.filter {
            $0.title.localizedStandardContains(needle) || $0.kind.localizedStandardContains(needle)
        }
    }

    func bounds() -> (Int64, Int64) {
        if let day {
            return NativeCalendarClock.dayInterval(containing: Date(timeIntervalSince1970: TimeInterval(day)), calendar: calendar)
        }
        return NativeCalendarClock.monthInterval(containing: clock, calendar: calendar)
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty, !writing else { return }
        let requested = projectID
        let (from, until) = bounds()
        let current = UUID()
        generation = current
        busy = true; loaded = false; recaps = []
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(
                NativeJourneyCommand.read,
                args: ["from": .integer(from), "until": .integer(until)],
                projectID: requested)
            guard generation == current, presented, self.projectID == requested else { return }
            let page = try JSONDecoder().decode(JourneyPage.self, from: JSONEncoder().encode(value))
            entries = page.entries; recaps = value["recaps"].array; loaded = true
            truncated = page.truncated
            error = nil
        } catch {
            guard generation == current, presented, self.projectID == requested else { return }
            self.error = "研究历程未能确认读取，不会自动重试。\n" + error.localizedDescription
        }
    }
}

struct NativeJourneyPage: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var journey: NativeJourneyModel
    @State private var relationships = false
    @State private var journalEditor = false
    @State private var recapEditor: NativeRecapDraft?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(localized("研究历程")).font(.title2.bold())
                Spacer()
                Button(localized("补充记录")) { journalEditor = true }.disabled(!journey.canWrite)
                Button(localized("刷新")) { Task { await journey.reload(model.calendarClient()) } }.disabled(journey.busy)
                Button(localized("返回对话")) { journey.dismiss(); model.journeyFocus = nil }
            }
            HStack {
                Button { Task { await journey.shiftMonth(-1, client: model.calendarClient()) } } label: { WispIcon(name: "chevron-left") }.accessibilityLabel("上个月")
                Text(NativeCalendarClock.label(journey.clock, format: "yyyy MMMM", calendar: journey.calendar)).font(.headline)
                Button { Task { await journey.shiftMonth(1, client: model.calendarClient()) } } label: { WispIcon(name: "chevron-right") }.accessibilityLabel("下个月")
                Spacer()
                if journey.day != nil { Button(localized("整月")) { Task { await journey.showMonth(client: model.calendarClient()) } } }
            }.disabled(journey.busy)
            Picker(localized("研究视图"), selection: $relationships) { Text(localized("研究记录")).tag(false); Text(localized("关系")).tag(true) }.pickerStyle(.segmented)
            if journey.uncertain { Text(localized("操作结果未确认。请刷新核对后继续，不会自动重试。")); Button(localized("已核对结果，允许继续")) { journey.acknowledge() }.disabled(journey.busy || journey.writing) }
            TextField(localized("搜索研究记录"), text: $journey.query).textFieldStyle(.roundedBorder)
            if let error = journey.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if journey.truncated { Text(localized("仅展示最近 2,000 条活动，请选择具体日期查看。")).font(.caption).foregroundStyle(.secondary) }
            if journey.busy { ProgressView(localized("正在读取研究历程…")) }
            if relationships { NativeJourneyGraph(journey: journey, client: model.calendarClient()) }
            else { ScrollView {
                LazyVStack(alignment: .leading, spacing: 20) {
                    ForEach(Array(journey.recaps.enumerated()), id: \.offset) { _, recap in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(NativeCalendarClock.label(Date(timeIntervalSince1970: TimeInterval(recap["day_start"].integer)), format: "yyyy-MM-dd", calendar: journey.calendar) + " · " + localized("每日研究回顾")).font(.headline)
                            Text(recap["headline"].string).textSelection(.enabled)
                            HStack { Text(NativeJourneyPresentation.label(recap["status"].string)).foregroundStyle(.secondary); Spacer(); Button(localized("查看与编辑回顾")) { recapEditor = NativeRecapDraft(value: recap) } }
                        }.padding(12).background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
                    }
                    ForEach(NativeJourneyDay.grouped(journey.visibleEntries(), calendar: journey.calendar)) { group in
                        VStack(alignment: .leading, spacing: 10) {
                            HStack {
                                Text(NativeCalendarClock.label(Date(timeIntervalSince1970: TimeInterval(group.id)), format: "yyyy-MM-dd EEEE", calendar: journey.calendar)).font(.headline)
                                Spacer()
                                if journey.day == nil {
                                    Button(localized("查看当日")) { Task { await journey.showDay(group.id, client: model.calendarClient()) } }
                                }
                            }
                            Text("会话 \(group.sessionActivityCounts.count) · 产物版本 \(group.entries.filter { $0.kind == "artifact" }.count)").font(.caption).foregroundStyle(.secondary)
                            ForEach(group.entries) { entry in
                                entryRow(entry, count: group.sessionActivityCounts[entry.sourceID ?? ""])
                            }
                        }
                    }
                    if journey.visibleEntries().isEmpty && journey.error == nil && !journey.busy {
                        Text(journey.query.isEmpty ? "这段时间没有已记录的研究活动。" : "没有匹配的研究记录。").foregroundStyle(.secondary)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            } }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(NativeSettingsEscape(enabled: !journalEditor && recapEditor == nil && journey.selectedEntry == nil && !journey.writing) { journey.dismiss(); model.journeyFocus = nil })
        .sheet(isPresented: $journalEditor) { NativeJournalEditor(journey: journey, client: model.calendarClient()) { journalEditor = false } }
        .sheet(item: $recapEditor) { draft in NativeRecapEditor(journey: journey, draft: draft, client: model.calendarClient(), openSession: { id in Task { await model.openJourneySession(id, projectID: journey.projectID ?? "") } }) { recapEditor = nil } }
        .onChange(of: relationships) { on in if on { Task { await journey.loadGraph(model.calendarClient()) } } }
        .sheet(isPresented: Binding(get: { journey.selectedEntry != nil }, set: { if !$0 { journey.closeDetail() } })) {
            JourneyDetailView(journey: journey, client: model.calendarClient())
        }
        .task(id: journey.projectID ?? "") { await journey.reload(model.calendarClient()); if relationships { await journey.loadGraph(model.calendarClient()) } }
    }

    private func entryRow(_ entry: CalendarEntry, count: Int?) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(alignment: .top) {
                WispIcon(name: entry.kind == "session" ? "chat" : entry.kind == "artifact" ? "doc" : "research-trail")
                Text(entry.title).font(.headline)
                Spacer()
                Text(NativeCalendarClock.label(Date(timeIntervalSince1970: TimeInterval(entry.occurredAt)), format: "HH:mm", calendar: journey.calendar)).font(.caption).foregroundStyle(.secondary)
            }
            if let count { Text("\(count) 次会话活动").font(.caption).foregroundStyle(.secondary) }
            if let summary = entry.summary, !summary.isEmpty { Text(summary).lineLimit(4).textSelection(.enabled) }
            HStack {
                if let version = entry.versionNumber { Text("版本 \(version)").font(.caption) }
                if entry.sourceDiscarded == true { Text(localized("来源已丢弃")).font(.caption).foregroundStyle(.secondary) }
                Spacer()
                if let frame = entry.frameID, !frame.isEmpty {
                    Button(localized("打开会话")) { Task { await model.openJourneySession(frame, projectID: model.activeProjectID ?? "") } }
                }
                Button(entry.kind == "artifact" ? "查看产物与来源" : "查看详情") {
                    Task { await journey.showDetail(entry, client: model.calendarClient()) }
                }
            }
        }.padding(12).background(Color.secondary.opacity(0.06)).cornerRadius(8)
    }
}

private struct JourneyDetailView: View {
    @ObservedObject var journey: NativeJourneyModel
    let client: any NativeSettingsQuerying
    @State private var run: NativeJourneyRunID?
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(journey.selectedEntry?.title ?? "研究记录").font(.headline); Spacer(); Button(localized("关闭")) { journey.closeDetail() } }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let entry = journey.selectedEntry {
                        Text(entry.summary ?? "").textSelection(.enabled)
                        Text("来源 ID：\(entry.sourceID ?? entry.id)").font(.caption)
                        if entry.kind == "run", let id = entry.sourceID { Button(localized("运行记录")) { run = NativeJourneyRunID(id: id) } }
                        if let frame = entry.frameID { Text("会话 ID：\(frame)").font(.caption) }
                    }
                    if journey.detailBusy { ProgressView(localized("正在读取产物…")) }
                    if let error = journey.detailError { Text(error).foregroundStyle(.red) }
                    if let artifact = journey.artifact {
                        Text("\(artifact.filename) · 版本 \(artifact.versionNumber)").font(.headline)
                        if let error = artifact.contentError { Text(error).foregroundStyle(.red) }
                        if let run = artifact.source.runID {
                            Text("生成运行：\(artifact.source.runTitle)")
                            Button(localized("运行记录")) { self.run = NativeJourneyRunID(id: run) }
                            Text("运行 ID：\(run)").font(.caption)
                            Text("执行环境：\(artifact.source.contextID)").font(.caption)
                        } else { Text(localized("未记录生成运行。")).foregroundStyle(.secondary) }
                        ForEach(Array(artifact.source.inputs.enumerated()), id: \.offset) { _, input in
                            Text("\(input.role) · \(input.title)")
                            if let version = input.versionID { Text("输入版本：\(version)").font(.caption) }
                        }
                        Divider()
                        if artifact.truncated { Text(localized("内容仅显示前缀。")).font(.caption) }
                        if let text = artifact.text { Text(text).font(.system(.body, design: .monospaced)) }
                        else if artifact.mime.hasPrefix("image/"), let base64 = artifact.base64, let data = Data(base64Encoded: base64), let image = NSImage(data: data) {
                            Image(nsImage: image).resizable().scaledToFit()
                        } else if artifact.contentError == nil { Text(localized("此文件类型暂不支持内联预览。")).foregroundStyle(.secondary) }
                    }
                }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(20).frame(minWidth: 420, idealWidth: 640, minHeight: 360, idealHeight: 560)
            .background(NativeSettingsEscape(enabled: run == nil) { journey.closeDetail() })
            .sheet(item: $run) { item in NativeJourneyRunView(id: item.id, projectID: journey.projectID ?? "", client: client) { run = nil } }
    }
}
