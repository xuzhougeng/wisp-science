import Foundation

/// User-authored data lives in Application Support, not the disposable read
/// cache. Corrupt or locked files throw; callers must never replace them with
/// an apparently empty notebook/checklist.
public enum OfflineResearchStorage {
    public static var directory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("WispResearch-v1", isDirectory: true)
    }
    public static func load<T: Decodable>(_ type: T.Type, from url: URL) throws -> T? {
        let data: Data
        do { data = try Data(contentsOf: url) }
        catch let error as CocoaError where error.code == .fileReadNoSuchFile { return nil }
        return try JSONDecoder().decode(type, from: data)
    }
    public static func save<T: Encodable>(_ value: T, to url: URL) throws {
        try write(JSONEncoder().encode(value), to: url)
    }
    public static func write(_ data: Data, to url: URL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        #if os(iOS)
        try data.write(to: url, options: [.atomic, .completeFileProtection])
        #else
        try data.write(to: url, options: .atomic)
        #endif
    }
}
