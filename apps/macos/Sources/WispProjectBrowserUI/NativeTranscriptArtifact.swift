import Foundation
import WispProjectBrowser

/// Read-only projections of completed table/formula blocks in the displayed page.
/// These are not registered files and must never be written to the artifact store.
struct NativeTranscriptArtifact: Identifiable, Equatable {
    let id: String
    let kind: String
    let title: String
    let source: String

    static func collect(_ items: [ConversationItem]) -> [Self] {
        var result: [Self] = []
        var counts: [String: Int] = [:]
        for (message, item) in items.enumerated() where item.role == "assistant" {
            let lines = item.text.components(separatedBy: .newlines)
            var i = 0
            var fence: (Character, Int)?
            func append(_ kind: String, _ source: String, at line: Int) {
                counts[kind, default: 0] += 1
                result.append(Self(id: "transcript:\(message):\(line):\(kind)", kind: kind,
                                   title: "\(kind == "table" ? "表格" : "公式") \(counts[kind]!)", source: source))
            }
            while i < lines.count {
                let line = lines[i].trimmingCharacters(in: .whitespaces)
                if let first = line.first, first == "`" || first == "~" {
                    let count = line.prefix(while: { $0 == first }).count
                    if count >= 3 {
                        if let open = fence {
                            if first == open.0 && count >= open.1 && line.dropFirst(count).trimmingCharacters(in: .whitespaces).isEmpty { fence = nil }
                        } else { fence = (first, count) }
                        i += 1; continue
                    }
                }
                if fence != nil || lines[i].hasPrefix("    ") || lines[i].hasPrefix("\t") { i += 1; continue }
                if line.hasPrefix("$$") {
                    let start = i
                    let rest = String(line.dropFirst(2))
                    if rest.hasSuffix("$$") {
                        let source = String(rest.dropLast(2)).trimmingCharacters(in: .whitespaces)
                        if !source.isEmpty { append("latex", source, at: start) }
                    } else {
                        var body = [rest]; var end = i + 1
                        while end < lines.count && !lines[end].trimmingCharacters(in: .whitespaces).hasSuffix("$$") { body.append(lines[end]); end += 1 }
                        if end < lines.count {
                            body.append(String(lines[end].trimmingCharacters(in: .whitespaces).dropLast(2)))
                            let source = body.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
                            if !source.isEmpty { append("latex", source, at: start) }
                            i = end
                        } else { break } // Incomplete streaming formula stays in the message.
                    }
                } else if line.contains("|"), i + 1 < lines.count, separator(lines[i + 1]) {
                    let start = i
                    i += 2
                    while i < lines.count && lines[i].contains("|") && !lines[i].trimmingCharacters(in: .whitespaces).isEmpty { i += 1 }
                    let candidate = lines[start..<i].joined(separator: "\n")
                    // Use the same CommonMark parser as the message renderer;
                    // pipes followed by a rule are not necessarily a table.
                    if let parsed = try? AttributedString(markdown: candidate, options: .init(interpretedSyntax: .full)),
                       parsed.runs.contains(where: { run in
                           (run.presentationIntent?.components ?? []).contains { if case .table = $0.kind { return true }; return false }
                       }) {
                        append("table", candidate, at: start)
                    }
                    continue
                }
                i += 1
            }
        }
        return result
    }
    private static func separator(_ line: String) -> Bool {
        let cells = line.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "|"))
            .components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
        return !cells.isEmpty && cells.allSatisfy {
            let dashes = $0.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
            return dashes.count >= 3 && dashes.allSatisfy { $0 == "-" }
        }
    }
}
