import SwiftUI
import WispProjectBrowser

struct NativeQuestion {
    enum State { case pending, answered, expired }
    struct Option {
        let label: String
        let description: String
        var answer: String { description.isEmpty ? label : label + "\n\n选项说明：" + description }
    }
    let title: String
    let options: [Option]
    let allowFreeform: Bool
    let requestID: String?
    let state: State
    init?(_ text: String) {
        guard let data = text.data(using: .utf8), let value = try? JSONDecoder().decode(SettingsValue.self, from: data),
              case .object = value else { return nil }
        title = value["question"].string
        options = value["options"].array.compactMap { value in
            let label = value["label"].string.trimmingCharacters(in: .whitespacesAndNewlines)
            return label.isEmpty ? nil : Option(label: label, description: value["description"].string.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        allowFreeform = value["allow_freeform"] == .null || value["allow_freeform"].bool || options.isEmpty
        requestID = value["request_id"].string.isEmpty ? nil : value["request_id"].string
        state = value["status"].string == "expired" ? .expired : value["status"].string == "answered" ? .answered : .pending
    }
}

struct NativeQuestionTarget: Equatable {
    let session: String
    let index: Int
    let text: String
    let generation: UUID
    func isSameQuestion(as other: Self) -> Bool { session == other.session && index == other.index && text == other.text }
}

struct NativeQuestionCard: View {
    @ObservedObject var conversation: NativeConversationModel
    let target: NativeQuestionTarget
    let question: NativeQuestion
    @State private var freeform = ""
    private var pending: Bool { conversation.questionState(target) == .pending }
    private var enabled: Bool { question.requestID == nil ? conversation.canStageQuestion(target) : conversation.canAnswerAcpQuestion(target) }
    private func answer(_ text: String) async -> Bool {
        if question.requestID != nil { return await conversation.answerAcpQuestion(text, target: target) }
        return conversation.stageQuestionAnswer(text, target: target)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(question.title).font(WispDesign.font(size: 14)).textSelection(.enabled)
            if pending {
                ForEach(Array(question.options.enumerated()), id: \.offset) { _, option in
                    Button { Task { _ = await answer(option.answer) } } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(option.label)
                            if !option.description.isEmpty { Text(option.description).font(.caption).foregroundStyle(.secondary) }
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.disabled(!enabled)
                }
                if question.allowFreeform {
                    HStack {
                        TextField("输入其他回答", text: $freeform).textFieldStyle(.roundedBorder)
                            .accessibilityLabel("问题自由回答")
                        Button(question.requestID == nil ? "填入回答" : "发送回答") {
                            let text = freeform
                            Task { if await answer(text), freeform == text { freeform = "" } }
                        }.disabled(!enabled || freeform.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }.disabled(!enabled)
                }
                Text(question.requestID == nil ? "回答会加入输入框并保留已有草稿，发送后继续。" : "回答会直接发送给 ACP Agent，保留输入框中的草稿。")
                    .font(WispDesign.font(size: 11)).foregroundStyle(.secondary)
            } else {
                Text(conversation.questionState(target) == .answered ? "已回答" : "问题已过期")
                    .font(WispDesign.font(size: 11)).foregroundStyle(.secondary)
            }
        }.accessibilityIdentifier("conversation-question")
    }
}
