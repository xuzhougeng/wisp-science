import SwiftUI
import WispProjectBrowser

struct PublicationRecord: Codable, Identifiable, Equatable {
    var id: String
    var projectID: String
    var title: String
    var description: String
    enum CodingKeys: String, CodingKey {
        case id, title, description
        case projectID = "project_id"
    }
}

struct PublicationRevisionRecord: Codable, Equatable {
    var id: String
    var label: String
    var state: String
    var revisionNumber: Int64
    enum CodingKeys: String, CodingKey {
        case id, label, state
        case revisionNumber = "revision_number"
    }
}

struct PublicationItemRecord: Codable, Identifiable, Equatable {
    var id: String
    var title: String
    var kind: String
    var ordinal: Int64
}

struct PublicationWorkspaceRecord: Codable, Equatable {
    var publications: [PublicationRecord]
    var publication: PublicationRecord?
    var revision: PublicationRevisionRecord?
    var items: [PublicationItemRecord]
}

struct PublicationDraft: Equatable {
    var title = ""
    var description = ""
    var revisionLabel = ""
}

enum NativePublicationCommand {
    static let read = "native_publication_workspace"
    static let create = "native_publication_create"
}

@MainActor
final class NativePublicationModel: ObservableObject {
    @Published var presented = false
    @Published var draft = PublicationDraft()
    @Published private(set) var projectID: String?
    @Published private(set) var workspace = PublicationWorkspaceRecord(publications: [], publication: nil, revision: nil, items: [])
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    func open(projectID: String) {
        self.projectID = projectID
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

    var canCreate: Bool {
        !busy && !draft.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty else { return }
        await send(client, command: NativePublicationCommand.read, args: [:], projectID: projectID, clearDraft: false)
    }

    func create(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty, !busy else { return }
        let title = draft.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, !label.isEmpty else {
            error = "请填写论文标题和版本标签。"
            return
        }
        await send(
            client,
            command: NativePublicationCommand.create,
            args: [
                "title": .string(draft.title),
                "description": .string(draft.description),
                "revision_label": .string(draft.revisionLabel),
            ],
            projectID: projectID,
            clearDraft: true)
    }

    private func send(_ client: any NativeSettingsQuerying, command: String, args: [String: SettingsValue], projectID: String, clearDraft: Bool) async {
        let current = UUID()
        generation = current
        busy = true
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(command, args: args, projectID: projectID)
            guard generation == current, presented, self.projectID == projectID else { return }
            workspace = try JSONDecoder().decode(PublicationWorkspaceRecord.self, from: JSONEncoder().encode(value))
            if clearDraft { draft = PublicationDraft() }
            error = nil
        } catch {
            guard generation == current, presented, self.projectID == projectID else { return }
            let lead = command == NativePublicationCommand.create ? "论文证据未能确认创建，不会自动重试。\n" : "论文证据未能确认读取，不会自动重试。\n"
            self.error = lead + error.localizedDescription
        }
    }
}

struct NativePublicationColumn: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var publication: NativePublicationModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("论文证据").font(.headline)
                Spacer()
                Button("返回对话") { publication.dismiss() }
            }
            if let error = publication.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            if publication.workspace.publications.isEmpty {
                Text("还没有论文。填写标题和版本后创建第一篇。").foregroundStyle(.secondary)
                TextField("论文标题", text: $publication.draft.title).textFieldStyle(.roundedBorder).disabled(publication.busy)
                TextField("说明", text: $publication.draft.description).textFieldStyle(.roundedBorder).disabled(publication.busy)
                TextField("版本标签", text: $publication.draft.revisionLabel).textFieldStyle(.roundedBorder).disabled(publication.busy)
                Button(publication.busy ? "正在创建…" : "创建论文") { Task { await publication.create(model.calendarClient()) } }
                    .disabled(!publication.canCreate)
            } else if let paper = publication.workspace.publication {
                Text(paper.title).font(.title3)
                if let revision = publication.workspace.revision {
                    Text("\(revision.label) · \(revision.state)").font(.caption).foregroundStyle(.secondary)
                }
                Text(paper.description).font(.body)
                ForEach(publication.workspace.items) { item in
                    Text("\(item.kind) · \(item.title)")
                }
            }
            Spacer(minLength: 0)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(NativeSettingsEscape(enabled: !publication.busy) { publication.dismiss() })
        .task(id: publication.projectID ?? "") { await publication.reload(model.calendarClient()) }
    }
}
