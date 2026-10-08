import Foundation
import Network
import WispProjectBrowser

/// A disposable loopback origin lets WebKit load the shared ES modules, workers
/// and WASM. It serves bundled renderer assets only; document bytes never enter
/// this server. The opaque route is retired when its preview closes.
final class NativePreviewServer {
    let token = UUID().uuidString.lowercased()
    private let queue = DispatchQueue(label: "science.wisp.preview-assets")
    private let listener: NWListener
    private var connections: [ObjectIdentifier: NWConnection] = [:]
    private var completed = false
    private let assets: NativePreviewAssets
    init(assets: NativePreviewAssets = .bundled) throws {
        self.assets = assets
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        listener = try NWListener(using: parameters)
    }
    func start(_ ready: @escaping (Result<URL, Error>) -> Void) {
        listener.stateUpdateHandler = { [weak self] state in
            guard let self, !self.completed else { return }
            switch state {
            case .ready:
                guard let port = self.listener.port else { return }
                self.completed = true
                ready(.success(URL(string: "http://127.0.0.1:\(port.rawValue)/\(self.token)/index.html")!))
            case .failed(let error): self.completed = true; ready(.failure(error))
            default: break
            }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self, self.connections.count < 32 else { connection.cancel(); return }
            self.connections[ObjectIdentifier(connection)] = connection
            connection.start(queue: self.queue)
            self.receive(connection, header: Data())
            self.queue.asyncAfter(deadline: .now() + 10) { [weak self, weak connection] in
                if let connection { self?.finish(connection) }
            }
        }
        listener.start(queue: queue)
    }
    func stop() {
        queue.async { [self] in
            listener.stateUpdateHandler = nil; listener.newConnectionHandler = nil; listener.cancel()
            connections.values.forEach { $0.cancel() }; connections.removeAll()
        }
    }
    private func receive(_ connection: NWConnection, header: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: max(1, 16384 - header.count)) { [weak self] bytes, _, complete, error in
            guard let self else { connection.cancel(); return }
            var data = header; if let bytes { data.append(bytes) }
            if data.range(of: Data("\r\n\r\n".utf8)) != nil {
                let first = String(decoding: data, as: UTF8.self).components(separatedBy: "\r\n").first ?? ""
                let parts = first.split(separator: " ")
                guard parts.count == 3, parts[0] == "GET", let asset = self.assets.read(String(parts[1]), token: self.token) else { self.respond(connection, status: "404 Not Found", mime: "text/plain", body: Data()); return }
                var body = asset.data
                if ["text/javascript", "text/html", "text/css"].contains(asset.mime), let text = String(data: body, encoding: .utf8) {
                    // The authored shared renderer uses root-relative vendor
                    // imports. Keep every fetch within this preview's route.
                    body = Data(text.replacingOccurrences(of: "/vendor-runtime/", with: "/\(self.token)/vendor-runtime/").utf8)
                }
                self.respond(connection, status: "200 OK", mime: asset.mime, body: body, moleculeWorker: parts[1] == "/\(self.token)/vendor-runtime/rdkit-worker.mjs")
            } else if complete || error != nil || data.count >= 16384 { self.finish(connection) }
            else { self.receive(connection, header: data) }
        }
    }
    private func respond(_ connection: NWConnection, status: String, mime: String, body: Data, moleculeWorker: Bool = false) {
        // RDKit's Embind registration requires dynamic functions only in its
        // disposable worker. The containing document never receives this policy.
        let csp = moleculeWorker
            ? "default-src 'none'; script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; connect-src 'self'; object-src 'none'"
            : "default-src 'none'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; connect-src 'self'; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; img-src data: blob:; font-src 'self' data: blob:; frame-src 'self'; object-src 'none'; base-uri 'self'"
        var response = Data("HTTP/1.1 \(status)\r\nContent-Type: \(mime)\r\nContent-Length: \(body.count)\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: \(csp)\r\nConnection: close\r\n\r\n".utf8)
        response.append(body)
        connection.send(content: response, completion: .contentProcessed { [weak self] _ in self?.finish(connection) })
    }
    private func finish(_ connection: NWConnection) { connections.removeValue(forKey: ObjectIdentifier(connection)); connection.cancel() }
    deinit { listener.cancel() }
}

struct NativePreviewAssets {
    let presentation: URL
    let vendor: URL
    static func developmentAssetsAllowed(in mainBundle: URL) -> Bool { mainBundle.pathExtension.lowercased() != "app" }
    static var bundled: Self {
        let presentation = Bundle.module.resourceURL!
        var vendor = presentation.appendingPathComponent("vendor-runtime", isDirectory: true)
        #if DEBUG
        if !FileManager.default.fileExists(atPath: vendor.path), developmentAssetsAllowed(in: Bundle.main.bundleURL) {
            var root = URL(fileURLWithPath: #filePath)
            for _ in 0..<5 { root.deleteLastPathComponent() }
            vendor = root.appendingPathComponent("ui/vendor-src", isDirectory: true)
        }
        #endif
        return Self(presentation: presentation, vendor: vendor)
    }
    func read(_ target: String, token: String) -> (data: Data, mime: String)? {
        guard let path = target.split(separator: "?", maxSplits: 1).first.map(String.init)?.removingPercentEncoding,
              path.hasPrefix("/\(token)/"), !path.contains("\\"), !path.contains("\0") else { return nil }
        let relative = String(path.dropFirst(token.count + 2))
        let components = relative.split(separator: "/", omittingEmptySubsequences: false)
        guard !components.isEmpty, components.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." && $0.utf8.allSatisfy { (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [45, 46, 95].contains($0) } }) else { return nil }
        let map = ["index.html": "native-rich-preview.html", "preview.mjs": "native-rich-preview.mjs", "preview.css": "native-rich-preview.css", "office.css": "native-office-preview.css", "selection.mjs": "native-preview-selection.mjs"]
        let root: URL, resource: String
        if let native = map[relative] { root = presentation; resource = native }
        else { root = vendor; resource = relative.hasPrefix("vendor-runtime/") ? String(relative.dropFirst(15)) : relative }
        let file = root.appendingPathComponent(resource).resolvingSymlinksInPath()
        guard file.path.hasPrefix(root.resolvingSymlinksInPath().path + "/"),
              let size = try? file.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey]), size.isRegularFile == true,
              let count = size.fileSize, count <= 32 * 1024 * 1024, let data = try? Data(contentsOf: file) else { return nil }
        let mime: String
        switch file.pathExtension.lowercased() {
        case "html": mime = "text/html"
        case "js", "mjs": mime = "text/javascript"
        case "css": mime = "text/css"
        case "wasm": mime = "application/wasm"
        case "woff": mime = "font/woff"
        case "woff2": mime = "font/woff2"
        case "ttf": mime = "font/ttf"
        case "json": mime = "application/json"
        default: mime = "text/plain"
        }
        return (data, mime)
    }
}
