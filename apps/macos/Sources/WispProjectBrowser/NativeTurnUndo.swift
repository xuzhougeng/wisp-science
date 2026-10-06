import Foundation

/// Existing wisp-dto::TurnUndoPreview, whose lists default to empty.
public struct NativeTurnUndoPreview: Codable, Equatable, Sendable {
    public let restore_files: [String]
    public let remove_files: [String]
    public let remove_artifacts: [String]
    public let unsupported_files: [String]
    public let conflicts: [String]
    enum CodingKeys: String, CodingKey { case restore_files, remove_files, remove_artifacts, unsupported_files, conflicts }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        restore_files = try values.decodeIfPresent([String].self, forKey: .restore_files) ?? []
        remove_files = try values.decodeIfPresent([String].self, forKey: .remove_files) ?? []
        remove_artifacts = try values.decodeIfPresent([String].self, forKey: .remove_artifacts) ?? []
        unsupported_files = try values.decodeIfPresent([String].self, forKey: .unsupported_files) ?? []
        conflicts = try values.decodeIfPresent([String].self, forKey: .conflicts) ?? []
    }
}
