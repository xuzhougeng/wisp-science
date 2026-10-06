import Foundation

/// Decodes the shared wisp-dto native_search contract.
public struct NativeSearchItem: Codable, Equatable, Identifiable, Sendable {
    public var id: String { kind + ":" + objectID }
    public let kind: String
    public let objectID: String
    public let project_id: String
    public let project_name: String
    public let title: String
    public let detail: String
    public let session_id: String?

    enum CodingKeys: String, CodingKey {
        case kind, objectID = "id", project_id, project_name, title, detail, session_id
    }
    public var valid: Bool {
        guard !objectID.isEmpty, !project_id.isEmpty else { return false }
        switch kind {
        case "project": return objectID == project_id && session_id == nil
        case "session": return objectID == session_id
        case "artifact": return session_id?.isEmpty == false
        default: return false
        }
    }
}

public struct NativeSearchResponse: Codable, Sendable {
    public static let schemaID = "wisp.native-search.v1"
    public let schema: String
    public let query: String
    public let preferred_project_id: String?
    public let items: [NativeSearchItem]

    public static func decode(_ value: SettingsValue, query: String, projectID: String?) throws -> Self {
        let response = try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value))
        guard response.schema == schemaID, response.query == query,
              response.preferred_project_id == projectID, response.items.count <= 44,
              response.items.allSatisfy(\.valid), Set(response.items.map(\.id)).count == response.items.count else {
            throw ProjectBrowserError.invalidResponse
        }
        return response
    }
}
