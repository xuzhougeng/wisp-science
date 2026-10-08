import Foundation
import CryptoKit

/// Per-device display preferences, isolated from executable template definitions.
struct NativeWorkflowLayoutStore {
    struct Snapshot: Codable {
        var positions: [String: CGPoint]
        var zoom: CGFloat
        var camera: CGPoint
    }
    let scope: String
    var defaults: UserDefaults = .standard
    private func key(project: String?, template: String) -> String {
        let bytes = (try? JSONEncoder().encode([scope, project ?? "", template])) ?? Data()
        return "nativeWorkflow.layout.v1." + SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }
    func read(project: String?, template: String) -> Snapshot? {
        guard let data = defaults.data(forKey: key(project: project, template: template)), let value = try? JSONDecoder().decode(Snapshot.self, from: data), value.zoom.isFinite, (0.25...2).contains(value.zoom), Self.valid(value.camera), value.positions.values.allSatisfy(Self.valid) else { return nil }
        return value
    }
    func save(_ value: Snapshot, project: String?, template: String) {
        guard !template.isEmpty, value.zoom.isFinite, Self.valid(value.camera), value.positions.values.allSatisfy(Self.valid), let data = try? JSONEncoder().encode(value) else { return }
        defaults.set(data, forKey: key(project: project, template: template))
    }
    func remove(project: String?, template: String) { defaults.removeObject(forKey: key(project: project, template: template)) }
    private static func valid(_ point: CGPoint) -> Bool { point.x.isFinite && point.y.isFinite && abs(point.x) <= 1_000_000 && abs(point.y) <= 1_000_000 }
}
