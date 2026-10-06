import AppKit
import SwiftUI
import WispProjectBrowser

/// The source stays unchanged for editing. Reading has the same byte/line and
/// table-row bounds as WebView, even for older hosts or artifact previews.
struct NativeFileTextDocument {
    enum Kind { case text, markdown, delimited }
    let kind: Kind
    let source: String
    let clipped: Bool
    let table: NativeDelimitedTable?
    let tableError: String?
    var markdown: String { Self.withoutFrontMatter(source) }
    var rawDisplaySource: String { Self.clipLines(source) }
    var rawDisplayClipped: Bool { rawDisplaySource != source }

    init(_ content: NativePanelFileContent) {
        let ext = (content.path as NSString).pathExtension.lowercased()
        if ["md", "markdown", "rmd", "qmd"].contains(ext) || content.mime == "text/markdown" { kind = .markdown }
        else if ["csv", "tsv"].contains(ext) || ["text/csv", "text/tab-separated-values"].contains(content.mime) { kind = .delimited }
        else { kind = .text }
        let raw = content.text ?? ""
        var bytes = Array(raw.utf8.prefix(1024 * 1024))
        while String(bytes: bytes, encoding: .utf8) == nil { bytes.removeLast() }
        let bounded = String(decoding: bytes, as: UTF8.self)
        source = kind == .delimited ? bounded : Self.clipLines(bounded)
        clipped = source.utf8.count < raw.utf8.count
        if kind == .delimited {
            do {
                table = try NativeDelimitedTable.parse(source, separator: ext == "tsv" || content.mime == "text/tab-separated-values" ? 9 : 44, truncated: content.truncated || clipped)
                tableError = nil
            } catch { table = nil; tableError = localized("表格格式无法解析，请查看源文本。") }
        } else { table = nil; tableError = nil }
    }
    private static func clipLines(_ source: String) -> String {
        var lines = 0
        for index in source.utf8.indices where source.utf8[index] == 10 {
            lines += 1
            if lines == 8_000 { return String(source[..<source.utf8.index(after: index)]) }
        }
        return source
    }

    @MainActor func containsSelection(_ text: String) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        if source.contains(text) { return true }
        if kind == .markdown {
            return NativeMathContent.plainText(NativeMarkdownContent.render(markdown, saved: [], scheme: .light)).contains(text)
        }
        return table?.containsSelection(text) == true
    }

    static func withoutFrontMatter(_ source: String) -> String {
        guard source.hasPrefix("---\n") || source.hasPrefix("---\r\n") else { return source }
        let lines = source.components(separatedBy: "\n")
        var yaml = false
        for index in lines.indices.dropFirst() {
            let line = lines[index].trimmingCharacters(in: .newlines)
            if line == "---" || line == "..." {
                guard yaml else { return source }
                return lines.dropFirst(index + 1).joined(separator: "\n").replacingOccurrences(of: "\\A[\\r\\n]+", with: "", options: .regularExpression)
            }
            yaml = yaml || line.contains(":")
        }
        return source
    }
}

struct NativeDelimitedTable: Equatable {
    let headers: [String]
    let rows: [[String]]
    let omittedPartialRecord: Bool
    static let visibleRows = 500
    static let visibleColumns = 128
    var columnCount: Int { max(headers.count, rows.map(\.count).max() ?? 0) }
    var copyText: String { ([headers] + rows).map(Self.tsv).joined(separator: "\n") }
    static func tsv(_ cells: [String]) -> String {
        cells.map { $0.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ") }.joined(separator: "\t")
    }
    func selectedText(_ indexes: IndexSet) -> String {
        indexes.filter { rows.indices.contains($0) && $0 < Self.visibleRows }.map { Self.tsv(rows[$0]) }.joined(separator: "\n")
    }
    func containsSelection(_ text: String) -> Bool {
        if copyText.contains(text) { return true }
        let lines = text.components(separatedBy: "\n")
        let known = Set(rows.map(Self.tsv))
        return !lines.isEmpty && lines.allSatisfy { !$0.isEmpty && known.contains($0) }
    }
    enum ParseError: Error { case invalidQuote }
    static func parse(_ source: String, separator: UInt8, truncated: Bool = false) throws -> Self {
        var bytes = Array(source.utf8)
        if bytes.starts(with: [0xef, 0xbb, 0xbf]) { bytes.removeFirst(3) }
        var records: [[String]] = [], row: [String] = [], field: [UInt8] = []
        var quoted = false, afterQuote = false, quotedRecord = false, index = 0
        func finishField() { row.append(String(decoding: field, as: UTF8.self)); field = []; afterQuote = false }
        func finishRow() {
            finishField()
            if quotedRecord || row.count > 1 || row.contains(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) { records.append(row) }
            row = []; quotedRecord = false
        }
        while index < bytes.count {
            let byte = bytes[index]
            if quoted {
                if byte == 34 {
                    if index + 1 < bytes.count, bytes[index + 1] == 34 { field.append(34); index += 1 }
                    else { quoted = false; afterQuote = true }
                } else if byte == 13 {
                    field.append(10)
                    if index + 1 < bytes.count, bytes[index + 1] == 10 { index += 1 }
                } else { field.append(byte) }
            } else if byte == separator { finishField() }
            else if byte == 10 || byte == 13 {
                finishRow()
                if byte == 13, index + 1 < bytes.count, bytes[index + 1] == 10 { index += 1 }
            } else if byte == 34, field.isEmpty, !afterQuote { quoted = true; quotedRecord = true }
            else if afterQuote {
                if byte != 32 && byte != 9 { throw ParseError.invalidQuote }
            } else if byte == 34 { throw ParseError.invalidQuote }
            else { field.append(byte) }
            index += 1
        }
        let partial = quoted || !field.isEmpty || !row.isEmpty || afterQuote
        if !truncated {
            guard !quoted else { throw ParseError.invalidQuote }
            if partial { finishRow() }
        }
        return Self(headers: records.first ?? [], rows: Array(records.dropFirst()), omittedPartialRecord: truncated && partial)
    }
}

/// Resolve relative Markdown assets against this document, never the process's
/// current directory. SSH home paths stay on the remote host.
enum NativeFileImagePath {
    static func resolve(_ reference: String, document: String, remote: Bool) -> String? {
        guard let url = URL(string: reference), !reference.hasPrefix("//") else { return nil }
        let path: String
        if url.isFileURL {
            guard !remote, url.host == nil || url.host == "" || url.host == "localhost" else { return nil }
            path = url.path
        } else {
            guard url.scheme == nil else { return nil }
            path = url.path
        }
        guard !path.isEmpty, !path.contains("\0"), !path.contains("\n"), !path.contains("\r"), NativeMessageImageRequest.isImageFile(path) else { return nil }
        let joined = path.hasPrefix("/") || path.hasPrefix("~/") ? path : (document as NSString).deletingLastPathComponent + "/" + path
        let home = joined == "~" || joined.hasPrefix("~/")
        guard joined.hasPrefix("/") || remote && home else { return nil }
        var parts: [String] = []
        for part in joined.split(separator: "/") {
            if part == "." { continue }
            if part == ".." {
                guard !parts.isEmpty, parts.last != "~" else { return nil }
                parts.removeLast()
            } else { parts.append(String(part)) }
        }
        return (home ? "" : "/") + parts.joined(separator: "/")
    }
}
