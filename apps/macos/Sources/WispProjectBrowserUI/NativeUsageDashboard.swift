import SwiftUI
import WispProjectBrowser

enum NativeUsageDashboardData {
    static func total(_ rows: [SettingsValue], field: String) -> Int64 { rows.reduce(0) { $0 + max(0, $1[field].integer) } }
    static func compact(_ value: Int64) -> String { value < 1000 ? String(value) : String(format: "%.1fk", Double(value) / 1000) }
    static func activity(_ rows: [SettingsValue], mode: String) -> [(String, Int64, Bool)] {
        var cumulative: Int64 = 0
        let days = rows.suffix(371).map { row -> (String, Int64, Bool) in
            let future = row["future"].bool
            let value = future ? 0 : max(0, row["tokens"].integer)
            cumulative += value
            return (row["date"].string, mode == "cumulative" ? cumulative : value, future)
        }
        if mode != "weekly" { return days }
        return stride(from: 0, to: days.count, by: 7).map { start in
            let week = days[start..<min(start + 7, days.count)]
            return (days[start].0, week.filter { !$0.2 }.reduce(0) { $0 + $1.1 }, week.allSatisfy { $0.2 })
        }
    }
}

private struct NativeUsageArc: Shape {
    let start: Double
    let end: Double
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.addArc(center: CGPoint(x: rect.midX, y: rect.midY), radius: min(rect.width, rect.height) * 0.38,
                    startAngle: .degrees(start * 360 - 90), endAngle: .degrees(end * 360 - 90), clockwise: false)
        return path
    }
}

struct NativeUsageDashboard: View {
    let overview: SettingsValue
    let profiles: [SettingsValue]
    @State private var mode = "daily"
    private let colors: [Color] = [.teal, .blue, .orange, .purple, .pink, .green, .indigo, .gray]
    private var models: [(label: String, value: Int64)] { NativeUsageRows.models(overview["models"].array, profiles: profiles) }
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            Text(localized("按工作区汇总用量；子智能体轮次计入根会话。")).font(.caption).foregroundStyle(.secondary)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150))], spacing: 12) {
                ForEach([("input", "输入"), ("output", "输出"), ("reasoning", "思考"), ("cached", "缓存")], id: \.0) { key, label in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(localized(label)).font(.caption).foregroundStyle(.secondary)
                        Text(NativeUsageDashboardData.compact(NativeUsageDashboardData.total(overview["workspaces"].array, field: key))).font(.title2.monospacedDigit())
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(16).background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                }
            }
            NativeSettingsGroup(title: "Token 活动") {
                Picker(localized("统计方式"), selection: $mode) { Text(localized("每日")).tag("daily"); Text(localized("每周")).tag("weekly"); Text(localized("累计")).tag("cumulative") }.pickerStyle(.segmented).frame(maxWidth: 320)
                let activity = NativeUsageDashboardData.activity(overview["days"].array, mode: mode)
                let maximum = max(1, activity.filter { !$0.2 }.map { $0.1 }.max() ?? 0)
                ScrollView(.horizontal) {
                    LazyHGrid(rows: Array(repeating: GridItem(.fixed(12), spacing: 3), count: mode == "weekly" ? 1 : 7), spacing: 3) {
                        ForEach(Array(activity.enumerated()), id: \.offset) { _, day in
                            RoundedRectangle(cornerRadius: 2).fill(day.2 ? Color.clear : Color.teal.opacity(day.1 == 0 ? 0.08 : 0.2 + 0.8 * Double(day.1) / Double(maximum)))
                                .frame(width: 12, height: 12)
                                .help(day.2 ? day.0 : day.0 + " · " + NativeUsageDashboardData.compact(day.1))
                                .accessibilityLabel(day.2 ? day.0 : day.0 + " · \(day.1) tokens")
                        }
                    }.padding(.vertical, 8)
                }
                .frame(height: mode == "weekly" ? 32 : 122)
                if activity.isEmpty { Text(localized("暂无用量记录")).foregroundStyle(.secondary) }
            }
            NativeSettingsGroup(title: "模型占比") {
                let total = max(1, models.reduce(Int64(0)) { $0 + $1.value })
                HStack(spacing: 24) {
                    ZStack {
                        Circle().stroke(Color.secondary.opacity(0.12), lineWidth: 22).padding(19)
                        ForEach(Array(models.enumerated()), id: \.offset) { index, row in
                            let before = models.prefix(index).reduce(Int64(0)) { $0 + $1.value }
                            NativeUsageArc(start: Double(before) / Double(total), end: Double(before + row.value) / Double(total)).stroke(colors[index % colors.count], lineWidth: 22)
                        }
                    }.frame(width: 160, height: 160).accessibilityLabel(localized("模型用量占比"))
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(models.enumerated()), id: \.offset) { index, row in
                            HStack { Circle().fill(colors[index % colors.count]).frame(width: 8, height: 8); Text(row.label); Spacer(); Text(String(format: "%.1f%%", Double(row.value) / Double(total) * 100)).monospacedDigit() }
                        }
                        if models.isEmpty { Text(localized("暂无模型用量")).foregroundStyle(.secondary) }
                    }
                }
            }
            NativeSettingsGroup(title: "SKILL / MCP 调用排行") {
                let rows = overview["tools"].array.sorted { $0["calls"].integer > $1["calls"].integer }
                let maximum = max(1, rows.first?["calls"].integer ?? 0)
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack { Text(row["name"].string); Text(row["kind"].string).font(.caption).foregroundStyle(.secondary); Spacer(); Text("\(row["calls"].integer)").monospacedDigit() }
                        ProgressView(value: Double(max(0, row["calls"].integer)), total: Double(maximum)).tint(.teal)
                    }
                }
                if rows.isEmpty { Text(localized("暂无调用记录")).foregroundStyle(.secondary) }
            }
        }
    }
}
