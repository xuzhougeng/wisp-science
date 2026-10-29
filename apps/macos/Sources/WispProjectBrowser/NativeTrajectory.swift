import Foundation

/// Mirrors the existing wisp-dto TrajectorySnapshotDto, shared with WebView.
public struct NativeTrajectory: Codable, Sendable {
    public let frame_id: String
    public let model: String?
    public let turns: [NativeTrajectoryTurn]
    public let stats: NativeTrajectoryStats
    public static func decode(_ value: SettingsValue, sessionID: String) throws -> Self {
        let result = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard result.frame_id == sessionID else { throw ProjectBrowserError.invalidResponse }
        return result
    }
}
public struct NativeTrajectoryTurn: Codable, Identifiable, Sendable {
    public var id: Int64 { index }
    public let index: Int64
    public let started_at: Int64?
    public let cells: [NativeTrajectoryCell]
}
public struct NativeTrajectoryCell: Codable, Sendable {
    public let kind: String
    public let summary: String
    public let detail_input: String?
    public let detail_output: String?
    public let ok: Bool?
    public let is_error: Bool
    public let ts: Int64?
    public let duration_ms: Int64?
    public let usage: NativeTrajectoryUsage?
    public func matches(_ query: String) -> Bool {
        query.isEmpty || [summary, detail_input ?? "", detail_output ?? ""].contains { $0.localizedCaseInsensitiveContains(query) }
    }
}
public struct NativeTrajectoryUsage: Codable, Sendable {
    public let round: Int64
    public let model: String?
    public let input_tokens: Int64
    public let output_tokens: Int64
    public let reasoning_tokens: Int64
    public let cached_input_tokens: Int64
}
public struct NativeTrajectoryStats: Codable, Sendable {
    public let turns: Int64
    public let steps: Int64
    public let llm_ms: Int64
    public let tool_ms: Int64
    public let input_tokens: Int64
    public let output_tokens: Int64
    public let cached_input_tokens: Int64
    public let cache_hit_pct: Double?
    public let tokens_per_sec: Double?
}

extension NativeTrajectoryCell {
    public func status(running: Bool) -> String {
        if is_error || ok == false { return "error" }
        if kind == "tool" && ok == nil { return running ? "running" : "pending" }
        if running && kind == "assistant" && ok == nil && duration_ms == nil { return "running" }
        return "completed"
    }
    public var rawJSON: String { Self.json(self) }
    /// The host writes usage summaries in English ("round 1 · 12845 in / 117 out");
    /// show the WebView's `usage_line` instead.
    public var displaySummary: String { kind == "usage" && usage != nil ? preview : summary }
    /// `trajectory.source.*` labels; the badge itself stays USER/TOOL as in the WebView.
    public var sourceLabel: String { ["user": "用户", "assistant": "助手", "tool": "工具", "usage": "用量"][kind] ?? "未知" }
    public var preview: String {
        if kind == "usage", let usage {
            var line = "第 \(usage.round) 轮 · 输入 \(Self.tokens(usage.input_tokens)) · 输出 \(Self.tokens(usage.output_tokens))"
            if usage.input_tokens > 0 && usage.cached_input_tokens > 0 {
                line += " · 缓存 \(Int((Double(usage.cached_input_tokens) * 100 / Double(usage.input_tokens)).rounded()))%"
            }
            return line
        }
        if let output = detail_output, !output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return output }
        return detail_input ?? summary
    }
    public var source: String {
        if kind == "tool" { return detail_input ?? summary }
        if kind == "usage", let usage { return Self.json(usage) }
        if let output = detail_output, !output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return output }
        return summary
    }
    /// `fmt_tokens` in ui/src/chat_render.rs.
    static func tokens(_ n: Int64) -> String { n < 1000 ? "\(max(0, n))" : String(format: "%.1fk", Double(n) / 1000) }
    private static func json<T: Encodable>(_ value: T) -> String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return (try? String(data: encoder.encode(value), encoding: .utf8)) ?? "—"
    }
}
