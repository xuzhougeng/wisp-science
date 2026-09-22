import SwiftUI
import WispProjectBrowser

struct JourneyPage: Codable, Equatable {
    var entries: [CalendarEntry]
    var truncated: Bool
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
    private var generation = UUID()
    var clock = Date()
    var calendar = Calendar.current

    func open(projectID: String, day: Int64?) {
        self.projectID = projectID
        self.day = day
        presented = true
    }

    func dismiss() {
        guard !busy else { return }
        presented = false
    }

    func invalidate() {
        generation = UUID()
        presented = false
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

struct NativeJourneySheet: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var journey: NativeJourneyModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("研究历程").font(.headline)
                Spacer()
                Button("关闭") { journey.dismiss() }
            }
            TextField("搜索研究记录", text: $journey.query)
                .textFieldStyle(.roundedBorder)
            if let error = journey.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            if journey.truncated {
                Text("仅展示最近 2,000 条活动。").font(.caption)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(journey.visibleEntries()) { entry in
                        Text(entry.title)
                    }
                    if journey.visibleEntries().isEmpty && journey.error == nil && !journey.busy {
                        Text("这段时间没有已记录的研究活动。").foregroundStyle(.secondary)
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 480, height: 420)
        .interactiveDismissDisabled(journey.busy)
        .background(NativeSettingsEscape(enabled: !journey.busy) { journey.dismiss() })
        .task(id: (journey.projectID ?? "") + ":\(journey.day ?? -1)") { await journey.reload(model.calendarClient()) }
    }
}
