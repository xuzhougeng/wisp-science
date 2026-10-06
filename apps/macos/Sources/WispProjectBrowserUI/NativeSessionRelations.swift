import SwiftUI
import WispProjectBrowser

/// Resolve only advertised conversations in the selected project. Missing
/// sources, foreign rows and malformed self-links never become navigation.
struct NativeSessionRelations {
    let source: BrowserSession?
    let parent: BrowserSession?
    let missingParent: Bool
    let branches: [BrowserSession]
    let siblings: [BrowserSession]
    let subagents: [BrowserSession]

    init(selected: BrowserSession, sessions: [BrowserSession]) {
        var visible: [String: BrowserSession] = [:]
        for session in sessions where session.projectID == selected.projectID && session.status != "deleted" {
            if visible[session.id] == nil { visible[session.id] = session }
        }
        let current = visible[selected.id]
        source = current
        let parentID = current?.branchState == "orphaned" ? nil : current?.branchedFrom ?? current?.dispatchedFrom
        parent = parentID != selected.id ? parentID.flatMap { visible[$0] } : nil
        missingParent = parentID != nil && parent == nil
        func ordered(_ predicate: (BrowserSession) -> Bool) -> [BrowserSession] {
            guard current != nil else { return [] }
            return visible.values.filter { $0.id != selected.id && predicate($0) }.sorted {
                $0.ts == $1.ts ? $0.id < $1.id : $0.ts > $1.ts
            }
        }
        branches = ordered { $0.branchedFrom == selected.id && $0.branchState != "orphaned" }
        siblings = ordered { current?.branchState != "orphaned" && current?.branchedFrom != nil && $0.branchedFrom == current?.branchedFrom && $0.id != parentID && $0.branchState != "orphaned" }
        subagents = ordered { $0.dispatchedFrom == selected.id && $0.branchedFrom != selected.id }
    }
    func canOpen(_ id: String) -> Bool {
        source != nil && ([parent].compactMap { $0 } + branches + siblings + subagents).contains { $0.id == id }
    }
    static func stateLabel(_ session: BrowserSession) -> String? {
        switch session.branchState {
        case "active": return "活动分支"
        case "merged": return "已合并分支"
        case "orphaned": return "来源已失效的分支"
        case .some: return "分支"
        case nil: return session.dispatchedFrom == nil ? nil : "子代理会话"
        }
    }
}

struct NativeSessionRelationsSheet: View {
    let selected: BrowserSession
    let sessions: [BrowserSession]
    let close: () -> Void
    let open: (String) -> Void
    private var relations: NativeSessionRelations { .init(selected: selected, sessions: sessions) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("会话关系")).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
            Text(relations.source?.title ?? selected.title).font(.subheadline).lineLimit(2)
            Text(localized("查看来源会话、同源分支和子会话。打开记录会保留当前输入草稿。"))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if relations.source == nil {
                        Text(localized("当前会话记录不可用，请刷新项目。"))
                    } else {
                        if relations.source?.branchState == "orphaned" { Text(localized("此分支的检查点已失效，仍可查看保存的记录。")) .font(.caption).foregroundStyle(.secondary) }
                        if let parent = relations.parent { section("来源会话", rows: [parent]) }
                        if relations.missingParent { Text(localized("来源会话已不在当前项目的可用记录中。")) .font(.caption).foregroundStyle(.secondary) }
                        if !relations.siblings.isEmpty { section("同源分支", rows: relations.siblings) }
                        if !relations.branches.isEmpty { section("此会话的分支", rows: relations.branches) }
                        if !relations.subagents.isEmpty { section("子代理会话", rows: relations.subagents) }
                        if relations.parent == nil && !relations.missingParent && relations.source?.branchState != "orphaned" && relations.siblings.isEmpty && relations.branches.isEmpty && relations.subagents.isEmpty {
                            Text(localized("此会话还没有来源或子会话。")) .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 520, maxWidth: 680, minHeight: 280, idealHeight: 460, maxHeight: 740)
            .background(NativeSettingsEscape(close: close))
    }
    private func section(_ title: String, rows: [BrowserSession]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(localized(title)).font(.subheadline).bold()
            ForEach(rows) { row in
                Button {
                    guard relations.canOpen(row.id) else { return }; open(row.id)
                } label: {
                    HStack(alignment: .top, spacing: 10) {
                        WispIcon(name: row.dispatchedFrom == nil ? "fork" : "chat", size: 16)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(row.title).lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                            if let state = NativeSessionRelations.stateLabel(row) { Text(localized(state)).font(.caption).foregroundStyle(.secondary) }
                        }
                        WispIcon(name: "chevron-right", size: 12)
                    }.padding(10)
                }.buttonStyle(.plain).accessibilityIdentifier("related-session-\(row.id)")
            }
        }
    }
}
