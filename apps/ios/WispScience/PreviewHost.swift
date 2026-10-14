#if DEBUG
import Foundation
import CryptoKit
import WispRemote

/// Only used when XCTest explicitly launches the Debug app with this flag.
/// No network, keychain, SSH host, model API, or real project is touched.
@MainActor enum PreviewHost {
    static func workspace() -> RemoteWorkspace? {
        let environment = ProcessInfo.processInfo.environment
        guard let mode = environment["WISP_UI_TESTING"] ?? (environment["WISP_TESTING"] == "1" ? "connection" : nil) else { return nil }
        let credential = MemoryCredential(connected: mode != "connection")
        let connection = RemoteConnection(socketFactory: { _ in
            Socket(running: mode == "running" || mode == "process-running", appearance: mode == "appearance" || mode == "interaction",
                   interaction: mode == "interaction", malformedUsage: mode == "usage-malformed",
                   process: mode == "process" || mode == "process-running", offline: mode == "cache-offline",
                   emptyInbox: mode == "inbox-empty")
        })
        let cache: RemoteReadCache?
        if mode.hasPrefix("cache-"), let id = environment["WISP_CACHE_TEST_ID"], let uuid = UUID(uuidString: id) {
            cache = RemoteReadCache(directory: FileManager.default.temporaryDirectory.appendingPathComponent("wisp-ui-cache-" + uuid.uuidString))
        } else { cache = nil }
        let offlineID = environment["WISP_OFFLINE_TEST_ID"].flatMap(UUID.init(uuidString:)) ?? UUID()
        let offline = FileManager.default.temporaryDirectory.appendingPathComponent("wisp-ui-research-" + offlineID.uuidString)
        return RemoteWorkspace(connection: connection, credentials: credential, preferences: nil, cache: cache, offlineDirectory: offline)
    }
    private struct MemoryCredential: RemoteCredentialStoring {
        let connected: Bool
        func load() throws -> RemoteConfiguration? {
            connected ? try RemoteConfiguration(address: "https://offline-fixture.example.test", code: "000102030405060708090a0b0c0d0e0f") : nil
        }
        func save(_ configuration: RemoteConfiguration) throws { }
        func remove() throws { }
    }
    private final class Socket: RemoteSocket {
        let hostOffline = false
        private let key = SymmetricKey(data: SHA256.hash(data: Data("wisp-remote/key/v1".utf8) + Data(0..<16)))
        private var frames: [String] = []
        private var waiter: CheckedContinuation<String, Error>?
        private var closed = false
        private var messages: [RemoteValue] = [.object(["role": .string("assistant"), "text": .string("# 样本质控\n\n检查完成。\n\n| 样本 | 结果 |\n| --- | --- |\n| A | 通过 |")])]
        private var sequence: UInt64 = 1
        private var running: Bool
        private let appearance: Bool
        private let interaction: Bool
        private let emptyInbox: Bool
        private var approved = false
        private let commands = ["list_projects", "remote_sessions", "native_conversation_snapshot", "native_conversation_create",
            "native_conversation_send", "native_conversation_enqueue", "native_conversation_queue_action", "native_conversation_stop",
            "native_conversation_approve", "native_conversation_seen", "native_conversation_inbox", "native_conversation_panel_file_directory",
            "native_conversation_panel_file_read", "remote_attach_image"]
        init(running: Bool, appearance: Bool, interaction: Bool, malformedUsage: Bool, process: Bool, offline: Bool, emptyInbox: Bool) {
            self.running = running
            self.appearance = appearance
            self.interaction = interaction
            self.emptyInbox = emptyInbox
            if interaction {
                messages = (1...40).map { index in
                    .object(["role": .string("assistant"), "text": .string("历史记录 \(index)\n\n这是一条较早的离线研究记录，用于验证长对话滚动和返回最新消息。")])
                } + messages
            }
            if appearance || malformedUsage {
                messages.append(.object(["role": .string("usage"), "text": .string(malformedUsage ? "invalid usage payload" :
                    #"{"input":323738,"output":1110,"reasoning":0,"cached":112128,"ctx_tokens":79761,"max_context":272000,"context_usage":{"conversation":59150}}"#)]))
            }
            if process {
                messages = [
                    .object(["role": .string("user"), "text": .string("生成跨物种表达热图")]),
                    .object(["role": .string("reasoning"), "text": .string("先检查表达矩阵。")]),
                    .object(["role": .string("assistant"), "text": .string("正在整理表达数据并生成图片。")]),
                    .object(["role": .string("tool"), "tool_name": .string("shell"), "ok": .bool(true),
                             "input": .string("python heatmap.py"), "text": .string("Heatmap generated.")]),
                ]
                messages += ["OG_log2FC_heatmap.pdf", "OG_log2FC_heatmap.png", "OG_log2FC_heatmap.svg", "heatmap_metadata.json"].map {
                    .object(["role": .string("file_changed"), "text": .string("analysis/en-expression/output/figures/" + $0)])
                }
                messages += [
                    .object(["role": .string("tool"), "tool_name": .string("view_image"), "ok": .bool(true), "text": .string("Image checked.")]),
                    .object(["role": .string("tool"), "tool_name": .string("read"), "ok": .bool(true), "text": .string("Matrix loaded.")]),
                    .object(["role": .string("tool"), "tool_name": .string("read"), "ok": .bool(false), "text": .string("Optional metadata was not found.")]),
                    .object(["role": .string("tool"), "tool_name": .string("edit"), "ok": .bool(true), "text": .string("README updated.")]),
                    .object(["role": .string("file_changed"), "text": .string("analysis/en-expression/README.md")]),
                ]
                if !running {
                    messages.append(.object(["role": .string("assistant"), "text": .string("# 表达分析完成\n\n热图和报告已生成，可在文件中查看。")]))
                }
            }
            frames = [seal(.object(["type": .string("hello"), "nonce": .string("preview"), "name": .string("离线测试主机"),
                "version": .string("fixture"), "commands": .array(commands.map(RemoteValue.string))]))]
            if offline { frames = [] }
        }
        func send(_ text: String) async throws {
            let data = try AES.GCM.open(AES.GCM.SealedBox(combined: Data(base64Encoded: text)!), using: key, authenticating: RemoteCrypto.clientToHost)
            let request = try JSONDecoder().decode(RemoteValue.self, from: data)
            let result = response(request)
            push(seal(.object(["type": .string("response"), "response": .object([
                "schema": .string("wisp.native-settings.v1"), "id": request["id"], "result": result, "error": .null,
            ])])))
        }
        func receive() async throws -> String {
            if closed { throw RemoteError.disconnected }
            if !frames.isEmpty { return frames.removeFirst() }
            return try await withCheckedThrowingContinuation { waiter = $0 }
        }
        func close() { closed = true; waiter?.resume(throwing: RemoteError.disconnected); waiter = nil }
        private func push(_ frame: String) {
            if let waiter { self.waiter = nil; waiter.resume(returning: frame) } else { frames.append(frame) }
        }
        private func seal(_ value: RemoteValue) -> String {
            try! AES.GCM.seal(JSONEncoder().encode(value), using: key, authenticating: RemoteCrypto.hostToClient).combined!.base64EncodedString()
        }
        private func response(_ request: RemoteValue) -> RemoteValue {
            let args = request["args"]
            switch request["command"].string {
            case "list_projects": return .array([.object(["id": .string("project-a"), "name": .string("RNA-seq 研究"), "session_count": .integer(1)])])
            case "native_conversation_inbox" where emptyInbox: return .array([])
            case "remote_sessions", "native_conversation_inbox": return .array([.object([
                "id": .string("session-a"), "title": .string(interaction ? "/data1/home/research/project/PlantRootAtlas/跨物种表达分析" : "样本质控"), "project_id": .string("project-a"),
                "project_name": .string("RNA-seq 研究"), "status": .string(running ? "running" : "needs_you"),
            ])])
            case "native_conversation_snapshot":
                return .object(["schema": .string("wisp.native-conversations.v1"), "epoch": .string("preview"), "sequence": .unsigned(sequence),
                    "project_id": request["project_id"], "session_id": args["session_id"], "items": .array(messages),
                    "running": .bool(running), "stopping": .bool(false), "read_only": .bool(false), "next_before_seq": .null,
                    "follow_ups": .array(appearance ? [
                        .string("对比图中 X26T3 与 big403/small403 在三个基因位点上有无可见的 SNP/Indel 差异，能否用 bcftools 做一下变异检测？"),
                        .string("能否把这三个基因的截图放大到仅基因本体区域（去掉侧翼），并生成 PDF 版本用于报告？"),
                        .string("其余两个 xhc 样品（big14_1/small14_1）是否也需要加入对比图并做同样的 QC 汇总？"),
                    ] : [.string("查看统计结果")]),
                    "run_cards": .array(appearance ? [
                        .object(["id": .string("probe"), "title": .string("Probe gene coords via exon rows, envs, disk"),
                                 "status": .string("succeeded"), "stdout_tail": .string("Gene coordinates found.")]),
                        .object(["id": .string("merge"), "title": .string("Rebuild merged BAM (header+body via sam)"),
                                 "status": .string("failed"), "command": .string("samtools merge merged.bam a.bam b.bam"),
                                 "stderr_tail": .string("Input file a.bam was not found.")]),
                        .object(["id": .string("download"), "title": .string("Download locus from CPU3"),
                                 "status": .string("succeeded"), "stdout_tail": .string("Download complete.")]),
                    ] : []),
                    "approvals": .array(running && !approved ? [.object(["approval_id": .string("approval-a"), "frame_id": args["session_id"],
                        "tool": .string("shell"), "message": .string("运行此命令？"), "preview": .string("python qc.py")])] : []),
                ])
            case "native_conversation_create": return .string("session-a")
            case "native_conversation_send", "native_conversation_enqueue":
                messages.append(.object(["role": .string("user"), "text": args["message"]]))
                messages.append(.object(["role": .string("assistant"), "text": .string("已收到离线测试消息。")]))
                sequence += 1
                return .null
            case "native_conversation_stop": running = false; sequence += 1; return .null
            case "native_conversation_approve": approved = true; sequence += 1; return .null
            case "native_conversation_panel_file_directory":
                return .object(["schema": .string("wisp.native-files.v1"), "project_id": request["project_id"], "session_id": args["session_id"],
                    "path": args["path"], "entries": .array([.object(["name": .string("report.md"), "is_dir": .bool(false), "size": .integer(16)])])])
            case "native_conversation_panel_file_read":
                return .object(["schema": .string("wisp.native-files.v1"), "project_id": request["project_id"], "session_id": args["session_id"],
                    "content": .object(["text": .string("# 结果\n\n质控通过。"), "truncated": .bool(false)])])
            default: return .null
            }
        }
    }
}
#endif
