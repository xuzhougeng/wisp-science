import Foundation

public enum NativeProjectCommand {
    public static let create = "native_project_create"

    public static func summary(from value: SettingsValue) throws -> ProjectSummary {
        let summary = try JSONDecoder().decode(ProjectSummary.self, from: JSONEncoder().encode(value))
        if summary.id.isEmpty { throw ProjectBrowserError.invalidResponse }
        return summary
    }
}
