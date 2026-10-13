import Foundation
import WispRemote

/// Bundled, synthetic research content. All interactions stay in memory;
/// this provider cannot execute commands, open sockets or modify project files.
@MainActor final class DemoDataset {
    static let commands: Set<String> = [
        "list_projects", "remote_sessions", "native_conversation_snapshot", "native_conversation_send",
        "native_conversation_approve", "native_conversation_seen", "native_conversation_inbox",
        "native_conversation_panel_file_directory", "native_conversation_panel_file_read", "native_conversation_image",
    ]
    private let root: URL
    private let manifest: RemoteValue
    private let epoch = UUID().uuidString
    private var sequence: UInt64 = 1
    private var messages: [String: [RemoteValue]] = [:]
    private var decisions: [String: Bool] = [:]
    private var seen: Set<String> = []

    init(bundle: Bundle = .main) throws {
        guard let root = bundle.url(forResource: "DemoData", withExtension: nil) else {
            throw RemoteError.service("演示数据缺失，请重新安装 App。")
        }
        self.root = root
        manifest = try JSONDecoder().decode(RemoteValue.self, from: Data(contentsOf: root.appendingPathComponent("manifest.json")))
        guard manifest["schema"].string == "wisp.ios-demo.v1", !manifest["sessions"].array.isEmpty else {
            throw RemoteError.service("演示数据版本不可用。")
        }
    }

    var projectID: String { manifest["project"]["id"].string }

    func call(_ command: String, project: String?, args: [String: RemoteValue]) throws -> RemoteValue {
        guard Self.commands.contains(command) else { throw RemoteError.unsupported }
        if command == "list_projects" {
            return .array([.object([
                "id": .string(projectID), "name": manifest["project"]["name"],
                "session_count": .integer(Int64(manifest["sessions"].array.count)),
                "needs_you_count": .integer(Int64(sessionRows.filter { $0["status"].string == "needs_you" }.count)),
            ])])
        }
        guard project == projectID else { throw RemoteError.service("演示项目不存在。") }
        if command == "remote_sessions" { return .array(sessionRows) }
        if command == "native_conversation_inbox" {
            return .array(sessionRows.filter { $0["status"].string == "needs_you" })
        }
        let sessionID = args["session_id"]?.string ?? ""
        guard let session = manifest["sessions"].array.first(where: { $0["id"].string == sessionID }) else {
            throw RemoteError.service("演示对话不存在。")
        }
        switch command {
        case "native_conversation_snapshot":
            let approvals = session["approvals"].array.filter { decisions[$0["approval_id"].string] == nil }
            return .object([
                "schema": .string("wisp.native-conversations.v1"), "project_id": .string(projectID),
                "session_id": .string(sessionID), "epoch": .string(epoch), "sequence": .unsigned(sequence),
                "items": .array(messages[sessionID] ?? session["items"].array), "run_cards": session["run_cards"],
                "approvals": .array(approvals), "follow_ups": session["follow_ups"],
                "running": .bool(false), "stopping": .bool(false), "read_only": .bool(false), "next_before_seq": .null,
                "queue": .object(["items": .array([])]),
            ])
        case "native_conversation_send":
            let text = (args["message"]?.string ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty, text.utf8.count <= 16_000 else {
                throw RemoteError.service("演示消息不能为空，且需少于 16 KB。")
            }
            let response = sessionID == "demo-options"
                ? "**本地预设回复 · 未调用 AI 或运行分析**\n\n已收到你的演示回答。你可以继续[查看质控报告](results/qc_report.md)，或在「演示设置」中重置数据，再体验其他选项。"
                : reply(to: text)
            try append([.object(["role": .string("user"), "text": .string(text)]),
                    .object(["role": .string("assistant"), "text": .string(response)])], to: session)
            return .null
        case "native_conversation_approve":
            let id = args["approval_id"]?.string ?? ""
            guard session["approvals"].array.contains(where: { $0["approval_id"].string == id }),
                  decisions[id] == nil, case .bool(let approved) = args["approved"] else {
                throw RemoteError.service("这条演示审批已处理或不存在。")
            }
            try append([.object(["role": .string("assistant"), "text": .string(approved
                ? "演示审批已批准。本次仅展示审批交互，没有运行命令或修改文件。你可以在项目文件中查看预先打包的质控报告。"
                : "演示审批已拒绝。没有运行命令或修改文件。可从演示设置重置数据，再次体验。")])], to: session)
            decisions[id] = approved
            return .null
        case "native_conversation_seen":
            seen.insert(sessionID)
            return .null
        case "native_conversation_panel_file_directory":
            let path = try normalized(args["path"]?.string ?? ".")
            let prefix = path == "." ? "" : path + "/"
            var entries: [String: RemoteValue] = [:]
            for file in manifest["files"].array {
                let name = file["path"].string
                guard name.hasPrefix(prefix) else { continue }
                let parts = name.dropFirst(prefix.count).split(separator: "/")
                guard let first = parts.first else { continue }
                let directory = parts.count > 1
                let size = directory ? 0 : try Data(contentsOf: fileURL(name)).count
                entries[String(first)] = .object(["name": .string(String(first)), "is_dir": .bool(directory), "size": .integer(Int64(size))])
            }
            guard path == "." || !entries.isEmpty else { throw RemoteError.service("演示目录不存在。") }
            return fileResponse(sessionID, content: ["path": .string(path), "entries": .array(entries.keys.sorted().compactMap { entries[$0] })])
        case "native_conversation_panel_file_read":
            let path = try normalized(args["path"]?.string ?? "")
            guard !path.hasSuffix(".png") else { throw RemoteError.service("请用图片预览打开此文件。") }
            let data = try Data(contentsOf: fileURL(path))
            guard let text = String(data: data, encoding: .utf8) else { throw RemoteError.service("无法读取演示文本。") }
            return fileResponse(sessionID, content: ["content": .object(["text": .string(text), "truncated": .bool(false)])])
        case "native_conversation_image":
            let path = try normalized(args["path"]?.string ?? "")
            guard path.hasSuffix(".png") else { throw RemoteError.service("演示图片不存在。") }
            return .object(["base64": .string(try Data(contentsOf: fileURL(path)).base64EncodedString()), "mime_type": .string("image/png")])
        default: throw RemoteError.unsupported
        }
    }

    private var sessionRows: [RemoteValue] {
        manifest["sessions"].array.map { session in
            let needsApproval = session["approvals"].array.contains { decisions[$0["approval_id"].string] == nil }
            let items = messages[session["id"].string] ?? session["items"].array
            let needsAnswer = items.enumerated().contains { index, item in
                RemoteQuestion(item, hasLaterUser: items.dropFirst(index + 1).contains { $0["role"].string == "user" })?.state == .pending
            }
            return .object(["id": session["id"], "title": session["title"], "project_id": .string(projectID),
                            "project_name": manifest["project"]["name"],
                            "status": .string(needsApproval || needsAnswer || !seen.contains(session["id"].string) ? "needs_you" : "idle")])
        }
    }
    private func append(_ items: [RemoteValue], to session: RemoteValue) throws {
        let id = session["id"].string
        let existing = messages[id] ?? session["items"].array
        guard existing.count + items.count <= 100 else {
            throw RemoteError.service("本次演示消息已满，请在演示设置中重置数据。")
        }
        messages[id] = existing + items
        sequence += 1
    }
    private func reply(to text: String) -> String {
        let prefix = "**本地预设回复 · 未调用 AI 或运行分析**\n\n"
        if text.contains("审批") || text.contains("流程") {
            return prefix + "从对话列表打开「分析流程与审批」，可以体验批准或拒绝操作。演示不会实际执行脚本；退出或重置后，交互记录会清除。"
        }
        return prefix + "这套合成数据包含 6 个样本，共 150.3 M read pairs，平均比对率约 96.0%。这些数值仅用于展示。\n\n[查看质控报告](results/qc_report.md) · [查看样本表](data/samples.csv)\n\n![演示样本比对率](results/qc_summary.png)\n\n连接你自己的 Wisp 主机后，才会由真实 Agent 处理你的问题。"
    }
    private func fileResponse(_ session: String, content: [String: RemoteValue]) -> RemoteValue {
        .object(content.merging(["schema": .string("wisp.native-files.v1"), "project_id": .string(projectID),
                                 "session_id": .string(session)]) { _, value in value })
    }
    private func normalized(_ path: String) throws -> String {
        let parts = path.split(separator: "/", omittingEmptySubsequences: true)
        guard !path.hasPrefix("/"), !parts.contains(".."), !path.contains(":") else {
            throw RemoteError.service("只能打开随 App 打包的演示文件。")
        }
        let value = parts.filter { $0 != "." }.joined(separator: "/")
        return value.isEmpty ? "." : value
    }
    private func fileURL(_ path: String) throws -> URL {
        guard manifest["files"].array.contains(where: { $0["path"].string == path }) else {
            throw RemoteError.service("演示文件不存在。")
        }
        return root.appendingPathComponent("files").appendingPathComponent(path)
    }
}
