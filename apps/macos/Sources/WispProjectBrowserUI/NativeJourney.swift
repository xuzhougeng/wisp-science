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
        entries = []
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
        guard presented, let projectID, !projectID.isEmpty else { return }
        let requested = projectID
        let (from, until) = bounds()
        let current = UUID()
        generation = current
        busy = true
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(
                NativeJourneyCommand.read,
                args: ["from": .integer(from), "until": .integer(until)],
                projectID: requested)
            guard generation == current, presented, self.projectID == requested else { return }
            let page = try JSONDecoder().decode(JourneyPage.self, from: JSONEncoder().encode(value))
            entries = page.entries
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

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(localized("研究历程")).font(.title2.bold())
                Spacer()
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
            TextField(localized("搜索研究记录"), text: $journey.query).textFieldStyle(.roundedBorder)
            if let error = journey.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if journey.truncated { Text(localized("仅展示最近 2,000 条活动，请选择具体日期查看。")).font(.caption).foregroundStyle(.secondary) }
            if journey.busy { ProgressView(localized("正在读取研究历程…")) }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 20) {
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
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(NativeSettingsEscape { journey.dismiss(); model.journeyFocus = nil })
        .sheet(isPresented: Binding(get: { journey.selectedEntry != nil }, set: { if !$0 { journey.closeDetail() } })) {
            JourneyDetailView(journey: journey)
        }
        .task(id: journey.projectID ?? "") { await journey.reload(model.calendarClient()) }
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
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(journey.selectedEntry?.title ?? "研究记录").font(.headline); Spacer(); Button(localized("关闭")) { journey.closeDetail() } }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let entry = journey.selectedEntry {
                        Text(entry.summary ?? "").textSelection(.enabled)
                        Text("来源 ID：\(entry.sourceID ?? entry.id)").font(.caption)
                        if let frame = entry.frameID { Text("会话 ID：\(frame)").font(.caption) }
                    }
                    if journey.detailBusy { ProgressView(localized("正在读取产物…")) }
                    if let error = journey.detailError { Text(error).foregroundStyle(.red) }
                    if let artifact = journey.artifact {
                        Text("\(artifact.filename) · 版本 \(artifact.versionNumber)").font(.headline)
                        if let error = artifact.contentError { Text(error).foregroundStyle(.red) }
                        if let run = artifact.source.runID {
                            Text("生成运行：\(artifact.source.runTitle)")
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
            .background(NativeSettingsEscape { journey.closeDetail() })
    }
}
