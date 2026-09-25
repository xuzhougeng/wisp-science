import Foundation

public enum NativeProjectCommand {
    public static let create = "native_project_create"
    public static let exportProject = "native_project_export"
    public static let importArchive = "native_project_import"
    public static let importDirectory = "native_project_import_directory"
    public static let recoveryPreview = "native_project_recovery_preview"
    public static let recoverWorkspace = "native_project_recover_workspace"

    public static func summary(from value: SettingsValue) throws -> ProjectSummary {
        let summary = try JSONDecoder().decode(ProjectSummary.self, from: JSONEncoder().encode(value))
        if summary.id.isEmpty { throw ProjectBrowserError.invalidResponse }
        return summary
    }
}
