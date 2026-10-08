import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeAssistantModel: ObservableObject {
    nonisolated static let project = "assistant:research"
    nonisolated static let session = "research-assistant"
    let client: any NativeSettingsQuerying
    let conversation: NativeConversationModel
    @Published private(set) var workspace: SettingsValue = .null
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published private(set) var uncertain = false
    @Published private(set) var active = false
    @Published var selectedProject = "" { didSet { applyContext() } }
    @Published var day = Date()
    private var generation = UUID()
    private var refreshed = false
    private var writing = false
    var projects: [SettingsValue] { workspace["projects"].array }
    var schedules: [SettingsValue] { workspace["schedules"].array }
    var plan: [SettingsValue] { workspace["plan"].array.filter { selectedProject.isEmpty || $0["project_id"].string == selectedProject } }
    var canWrite: Bool { active && !busy && !writing && !uncertain && workspace != .null }
    var canAcknowledge: Bool { uncertain && refreshed && !busy && !writing }
    var dayKey: String { NativeCalendarClock.label(day, format: "yyyy-MM-dd", calendar: .current) }
    init(client: any NativeSettingsQuerying) {
        self.client = client
        conversation = NativeConversationModel(client: NativeConversationClient(transport: client))
        conversation.restrictedAssistant = true
        conversation.contextReady = false
    }
    func open() async {
        generation = UUID(); let token = generation
        active = true; busy = true; workspace = .null; error = nil
        do {
            let opened = try await client.invoke("native_assistant_open", args: [:], projectID: nil)
            guard active, generation == token else { return }
            guard opened["project_id"].string == Self.project, opened["session_id"].string == Self.session else { throw ProjectBrowserError.invalidResponse }
            busy = false
            await reload()
            guard active, generation == token, workspace != .null else { return }
            await conversation.open(project: Self.project, session: Self.session)
            guard active, generation == token else { return }
            applyContext()
        } catch { if active, generation == token { busy = false; self.error = error.localizedDescription } }
    }
    func selectDay(_ value: Date) {
        day = value
        guard !writing else { return }
        generation = UUID(); busy = false
        Task { await reload() }
    }
    func close() { active = false; generation = UUID(); busy = false; workspace = .null; conversation.pause() }
    func reload() async {
        guard active, !busy else { return }
        let token = generation, date = dayKey
        busy = true; workspace = .null; error = nil; refreshed = false
        defer { if generation == token { busy = false } }
        // Remove context until the authoritative privacy-filtered read succeeds.
        conversation.contextReference = nil
        conversation.contextReady = false
        do {
            let value = try await client.invoke("native_assistant_workspace", args: ["day": .string(date)], projectID: nil)
            guard active, generation == token, dayKey == date else { return }
            guard value["day"].string == date, value["projects"].array.allSatisfy({ !$0["id"].string.isEmpty && $0["id"].string != Self.project }) else { throw ProjectBrowserError.invalidResponse }
            let ids = Set(value["projects"].array.map { $0["id"].string })
            guard value["plan"].array.allSatisfy({ $0["project_id"] == .null || ids.contains($0["project_id"].string) }),
                  value["schedules"].array.allSatisfy({ ids.contains($0["project_id"].string) }) else { throw ProjectBrowserError.invalidResponse }
            workspace = value; refreshed = true
            conversation.contextReady = true
            if !ids.contains(selectedProject) { selectedProject = "" }
            applyContext()
        } catch { if active, generation == token { self.error = error.localizedDescription } }
    }
    func acknowledge() { guard canAcknowledge else { return }; uncertain = false; error = nil }
    @discardableResult func mutate(_ operation: [String: SettingsValue]) async -> Bool {
        guard canWrite else { return false }
        let token = generation
        writing = true; busy = true; uncertain = true; refreshed = false; error = nil
        var confirmed = false
        do {
            _ = try await client.invoke("native_assistant_mutate", args: ["operation": .object(operation)], projectID: nil)
            confirmed = true; uncertain = false
        } catch { if active, token == generation { self.error = localized("自动化操作结果未确认，请刷新并核对；不会自动重试。") + "\n" + error.localizedDescription } }
        writing = false
        guard active, token == generation else { return confirmed }
        busy = false
        if confirmed { await reload() }
        return confirmed
    }
    private func applyContext() {
        conversation.contextReference = projects.first { $0["id"].string == selectedProject }.map {
            NativeComposerReference(reference: .object(["kind": .string("project"), "id": $0["id"]]), label: $0["name"].string, detail: $0["description"].string)
        }
    }
}

struct NativeAutomationDraft: Equatable {
    var name = ""
    var prompt = ""
    var project = ""
    var session = ""
    var skill = ""
    var cadence = "interval"
    var minutes = 60
    var time = Date()
    var weekday = 2
    var timer = false
    func operation(now: Date = Date(), calendar: Calendar = .current) -> [String: SettingsValue]? {
        guard !project.isEmpty, !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, (1...525600).contains(minutes), !timer || !session.isEmpty else { return nil }
        if timer { return ["action": .string("set_timer"), "project_id": .string(project), "session_id": .string(session), "expression": .string("\(minutes)m \(prompt)")] }
        var interval = Int64(minutes * 60), start: SettingsValue = .null
        if cadence != "interval" {
            var match = calendar.dateComponents([.hour, .minute], from: time)
            if cadence == "weekly" { match.weekday = weekday; interval = 7 * 86400 } else { interval = 86400 }
            guard let next = calendar.nextDate(after: now, matching: match, matchingPolicy: .nextTime) else { return nil }
            start = .integer(Int64(next.timeIntervalSince1970))
        }
        return ["action": .string("create_schedule"), "project_id": .string(project), "name": .string(name), "prompt": .string(prompt), "interval_secs": .integer(interval), "session_id": session.isEmpty ? .null : .string(session), "skill": skill.isEmpty ? .null : .string(skill), "start_at": start]
    }
}

struct NativeAutomationTemplate: Decodable, Identifiable {
    let icon: String
    let en: String
    let zh: String
    let prompt_en: String
    let prompt_zh: String
    let cadence: String
    let time: String
    let weekday: Int
    var id: String { en }
    static let all: [Self] = {
        guard let url = WispDesign.resources.url(forResource: "automation-templates", withExtension: "json"), let data = try? Data(contentsOf: url) else { return [] }
        return (try? JSONDecoder().decode([Self].self, from: data)) ?? []
    }()
    func draft(project: String, locale: String, now: Date = Date(), calendar: Calendar = .current) -> NativeAutomationDraft {
        var draft = NativeAutomationDraft()
        draft.project = project; draft.name = locale == "en" ? en : zh; draft.prompt = locale == "en" ? prompt_en : prompt_zh; draft.cadence = cadence
        draft.weekday = weekday + 1
        let parts = time.split(separator: ":").compactMap { Int($0) }
        if parts.count == 2 { draft.time = calendar.date(bySettingHour: parts[0], minute: parts[1], second: 0, of: now) ?? now }
        return draft
    }
}

struct NativeAssistantPage: View {
    @ObservedObject var browser: ProjectBrowserModel
    @ObservedObject var model: NativeAssistantModel
    @StateObject private var inlineCalendar = NativeCalendarModel()
    @State private var details = false
    @State private var automations = false
    var body: some View {
        GeometryReader { geometry in
            VStack(spacing: 0) {
                ViewThatFits(in: .horizontal) {
                    HStack { heading; Spacer(); actions }
                    VStack(alignment: .leading) { heading; actions }
                }.padding(16)
                Divider()
                HStack(spacing: 0) {
                    if geometry.size.width >= 1200 { sidebar.frame(width: 260); Divider() }
                    NativeConversationView(conversation: model.conversation, projectID: NativeAssistantModel.project, sessionID: NativeAssistantModel.session)
                    if geometry.size.width >= 1200 { Divider(); NativeCalendarPage(model: browser, calendar: inlineCalendar, embedded: true).frame(width: 340) }
                    else if geometry.size.width >= 1000 { Divider(); sidebar.frame(width: 300) }
                }
            }
        }
        .task { await model.open() }
        .onDisappear { model.close() }
        .onChange(of: model.conversation.snapshot?.running) { running in if running == false { Task { await model.reload() } } }
        .sheet(isPresented: $details) { sidebar.padding(20).frame(minWidth: 340, idealWidth: 440, minHeight: 480).background(NativeSettingsEscape { details = false }) }
        .sheet(isPresented: $automations) { NativeAutomationsSheet(model: model) { automations = false } }
    }
    private var heading: some View {
        HStack { Button(localized("返回首页")) { browser.assistantPresented = false }; Text(localized("科研助理")).font(.title2.bold()) }
    }
    private var actions: some View {
        HStack {
            Button { details = true } label: { WispIcon(name: "list") }.help(localized("项目与计划")).accessibilityLabel(localized("项目与计划"))
            Button { browser.calendar.presented = true } label: { WispIcon(name: "calendar") }.help(localized("研究日历")).accessibilityLabel(localized("研究日历"))
            Button { automations = true } label: { WispIcon(name: "clock") }.help(localized("自动化")).accessibilityLabel(localized("自动化"))
            Button { Task { await model.reload() } } label: { WispIcon(name: "refresh") }.disabled(model.busy).accessibilityLabel(localized("刷新"))
        }.buttonStyle(WispButtonStyle())
    }
    private var sidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(localized("汇总记录、保存研究计划，并把工作交给项目会话。")).foregroundStyle(.secondary)
                if model.busy { ProgressView() }
                if model.uncertain { Text(localized("自动化操作结果未确认，请刷新并核对；不会自动重试。")).foregroundStyle(.orange) }
                if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
                Picker(localized("项目上下文"), selection: $model.selectedProject) {
                    Text(localized("全部项目")).tag("")
                    ForEach(Array(model.projects.enumerated()), id: \.offset) { _, row in Text(row["name"].string).tag(row["id"].string) }
                }.disabled(model.workspace == .null)
                Text(localized("所选项目作为后续提问的上下文。")).font(.caption).foregroundStyle(.secondary)
                DatePicker(localized("研究计划"), selection: Binding(get: { model.day }, set: model.selectDay), displayedComponents: .date)
                ForEach(Array(model.plan.enumerated()), id: \.offset) { _, row in
                    VStack(alignment: .leading) {
                        Text(row["title"].string).fontWeight(.medium)
                        Text(row["project_name"].string + " · " + localized(row["status"].string)).font(.caption).foregroundStyle(.secondary)
                        if !row["session_id"].string.isEmpty, !row["project_id"].string.isEmpty {
                            Button(localized("打开会话")) { browser.assistantPresented = false; Task { await browser.openProject(row["project_id"].string, sessionID: row["session_id"].string) } }
                        }
                    }
                }
                if model.plan.isEmpty && !model.busy { Text(localized("这一天没有保存的计划。向助理说明计划即可保存。")).foregroundStyle(.secondary) }
            }.padding(16)
        }
    }
}

struct NativeAutomationsSheet: View {
    @ObservedObject var model: NativeAssistantModel
    let close: () -> Void
    @State private var editor = false
    @State private var draft = NativeAutomationDraft()
    @State private var recapTime = "09:00"
    @State private var deletion: SettingsValue?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("自动化")).font(.title2.bold()); Spacer(); Button(localized("关闭"), action: close) }
            if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if model.uncertain {
                Text(localized("自动化操作结果未确认，请刷新并核对；不会自动重试。"))
                HStack { Button(localized("刷新")) { Task { await model.reload() } }; Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(!model.canAcknowledge) }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    NativeSettingsGroup(title: "每日研究回顾") {
                        Toggle(localized("启用"), isOn: Binding(get: { model.workspace["daily_recap"]["enabled"].bool }, set: { on in Task { await model.mutate(["action": .string("set_daily_recap"), "enabled": .bool(on), "time": model.workspace["daily_recap"]["time"]]) } })).disabled(!model.canWrite)
                        HStack { TextField(localized("时间（HH:MM）"), text: $recapTime); Button(localized("保存")) { Task { await model.mutate(["action": .string("set_daily_recap"), "enabled": model.workspace["daily_recap"]["enabled"], "time": .string(recapTime)]) } }.disabled(!model.canWrite) }
                        Button(localized("立即运行")) { Task { await model.mutate(["action": .string("run_daily_recap")]) } }.disabled(!model.canWrite || model.workspace["daily_recap"]["running"].bool)
                        if !model.workspace["daily_recap"]["error"].string.isEmpty { Text(model.workspace["daily_recap"]["error"].string).foregroundStyle(.orange) }
                    }
                    NativeSettingsGroup(title: "定时任务模板") {
                        ForEach(NativeAutomationTemplate.all) { template in
                            Button {
                                draft = template.draft(project: model.selectedProject, locale: UserDefaults.standard.string(forKey: "nativeSettings.locale") ?? "zh")
                                editor = true
                            } label: {
                                HStack(alignment: .top) { WispIcon(name: template.icon); VStack(alignment: .leading, spacing: 6) { Text(UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? template.en : template.zh).fontWeight(.semibold); Text(UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? template.prompt_en : template.prompt_zh).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading) } }.frame(maxWidth: .infinity, alignment: .leading)
                            }.buttonStyle(.plain).disabled(!model.canWrite)
                        }
                    }
                    Button(localized("添加自动化")) { draft = NativeAutomationDraft(); draft.project = model.selectedProject; editor = true }.disabled(!model.canWrite)
                    Text(localized("应用运行时按间隔触发；每日和每周从所选本地时间开始，之后按固定间隔运行。")).font(.caption).foregroundStyle(.secondary)
                    ForEach(Array(model.schedules.enumerated()), id: \.offset) { _, row in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack { Text(row["name"].string).fontWeight(.semibold); Spacer(); Toggle(localized("启用"), isOn: Binding(get: { row["enabled"].bool }, set: { on in Task { await model.mutate(["action": .string("set_enabled"), "id": row["id"], "enabled": .bool(on)]) } })).disabled(!model.canWrite) }
                            Text(row["prompt"].string).textSelection(.enabled)
                            Text((model.projects.first { $0["id"] == row["project_id"] }?["name"].string ?? "") + " · " + localized("下次运行") + ": " + Date(timeIntervalSince1970: TimeInterval(row["next_run_at"].integer)).formatted()).font(.caption).foregroundStyle(.secondary)
                            HStack { Button(localized("立即运行")) { Task { await model.mutate(["action": .string("run_now"), "id": row["id"]]) } }; Button(localized("删除"), role: .destructive) { deletion = row } }.disabled(!model.canWrite)
                            ForEach(Array(model.workspace["runs"].array.filter { $0["schedule_id"] == row["id"] }.prefix(10).enumerated()), id: \.offset) { _, run in
                                Text(localized(run["status"].string) + " · " + Date(timeIntervalSince1970: TimeInterval(run["fired_at"].integer)).formatted() + (run["error"].string.isEmpty ? "" : " · " + run["error"].string)).font(.caption).textSelection(.enabled)
                            }
                        }; Divider()
                    }
                }
            }
        }.padding(24).frame(minWidth: 380, idealWidth: 680, minHeight: 480)
        .background(NativeSettingsEscape(enabled: !model.busy) { close() })
        .onAppear { recapTime = model.workspace["daily_recap"]["time"].string }
        .sheet(isPresented: $editor) { NativeAutomationEditor(model: model, draft: $draft) { editor = false } }
        .alert(localized("删除自动化？"), isPresented: Binding(get: { deletion != nil }, set: { if !$0 { deletion = nil } })) {
            Button(localized("删除"), role: .destructive) { if let row = deletion { Task { await model.mutate(["action": .string("delete"), "id": row["id"]]) } }; deletion = nil }
            Button(localized("取消"), role: .cancel) { deletion = nil }
        }
    }
}

struct NativeAutomationEditor: View {
    @ObservedObject var model: NativeAssistantModel
    @Binding var draft: NativeAutomationDraft
    let close: () -> Void
    @State private var discard = false
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("添加自动化")).font(.title2.bold()); Spacer(); Button(localized("关闭")) { dismiss() }.disabled(model.busy) }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Picker(localized("项目"), selection: $draft.project) { Text(localized("选择项目")).tag(""); ForEach(Array(model.projects.enumerated()), id: \.offset) { _, row in Text(row["name"].string).tag(row["id"].string) } }
                    TextField(localized("名称"), text: $draft.name)
                    Text(localized("提示词")); TextEditor(text: $draft.prompt).frame(minHeight: 100)
                    TextField(localized("目标会话 ID（留空新建会话）"), text: $draft.session)
                    Toggle(localized("会话定时器：替换上次完整轮次"), isOn: $draft.timer)
                    if draft.timer { Stepper(localized("间隔分钟") + ": \(draft.minutes)", value: $draft.minutes, in: 1...525600) }
                    else {
                        Picker(localized("频率"), selection: $draft.cadence) { Text(localized("固定间隔")).tag("interval"); Text(localized("每天")).tag("daily"); Text(localized("每周")).tag("weekly") }
                        if draft.cadence == "interval" { Stepper(localized("间隔分钟") + ": \(draft.minutes)", value: $draft.minutes, in: 1...525600) }
                        else { DatePicker(localized("时间"), selection: $draft.time, displayedComponents: .hourAndMinute); if draft.cadence == "weekly" { Picker(localized("星期"), selection: $draft.weekday) { ForEach(1...7, id: \.self) { i in Text(Calendar.current.weekdaySymbols[i - 1]).tag(i) } } } }
                        TextField(localized("技能（可选）"), text: $draft.skill)
                    }
                    if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
                    Button(localized("保存")) { if let operation = draft.operation() { Task { if await model.mutate(operation) { close() } } } }.disabled(!model.canWrite || draft.operation() == nil)
                }.textFieldStyle(NativeSettingsTextFieldStyle())
            }
        }.padding(24).frame(minWidth: 360, idealWidth: 560, minHeight: 460)
        .background(NativeSettingsEscape(enabled: !model.busy) { dismiss() })
        .alert(localized("放弃未保存的修改？"), isPresented: $discard) { Button(localized("放弃修改"), role: .destructive, action: close); Button(localized("取消"), role: .cancel) {} }
    }
    private func dismiss() { if draft.name.isEmpty && draft.prompt.isEmpty && draft.session.isEmpty { close() } else { discard = true } }
}
