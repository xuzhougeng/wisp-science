import Foundation
import Combine
import UIKit
import ImageIO
import WispRemote

struct AttachedPhoto: Identifiable {
    let id = UUID()
    let path: String
    let data: Data
}

enum QuestionSubmission: Equatable { case sent, uncertain }

enum WorkspaceSheet: Identifiable {
    case connection, inbox, files(ConversationRoute, String, Bool)
    case conversationInfo(String, String)
    case research(String?)
    case checklist, notebook
    var id: String {
        switch self {
        case .connection: return "connection"
        case .inbox: return "inbox"
        case .research(let project): return "research:\(project ?? "calendar")"
        case .checklist: return "checklist"
        case .notebook: return "notebook"
        case .conversationInfo: return "conversation-info"
        case .files(let route, let path, _): return "files:\(route.project):\(route.session):\(path)"
        }
    }
}

@MainActor final class RemoteWorkspace: ObservableObject {
    let connection: RemoteConnection
    let checklist: ChecklistModel
    let notebook: NotebookModel
    private let credentials: any RemoteCredentialStoring
    private let preferences: UserDefaults?
    private let cache: RemoteReadCache?
    private var cacheGeneration = UUID()
    private var reads: [String: (UUID, Task<RemoteValue, Error>)] = [:]
    private var projectsRefreshed = false
    private var sessionsRefreshed: String?
    @Published private(set) var snapshotIsCached = false
    @Published private(set) var snapshotSavedAt: Date?
    private var snapshotSession: UUID?
    @Published private(set) var cachedHostName = ""
    @Published private(set) var cacheBytes = 0
    @Published private(set) var clearingCache = false
    @Published private(set) var cacheNotice: String?
    private static let demoPreference = "wisp.workspace.demo"
    private var demo: DemoDataset?
    @Published private(set) var isDemo = false
    @Published private(set) var configuration: RemoteConfiguration?
    @Published var projects: [RemoteValue] = []
    @Published var sessions: [RemoteValue] = []
    @Published var inbox: [RemoteValue] = []
    @Published var selectedProject: String? { didSet {
        if selectedProject != oldValue { selectedSession = nil; sessions = []; sessionsRefreshed = nil }
    } }
    @Published var selectedSession: String? { didSet {
        if selectedSession != oldValue { history = RemoteHistory(); error = nil; snapshotIsCached = false; snapshotSavedAt = nil; snapshotSession = nil }
    } }
    @Published var history = RemoteHistory()
    @Published var sheet: WorkspaceSheet? { didSet {
        if sheet?.id != oldValue?.id { researchPath = []; checklistPath = []; notebookPath = []; notebookCameraRecord = nil; notebookLibraryPresented = false }
    } }
    @Published var researchPath: [ResearchDestination] = []
    @Published var checklistPath: [String] = []
    @Published var notebookPath: [NotebookDestination] = []
    @Published var notebookCameraRecord: String?
    @Published var notebookLibraryPresented = false
    @Published var error: String?
    @Published var drafts: [ConversationRoute: String] = [:]
    @Published var photos: [ConversationRoute: [AttachedPhoto]] = [:]
    @Published var busy: Set<ConversationRoute> = []
    @Published private(set) var questionSubmissions: [String: QuestionSubmission] = [:]
    @Published var loadingEarlier = false
    @Published var creating = false
    var visibleRoute: ConversationRoute?
    private var identity = UUID()
    private var seen: Set<String> = []
    private var active = true
    private var restored = false

    init(connection: RemoteConnection? = nil, credentials: any RemoteCredentialStoring = RemoteCredentials(),
         preferences: UserDefaults? = .standard, cache: RemoteReadCache? = .shared,
         offlineDirectory: URL = OfflineResearchStorage.directory) {
        self.connection = connection ?? RemoteConnection()
        self.checklist = ChecklistModel(directory: offlineDirectory)
        self.notebook = NotebookModel(directory: offlineDirectory)
        self.credentials = credentials
        self.preferences = preferences
        self.cache = cache
        self.connection.authenticated = { [weak self] in
            guard let self, !self.isDemo, let configuration = self.configuration else { return }
            do { try self.credentials.save(configuration) }
            catch { self.error = error.localizedDescription }
            self.seen = []
            self.snapshotIsCached = self.snapshot != nil
            let generation = self.cacheGeneration
            let scope = RemoteReadCache.scope(configuration)
            let metadata: RemoteValue = .object(["name": .string(self.connection.hostName)])
            Task {
                let revision = await self.cache?.currentRevision()
                guard self.cacheGeneration == generation else { return }
                await self.cache?.store(metadata, scope: scope, command: "host_metadata", expectedRevision: revision)
            }
        }
    }

    var route: ConversationRoute? {
        guard let project = selectedProject, let session = selectedSession else { return nil }
        return ConversationRoute(project: project, session: session)
    }
    var snapshot: RemoteSnapshot? { history.snapshot }
    var title: String { sessions.first { $0["id"].string == selectedSession }?["title"].string ?? "对话" }
    var canActOnSnapshot: Bool { connection.state == .online && snapshot != nil && !snapshotIsCached && snapshotSession == connection.readSessionID }
    var hostName: String { connection.hostName.isEmpty ? cachedHostName : connection.hostName }
    private var cacheScope: String? { isDemo ? nil : configuration.map(RemoteReadCache.scope) }

    func restore() {
        guard !restored else { return }
        restored = true
        if preferences?.bool(forKey: Self.demoPreference) == true { enterDemo(); return }
        do {
            if let saved = try credentials.load() {
                configuration = saved
                if active { connection.connect(saved) }
                Task { await self.hydrateCache() }
            }
        } catch { self.error = error.localizedDescription }
    }
    func connect(address: String, code: String) {
        do {
            let configuration = try RemoteConfiguration(address: address, code: code)
            preferences?.set(false, forKey: Self.demoPreference)
            demo = nil
            isDemo = false
            clearWorkspace()
            self.configuration = configuration
            connection.connect(configuration)
            Task { await self.hydrateCache() }
        } catch { self.error = error.localizedDescription }
    }
    func setActive(_ active: Bool) {
        guard active != self.active else { return }
        self.active = active
        if isDemo { return }
        if active, let configuration { connection.connect(configuration) }
        else { connection.disconnect() }
    }
    func reconnect() { if let configuration { connection.connect(configuration) } }

    func enterDemo() {
        do {
            let dataset = try DemoDataset()
            restored = true
            connection.disconnect()
            clearWorkspace()
            configuration = nil
            demo = dataset
            isDemo = true
            sheet = nil
            preferences?.set(true, forKey: Self.demoPreference)
            connection.useLocalService(name: "本地演示 · 合成数据", commands: DemoDataset.commands) { command, project, args in
                try dataset.call(command, project: project, args: args)
            }
            let identity = self.identity
            Task {
                guard self.identity == identity, self.isDemo else { return }
                await refreshSelection()
            }
        } catch { self.error = error.localizedDescription }
    }
    func exitDemo() {
        guard isDemo else { return }
        connection.disconnect()
        demo = nil
        isDemo = false
        configuration = nil
        sheet = nil
        preferences?.set(false, forKey: Self.demoPreference)
        clearWorkspace()
    }
    func forget() {
        if isDemo { exitDemo(); return }
        do {
            try credentials.remove()
            let scope = cacheScope
            connection.disconnect()
            configuration = nil
            sheet = nil
            clearWorkspace()
            if let scope { Task { await cache?.clear(scope: scope) } }
        } catch { self.error = error.localizedDescription }
    }
    private func clearWorkspace() {
        identity = UUID()
        sheet = nil; researchPath = []
        cacheGeneration = UUID()
        for (_, task) in reads.values { task.cancel() }
        reads = [:]; projectsRefreshed = false; sessionsRefreshed = nil
        snapshotIsCached = false; snapshotSavedAt = nil; snapshotSession = nil; cachedHostName = ""; cacheNotice = nil
        visibleRoute = nil
        selectedProject = nil; selectedSession = nil
        projects = []; sessions = []; inbox = []; drafts = [:]; photos = [:]
        busy = []; loadingEarlier = false; creating = false
        history = RemoteHistory(); seen = []; error = nil; questionSubmissions = [:]
    }

    func cachedRead(_ command: String, project: String? = nil, args: [String: RemoteValue] = [:]) async -> RemoteReadCache.Entry? {
        guard let cache, let scope = cacheScope else { return nil }
        let identity = self.identity, generation = cacheGeneration
        let entry = await cache.read(scope: scope, command: command, project: project, args: args)
        guard self.identity == identity, cacheGeneration == generation, let entry,
              (try? validateRead(entry.value, command: command, project: project, args: args)) != nil else { return nil }
        return entry
    }

    /// Coalesce identical reads; mutations continue to use the connection directly.
    func read(_ command: String, project: String? = nil, args: [String: RemoteValue] = [:]) async throws -> RemoteValue {
        guard RemoteReadCache.accepts(command), command != "host_metadata" else { throw RemoteError.unsupported }
        let identity = self.identity, generation = cacheGeneration
        let session = connection.readSessionID
        let scope = cacheScope
        let key = RemoteReadCache.key(scope: scope ?? "demo", command: command, project: project, args: args)
        let request: (UUID, Task<RemoteValue, Error>)
        if let existing = reads[key] { request = existing }
        else {
            request = (UUID(), Task {
                let revision = await self.cache?.currentRevision()
                guard self.identity == identity, self.connection.readSessionID == session else { throw RemoteError.disconnected }
                let result = try await self.connection.call(command, project: project, args: args)
                try Task.checkCancellation()
                guard self.identity == identity, self.connection.readSessionID == session else { throw RemoteError.disconnected }
                try self.validateRead(result, command: command, project: project, args: args)
                if let scope, self.cacheGeneration == generation {
                    await self.cache?.store(result, scope: scope, command: command, project: project, args: args, expectedRevision: revision)
                }
                return result
            })
            reads[key] = request
        }
        defer { if reads[key]?.0 == request.0 { reads.removeValue(forKey: key) } }
        let result = try await request.1.value
        try Task.checkCancellation()
        guard self.identity == identity, connection.readSessionID == session, connection.state == .online else { throw RemoteError.disconnected }
        return result
    }

    private func validateRead(_ value: RemoteValue, command: String, project: String?, args: [String: RemoteValue]) throws {
        switch command {
        case "list_projects", "remote_sessions", "native_conversation_inbox":
            guard case .array = value, value.array.allSatisfy({ !$0["id"].string.isEmpty }) else { throw RemoteError.incompatibleResponse }
        case "native_conversation_snapshot":
            _ = try RemoteSnapshot(value, route: ConversationRoute(project: project ?? "", session: (args["session_id"]?.string ?? "")))
        case "native_conversation_panel_file_directory", "native_conversation_panel_file_read":
            guard value["schema"].string == "wisp.native-files.v1", value["project_id"].string == project,
                  value["session_id"].string == (args["session_id"]?.string ?? "") else { throw RemoteError.incompatibleResponse }
        case "native_conversation_image":
            guard let data = Data(base64Encoded: value["base64"].string), UIImage(data: data) != nil else { throw RemoteError.service("图片无法显示。") }
        case "host_metadata": break
        default: throw RemoteError.unsupported
        }
    }

    private func hydrateCache() async {
        guard !isDemo else { return }
        let identity = self.identity, project = selectedProject, route = self.route
        if let entry = await cachedRead("list_projects"), self.identity == identity, !projectsRefreshed { projects = entry.value.array }
        if let entry = await cachedRead("host_metadata"), self.identity == identity { cachedHostName = entry.value["name"].string }
        if let project, let entry = await cachedRead("remote_sessions", project: project),
           self.identity == identity, selectedProject == project, sessionsRefreshed != project { sessions = entry.value.array }
        if let route, let entry = await cachedRead("native_conversation_snapshot", project: route.project, args: route.args),
           self.identity == identity, self.route == route, history.snapshot == nil,
           let snapshot = try? RemoteSnapshot(entry.value, route: route) {
            history.replace(snapshot); snapshotIsCached = true; snapshotSavedAt = entry.savedAt
        }
    }

    func updateCacheSize() async { cacheBytes = await cache?.size() ?? 0 }
    func clearCachedData() async {
        guard !clearingCache else { return }
        clearingCache = true
        defer { clearingCache = false }
        cacheGeneration = UUID()
        await cache?.clear()
        await updateCacheSize()
        cacheNotice = "本机缓存已清除，后续读取时会重新缓存。"
    }

    func monitor() async {
        if isDemo { await refreshSelection(); return }
        let identity = self.identity
        var lastSidebar = Date.distantPast
        while !Task.isCancelled && connection.state == .online && active && self.identity == identity && !isDemo {
            do {
                if Date().timeIntervalSince(lastSidebar) > 8 {
                    try await refreshSidebar()
                    lastSidebar = Date()
                }
                try await refreshConversation()
            } catch is CancellationError { return }
            catch { if !Task.isCancelled && self.identity == identity { self.error = error.localizedDescription } }
            do { try await Task.sleep(nanoseconds: snapshot?.running == true ? 1_500_000_000 : 4_000_000_000) }
            catch { return }
        }
    }

    func refreshSidebar() async throws {
        let identity = self.identity
        let configuration = self.configuration
        let result = try await read("list_projects")
        try Task.checkCancellation()
        guard self.configuration == configuration, self.identity == identity else { return }
        projects = result.array
        projectsRefreshed = true
        if let project = selectedProject {
            let result = try await read("remote_sessions", project: project)
            try Task.checkCancellation()
            if selectedProject == project && self.configuration == configuration && self.identity == identity {
                sessions = result.array; sessionsRefreshed = project
            }
        }
        if let project = projects.first?["id"].string, connection.supports("native_conversation_inbox") {
            let result = try await read("native_conversation_inbox", project: project)
            try Task.checkCancellation()
            if self.configuration == configuration && self.identity == identity { inbox = result.array }
        }
    }
    func refreshConversation() async throws {
        guard let route else { return }
        let identity = self.identity
        let session = connection.readSessionID
        let configuration = self.configuration
        let result: RemoteValue
        do { result = try await read("native_conversation_snapshot", project: route.project, args: route.args) }
        catch {
            if self.route == route, self.identity == identity { snapshotIsCached = snapshot != nil }
            throw error
        }
        let next = try RemoteSnapshot(result, route: route)
        try Task.checkCancellation()
        guard self.route == route, self.configuration == configuration, self.identity == identity, connection.readSessionID == session else { return }
        guard next.supersedes(history.snapshot) else { return }
        history.replace(next)
        snapshotIsCached = false; snapshotSavedAt = Date(); snapshotSession = session
        let stamp = "\(next.epoch):\(route.project):\(route.session):\(next.sequence)"
        if !next.running && active && visibleRoute == route && sheet == nil && connection.supports("native_conversation_seen"), !seen.contains(stamp) {
            _ = try await connection.call("native_conversation_seen", project: route.project, args: route.args)
            seen = [stamp]
        }
    }
    func refreshSelection() async {
        let identity = self.identity
        await hydrateCache()
        guard self.identity == identity, connection.state == .online else { return }
        do {
            try await refreshSidebar()
            guard self.identity == identity else { return }
            try await refreshConversation()
            if isDemo && self.identity == identity { try await refreshSidebar() }
        }
        catch is CancellationError { }
        catch { if self.identity == identity { self.error = error.localizedDescription } }
    }

    func createConversation() async {
        guard let project = selectedProject, !creating else { return }
        let identity = self.identity
        creating = true
        defer { creating = false }
        do {
            let result = try await connection.call("native_conversation_create", project: project)
            guard selectedProject == project, self.identity == identity else { return }
            guard !result.string.isEmpty else { throw RemoteError.incompatibleResponse }
            selectedSession = result.string
            await refreshSelection()
        } catch { if self.identity == identity { self.error = error.localizedDescription } }
    }
    func question(at index: Int, item: RemoteValue) -> RemoteQuestion? {
        let items = history.older + (snapshot?.items ?? [])
        let offset = index + history.older.count
        guard items.indices.contains(offset), items[offset] == item else { return nil }
        return RemoteQuestion(item, hasLaterUser: items.dropFirst(offset + 1).contains { $0["role"].string == "user" })
    }

    func questionID(at index: Int, item: RemoteValue, route: ConversationRoute) -> String {
        RemoteReadCache.key(scope: snapshot?.epoch ?? "", command: "question", project: route.project,
            args: route.args.merging(["index": .integer(Int64(index)), "before": snapshot?.before ?? .null, "item": item]) { _, v in v })
    }

    func canAnswerQuestion(at index: Int, item: RemoteValue, route: ConversationRoute) -> Bool {
        guard canActOnSnapshot, self.route == route, let snapshot, snapshot.writable, !snapshot.running,
              !busy.contains(route), connection.supports("native_conversation_send"),
              let question = question(at: index, item: item), question.isNative, question.state == .pending else { return false }
        return questionSubmissions[questionID(at: index, item: item, route: route)] == nil
    }

    func answerQuestion(at index: Int, item: RemoteValue, id: String, route: ConversationRoute,
                        option: Int?, freeform: String) async {
        guard id == questionID(at: index, item: item, route: route),
              canAnswerQuestion(at: index, item: item, route: route),
              let answer = question(at: index, item: item)?.answer(option: option, freeform: freeform) else { return }
        let identity = self.identity
        busy.insert(route)
        defer { if self.identity == identity { busy.remove(route) } }
        do {
            // Keep the composer's draft and attachments untouched. This answer
            // is a separate user message and is never queued or auto-replayed.
            _ = try await connection.call("native_conversation_send", project: route.project,
                args: route.args.merging(["request_id": .string(UUID().uuidString), "message": .string(answer),
                                         "attachments": .array([])]) { _, v in v })
            guard self.identity == identity else { return }
            questionSubmissions[id] = .sent
            if self.route == route { try await refreshConversation() }
            if isDemo { try await refreshSidebar() }
        } catch {
            guard self.identity == identity else { return }
            if questionSubmissions[id] == nil,
               let remote = error as? RemoteError, [.outcomeUnknown, .disconnected].contains(remote) {
                questionSubmissions[id] = .uncertain
            }
            self.error = error.localizedDescription
        }
    }

    func send(_ route: ConversationRoute) async {
        guard canActOnSnapshot, self.route == route, let snapshot, snapshot.writable, !busy.contains(route) else { return }
        let identity = self.identity
        let message = (drafts[route] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let attached = photos[route] ?? []
        guard !message.isEmpty || !attached.isEmpty else { return }
        busy.insert(route)
        defer { busy.remove(route) }
        do {
            var args = route.args
            args["request_id"] = .string(UUID().uuidString)
            args["message"] = .string(message)
            args["attachments"] = .array(attached.map { .string($0.path) })
            _ = try await connection.call(snapshot.running ? "native_conversation_enqueue" : "native_conversation_send",
                                          project: route.project, args: args)
            guard self.identity == identity else { return }
            // A typed replacement draft must survive an in-flight response.
            if (drafts[route] ?? "").trimmingCharacters(in: .whitespacesAndNewlines) == message { drafts[route] = "" }
            photos[route]?.removeAll { photo in attached.contains { $0.id == photo.id } }
            if self.route == route { try await refreshConversation() }
        } catch { if self.identity == identity { self.error = error.localizedDescription } }
    }
    func action(_ command: String, route: ConversationRoute, extra: [String: RemoteValue] = [:]) async {
        guard canActOnSnapshot, self.route == route, !busy.contains(route) else { return }
        let identity = self.identity
        busy.insert(route)
        defer { busy.remove(route) }
        do {
            _ = try await connection.call(command, project: route.project, args: route.args.merging(extra) { _, new in new })
            guard self.identity == identity else { return }
            if self.route == route { try await refreshConversation() }
            if isDemo { try await refreshSidebar() }
        } catch { if self.identity == identity { self.error = error.localizedDescription } }
    }
    func earlier() async {
        guard let route, let snapshot, history.cursor != .null, !loadingEarlier else { return }
        loadingEarlier = true
        defer { loadingEarlier = false }
        let cursor = history.cursor
        let identity = self.identity
        do {
            let args = route.args.merging(["before_seq": cursor]) { _, v in v }
            let result: RemoteValue
            if connection.state == .online {
                result = try await read("native_conversation_snapshot", project: route.project, args: args)
            } else if let cached = await cachedRead("native_conversation_snapshot", project: route.project, args: args) {
                result = cached.value
            } else { throw RemoteError.service("这些更早的消息尚未缓存，请联网后加载。") }
            let page = try RemoteSnapshot(result, route: route)
            guard self.route == route, self.identity == identity else { return }
            guard page.epoch == snapshot.epoch else { throw RemoteError.service("历史缓存已过期，请联网刷新此对话。") }
            history.prepend(page, expectedEpoch: snapshot.epoch, expectedBoundary: snapshot.before, requestedCursor: cursor)
        } catch { if self.identity == identity { self.error = error.localizedDescription } }
    }
    func upload(_ data: Data, route: ConversationRoute) async {
        let identity = self.identity
        guard (photos[route]?.count ?? 0) < 4 else { error = "一条消息最多添加 4 张图片。"; return }
        do {
            let jpeg = try PhotoPreparation.jpeg(data)
            let result = try await connection.call("remote_attach_image", project: route.project,
                args: route.args.merging(["base64": .string(jpeg.base64EncodedString())]) { _, v in v })
            guard self.identity == identity else { return }
            guard !result["path"].string.isEmpty else { throw RemoteError.incompatibleResponse }
            photos[route, default: []].append(AttachedPhoto(path: result["path"].string, data: jpeg))
        } catch { if self.identity == identity { self.error = error.localizedDescription } }
    }
    func openInbox(_ entry: RemoteValue) {
        selectedProject = entry["project_id"].string
        selectedSession = entry["id"].string
        sheet = nil
    }

    func projectName(_ id: String) -> String {
        projects.first { $0["id"].string == id }?["name"].string ?? id
    }

    func sessionName(_ id: String, project: String) -> String {
        if selectedProject == project, let session = sessions.first(where: { $0["id"].string == id }) {
            return session["title"].string.isEmpty ? "未命名对话" : session["title"].string
        }
        return "对话 · \(id)"
    }

    func openResearchConversation(_ entry: RemoteResearchEntry, project: String) {
        guard connection.state == .online, let route = entry.route(project: project) else { return }
        selectedProject = route.project
        selectedSession = route.session
        sheet = nil
    }

    func dismissTopSurface() {
        if notebookLibraryPresented { notebookLibraryPresented = false }
        else if notebookCameraRecord != nil { notebookCameraRecord = nil }
        else if case .notebook? = sheet, !notebookPath.isEmpty { notebookPath.removeLast() }
        else if case .research? = sheet, !researchPath.isEmpty { researchPath.removeLast() }
        else if case .checklist? = sheet, !checklistPath.isEmpty { checklistPath.removeLast() }
        else { sheet = nil }
    }
}

enum PhotoPreparation {
    /// Downsample before decoding a full-size camera image, then fit the same
    /// 560 KiB budget as remote.js (base64 nested inside an encrypted frame).
    static func jpeg(_ data: Data) throws -> Data {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { throw RemoteError.service("无法读取图片。") }
        for side in [1600, 1120, 784, 548, 400] {
            let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: side]
            guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { continue }
            let size = CGSize(width: CGFloat(thumbnail.width), height: CGFloat(thumbnail.height))
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1; format.opaque = true
            let image = UIGraphicsImageRenderer(size: size, format: format).image { context in
                UIColor.white.setFill(); context.fill(CGRect(origin: .zero, size: size))
                UIImage(cgImage: thumbnail).draw(in: CGRect(origin: .zero, size: size))
            }
            if let jpeg = image.jpegData(compressionQuality: 0.8), jpeg.count <= 560 * 1024 { return jpeg }
        }
        throw RemoteError.service("无法将图片缩小到上传限制内。")
    }
}
