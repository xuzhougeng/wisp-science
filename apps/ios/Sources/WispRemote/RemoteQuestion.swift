import Foundation

/// Presentation adapter for wisp-dto's parse_question_card / settle_question_cards.
/// The wire payload remains the existing JSON in a `question` transcript row.
public struct RemoteQuestion: Equatable, Sendable {
    public struct Option: Equatable, Sendable {
        public let label: String
        public let description: String
    }
    public enum State: Equatable, Sendable { case pending, answered, expired }
    public let question: String
    public let options: [Option]
    public let allowsFreeform: Bool
    public let isNative: Bool
    public let state: State

    public init?(_ item: RemoteValue, hasLaterUser: Bool = false) {
        guard item["role"].string == "question",
              let payload = try? JSONDecoder().decode(RemoteValue.self, from: Data(item["text"].string.utf8)),
              !payload["question"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        question = payload["question"].string
        options = payload["options"].array.compactMap { value in
            let label = value["label"].string.trimmingCharacters(in: .whitespacesAndNewlines)
            return label.isEmpty ? nil : Option(label: label, description: value["description"].string)
        }
        allowsFreeform = payload["allow_freeform"] != .bool(false) || options.isEmpty
        isNative = ["", "native"].contains(payload["source"].string) && payload["request_id"].string.isEmpty
        switch payload["status"].string {
        case "answered": state = .answered
        case "expired": state = .expired
        default: state = isNative && hasLaterUser ? .answered : .pending
        }
    }

    /// A choice is an ordinary user message for native questions. Preserve its
    /// description, as the desktop does, and optionally append the user's note.
    public func answer(option: Int?, freeform: String) -> String? {
        guard isNative, state == .pending else { return nil }
        let note = freeform.trimmingCharacters(in: .whitespacesAndNewlines)
        guard allowsFreeform || note.isEmpty else { return nil }
        var parts: [String] = []
        if let option {
            guard options.indices.contains(option) else { return nil }
            let choice = options[option]
            parts.append(choice.label)
            if !choice.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                parts.append("选项说明：" + choice.description)
            }
        }
        if !note.isEmpty { parts.append(parts.isEmpty ? note : "补充说明：" + note) }
        return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
    }
}
