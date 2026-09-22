import SwiftUI
import WispProjectBrowser

enum IssueReportDraft {
    static let repository = "xuzhougeng/wisp-science"
    static let issueBase = "https://github.com/xuzhougeng/wisp-science/issues/new"

    static func offered(activeSessionID: String?) -> Bool {
        activeSessionID?.isEmpty == false
    }

    static func modelLabel(models: [SettingsValue], modelID: String?) -> String {
        if let modelID, let label = modelID.split(separator: ":", maxSplits: 1).dropFirst().first, modelID.hasPrefix("acp:"), !label.isEmpty {
            return String(label)
        }
        func text(_ row: SettingsValue) -> String {
            let label = row["label"].string
            return label.isEmpty ? row["model"].string : label
        }
        if let modelID, let row = models.first(where: { $0["id"].string == modelID }) {
            let label = text(row)
            if !label.isEmpty { return label }
        }
        if let row = models.first(where: { $0["active"].bool }) {
            let label = text(row)
            if !label.isEmpty { return label }
        }
        if let row = models.first {
            let label = text(row)
            if !label.isEmpty { return label }
        }
        return "not configured"
    }

    static func prompt(version: String, os: String, arch: String, model: String, startup: String) -> String {
        let recorded = startup.trimmingCharacters(in: .whitespacesAndNewlines)
        let startupText = recorded.isEmpty ? "未记录" : recorded
        return """
        请帮我向 \(repository) 提交一个 GitHub issue。

        【已自动采集，请勿向我索要 API key、transcript、项目文件、环境变量、用户名或绝对路径】
        - Wisp 版本：\(version)
        - OS / 架构：\(os) / \(arch)
        - 模型配置：\(model)
        - 启动耗时：\(startupText)

        请用中文逐条引导我说明：发生了什么、复现步骤、预期与实际行为，以及我知道的 Run ID 或错误信息。
        若启动很慢或长时间白屏，提醒我在 Windows 正式版可把日志发给维护者：%APPDATA%\\science.wisp-science\\wisp-science\\logs\\wisp.log（上次启动：wisp.previous.log）。
        信息足够后，给出简短 issue 标题和 Markdown 正文（含问题描述、复现步骤、预期、实际、环境等小节），并提供预填链接：\(issueBase)?title=...&body=...
        提醒截图需在 GitHub 页面手动附加，Wisp 不会上传截图。
        """
    }
}

@MainActor
final class NativeIssueReport: ObservableObject {
    @Published private(set) var error: String?

    func prepare(model: ProjectBrowserModel, conversation: NativeConversationModel, client: any NativeSettingsQuerying) async {
        guard let projectID = model.activeProjectID, IssueReportDraft.offered(activeSessionID: model.activeSessionID) else {
            error = "请先打开一个会话。不会自动发送。"
            return
        }
        let sessionID = model.activeSessionID
        do {
            let bootstrap = try await client.invoke("get_bootstrap_status", args: [:], projectID: projectID)
            guard model.activeProjectID == projectID, model.activeSessionID == sessionID else { return }
            let text = IssueReportDraft.prompt(
                version: bootstrap["app_version"].string,
                os: bootstrap["os"].string,
                arch: bootstrap["arch"].string,
                model: IssueReportDraft.modelLabel(models: conversation.models, modelID: conversation.snapshot?.model_id),
                startup: bootstrap["startup"].string)
            guard conversation.replaceDraft(text) else { return }
            error = nil
        } catch {
            guard model.activeProjectID == projectID, model.activeSessionID == sessionID else { return }
            self.error = "反馈草稿未能确认读取，不会自动重试。\n" + error.localizedDescription
        }
    }
}
