import SwiftUI
import WispProjectBrowser

struct ProjectFolder: Codable, Identifiable, Equatable {
    let id: String
    let name: String
}

struct SessionSection: Equatable, Identifiable {
    var id: String { title }
    let title: String
    let sessions: [BrowserSession]
}

enum SessionArrangement {
    static func sections(_ sessions: [BrowserSession], folders: [ProjectFolder], sort: String, group: String) -> [SessionSection] {
        let ordered = sessions.sorted { left, right in
            if sort == "name" {
                let order = left.title.localizedStandardCompare(right.title)
                if order != .orderedSame { return order == .orderedAscending }
                return left.id < right.id
            }
            if left.ts != right.ts { return left.ts > right.ts }
            return left.id < right.id
        }
        switch group {
        case "folder":
            var sections = folders.map { folder in
                SessionSection(title: folder.name, sessions: ordered.filter { $0.folderID == folder.id })
            }
            let ungrouped = ordered.filter { session in session.folderID == nil || !folders.contains { $0.id == session.folderID } }
            if !ungrouped.isEmpty || sections.isEmpty {
                sections.append(SessionSection(title: "未分组", sessions: ungrouped))
            }
            return sections
        case "date":
            let formatter = DateFormatter()
            formatter.dateStyle = .medium
            formatter.timeStyle = .none
            var titles: [String] = []
            var grouped: [String: [BrowserSession]] = [:]
            for session in ordered {
                let title = formatter.string(from: Date(timeIntervalSince1970: TimeInterval(session.ts)))
                if grouped[title] == nil { titles.append(title) }
                grouped[title, default: []].append(session)
            }
            return titles.map { SessionSection(title: $0, sessions: grouped[$0] ?? []) }
        default:
            return [SessionSection(title: "会话", sessions: ordered)]
        }
    }
}

@MainActor
final class NativeSessionGroups: ObservableObject {
    @Published private(set) var folders: [ProjectFolder] = []
    @Published var sort = "newest"
    @Published var group = "none"
    @Published var selecting = false
    @Published var selected: Set<String> = []
    @Published var creating = false
    @Published var draft = ""
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published var menuPresented = false

    func sections(_ sessions: [BrowserSession]) -> [SessionSection] {
        SessionArrangement.sections(sessions, folders: folders, sort: sort, group: group)
    }

    func dismissCreate() {
        guard !busy else { return }
        creating = false
    }

    func load(_ client: any NativeConversationQuerying, projectID: String) async {
        do {
            let value = try await client.invoke("native_project_folders", args: [:], projectID: projectID)
            let data = try JSONEncoder().encode(value)
            folders = try JSONDecoder().decode([ProjectFolder].self, from: data)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func create(_ client: any NativeConversationQuerying, projectID: String) async {
        let name = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !busy else { return }
        guard !name.isEmpty else { error = "请填写分组名称。"; return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            _ = try await client.invoke("native_project_folder_create", args: ["name": .string(name)], projectID: projectID)
            draft = ""
            creating = false
            await load(client, projectID: projectID)
        } catch {
            self.error = "分组未能确认创建，不会自动重试。\n" + error.localizedDescription
        }
    }

    func moveSelected(_ client: any NativeConversationQuerying, projectID: String, folderID: String?) async {
        guard !busy, !selected.isEmpty else { return }
        busy = true
        error = nil
        defer { busy = false }
        let ids = selected
        do {
            for id in ids {
                _ = try await client.invoke(
                    "native_project_session_move",
                    args: ["session_id": .string(id), "folder_id": folderID.map(SettingsValue.string) ?? .null],
                    projectID: projectID)
            }
            selected = []
            selecting = false
        } catch {
            self.error = "移动未能确认，不会自动重试。\n" + error.localizedDescription
        }
    }
}
