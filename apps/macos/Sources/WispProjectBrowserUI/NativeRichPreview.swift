import AppKit
import SwiftUI
import WebKit
import WispProjectBrowser

@MainActor
final class NativeRichPreviewState: ObservableObject {
    @Published var error: String?
    @Published var ready = false
    @Published var hasSelection = false
    var requestQuote: (() -> Void)?
}

struct NativeRichPreview: View {
    let content: NativePanelFileContent
    let kind: NativeDocumentKind
    var quote: ((NativeDocumentSelection) -> Bool)?
    var loadImage: NativeFileImageLoader?
    @StateObject private var state = NativeRichPreviewState()
    @State private var prepared: String?
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let error = state.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            if !content.truncated {
                NativeRichPreviewSurface(content: content, kind: kind, html: prepared, scheme: scheme, state: state, quote: quote)
                if let _ = quote { Button(localized("引用选中文字")) { state.requestQuote?() }.disabled(!state.ready || !state.hasSelection) }
            } else { Text(localized("文件内容不完整，请下载完整文件后预览。")).foregroundStyle(.orange) }
        }.task(id: content.path) {
            if kind == .html, let text = content.text {
                prepared = await NativeHTMLPreviewImages.prepare(text, loader: loadImage)
            }
        }
    }
}

enum NativeHTMLPreviewImages {
    static func prepare(_ text: String, loader: NativeFileImageLoader?) async -> String {
        guard text.utf8.count <= 1024 * 1024, let loader,
              let regex = try? NSRegularExpression(pattern: "(<img\\b[^>]*?\\bsrc\\s*=\\s*)([\"'])([^\"']+)\\2", options: .caseInsensitive) else { return text }
        let source = text as NSString
        let matches = Array(regex.matches(in: text, range: NSRange(location: 0, length: source.length)).prefix(128))
        var images: [String: String] = [:]
        for match in matches {
            if Task.isCancelled { return text }
            let reference = source.substring(with: match.range(at: 3))
            guard images[reference] == nil, !reference.hasPrefix("data:"), !reference.hasPrefix("blob:") else { continue }
            // The scoped loader resolves relative paths and approves bytes.
            if let image = try? await loader(reference), image.mime == "image/png", !image.truncated,
               let b64 = image.base64, b64.utf8.count <= 12 * 1024 * 1024, Data(base64Encoded: b64) != nil {
                let value = "data:image/png;base64," + b64
                if images.values.reduce(0, { $0 + $1.utf8.count }) + value.utf8.count > 24 * 1024 * 1024 { break }
                images[reference] = value
            }
        }
        var result = text
        for match in matches.reversed() {
            let reference = source.substring(with: match.range(at: 3))
            if let image = images[reference], let range = Range(match.range(at: 3), in: result) { result.replaceSubrange(range, with: image) }
        }
        return result
    }
}

enum NativePreviewNavigation {
    static func assetPrefix(_ document: URL) -> String {
        let prefix = document.deletingLastPathComponent().absoluteString
        return prefix.hasSuffix("/") ? prefix : prefix + "/"
    }
    static func sameDocument(_ target: URL?, _ document: URL?) -> Bool {
        guard let target, let document,
              var parts = URLComponents(url: target, resolvingAgainstBaseURL: false) else { return false }
        parts.fragment = nil
        return parts.url == document
    }
    static func allowed(_ target: URL?, document: URL?, mainFrame: Bool) -> Bool {
        if mainFrame { return sameDocument(target, document) }
        if ["about:srcdoc", "about:blank"].contains(target?.absoluteString ?? "") { return true }
        guard let target, let document else { return false }
        return target.absoluteString.hasPrefix(assetPrefix(document))
    }
}

struct NativeRichPreviewSurface: NSViewRepresentable {
    let content: NativePanelFileContent
    let kind: NativeDocumentKind
    let html: String?
    let scheme: ColorScheme
    @ObservedObject var state: NativeRichPreviewState
    let quote: ((NativeDocumentSelection) -> Bool)?
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        configuration.userContentController.add(context.coordinator, name: "preview")
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.allowsLinkPreview = false; view.navigationDelegate = context.coordinator; view.uiDelegate = context.coordinator
        context.coordinator.view = view; context.coordinator.start()
        return view
    }
    func updateNSView(_ view: WKWebView, context: Context) {
        let old = context.coordinator.parent
        context.coordinator.parent = self
        if old.html != html || old.scheme != scheme { context.coordinator.send() }
    }
    static func dismantleNSView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.disposed = true; coordinator.parent.state.requestQuote = nil
        if let token = coordinator.server?.token { WKContentRuleListStore.default().removeContentRuleList(forIdentifier: "wisp-preview-" + token) { _ in } }
        coordinator.server?.stop(); coordinator.server = nil
        view.stopLoading(); view.configuration.userContentController.removeScriptMessageHandler(forName: "preview")
        view.navigationDelegate = nil; view.uiDelegate = nil; view.loadHTMLString("", baseURL: nil)
    }
    @MainActor final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate, WKUIDelegate {
        var parent: NativeRichPreviewSurface
        weak var view: WKWebView?
        var server: NativePreviewServer?
        var url: URL?
        var disposed = false
        var requestingQuote = false
        init(_ parent: NativeRichPreviewSurface) { self.parent = parent }
        func start() {
            do {
                let server = try NativePreviewServer(); self.server = server
                server.start { [weak self] result in
                    DispatchQueue.main.async {
                        guard let self, !self.disposed, let view = self.view else { return }
                        switch result {
                        case .failure(let error): self.parent.state.error = error.localizedDescription
                        case .success(let url):
                            self.url = url
                            let escaped = NSRegularExpression.escapedPattern(for: NativePreviewNavigation.assetPrefix(url))
                            let rules: [[String: Any]] = [["trigger": ["url-filter": ".*"], "action": ["type": "block"]],
                                ["trigger": ["url-filter": "^" + escaped], "action": ["type": "ignore-previous-rules"]],
                                ["trigger": ["url-filter": "^data:"], "action": ["type": "ignore-previous-rules"]],
                                ["trigger": ["url-filter": "^blob:"], "action": ["type": "ignore-previous-rules"]]]
                            let encoded = String(data: try! JSONSerialization.data(withJSONObject: rules), encoding: .utf8)!
                            WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "wisp-preview-" + server.token, encodedContentRuleList: encoded) { [weak self, weak view] list, error in
                                guard let self, !self.disposed, let view else { WKContentRuleListStore.default().removeContentRuleList(forIdentifier: "wisp-preview-" + server.token) { _ in }; return }
                                guard let list else { self.parent.state.error = error?.localizedDescription ?? localized("离线预览资源未能加载。"); return }
                                view.configuration.userContentController.add(list); view.load(URLRequest(url: url))
                            }
                        }
                    }
                }
            } catch { parent.state.error = error.localizedDescription }
            parent.state.requestQuote = { [weak self] in
                guard let self, !self.disposed, self.parent.state.hasSelection else { return }
                self.requestingQuote = true
                self.view?.evaluateJavaScript("window.nativePreviewQuote(); true") { [weak self] _, error in if error != nil { self?.requestingQuote = false } }
            }
        }
        func send() {
            guard !disposed, parent.state.ready, let view else { return }
            let content = parent.content
            guard !content.truncated, !parent.kind.textual || (content.text?.utf8.count ?? Int.max) <= 1024 * 1024,
                  parent.kind.textual || (content.base64?.utf8.count ?? Int.max) <= 45 * 1024 * 1024 else { parent.state.error = localized("文件超过离线预览限制。"); return }
            if parent.kind == .html && parent.html == nil { return }
            func color(_ token: String) -> String {
                let color = NSColor(WispDesign.color(token, parent.scheme)).usingColorSpace(.sRGB) ?? .textColor
                return String(format: "#%02x%02x%02x", Int(color.redComponent * 255), Int(color.greenComponent * 255), Int(color.blueComponent * 255))
            }
            let payload: [String: Any] = ["kind": parent.kind.rawValue, "text": parent.kind == .html ? parent.html ?? "" : content.text ?? "", "base64": content.base64 ?? "", "format": parent.kind.format(path: content.path), "truncated": content.truncated,
                "canQuote": parent.quote != nil, "background": color("bg-app"), "foreground": color("text"), "fontSize": 14,
                "title": (content.path as NSString).lastPathComponent, "loading": localized("正在加载文档…"), "error": localized("无法预览此文档。"), "oversized": localized("文件超过离线预览限制。"), "quoteLabel": localized("引用选中文字"), "formulaLabel": localized("公式"), "truncatedLabel": localized("工作簿较大，仅显示限定范围内的数据。")]
            guard let data = try? JSONSerialization.data(withJSONObject: payload), let json = String(data: data, encoding: .utf8) else { return }
            requestingQuote = false; parent.state.hasSelection = false
            view.evaluateJavaScript("window.nativePreviewReceive({data:\(json)}); true") { [weak self] _, error in if let error, self?.disposed == false { self?.parent.state.error = error.localizedDescription } }
        }
        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
            guard !disposed, message.frameInfo.isMainFrame, NativePreviewNavigation.sameDocument(message.frameInfo.request.url, url),
                  let object = message.body as? [String: Any], let data = try? JSONSerialization.data(withJSONObject: object),
                  let value = try? JSONDecoder().decode(SettingsValue.self, from: data) else { return }
            switch value["type"].string {
            case "ready": parent.state.ready = true; send()
            case "selection": parent.state.hasSelection = value["active"].bool
            case "quote":
                guard requestingQuote else { return }; requestingQuote = false
                let selection = NativeDocumentSelection.parse(value, expected: parent.kind)
                let accepted = selection.map { parent.quote?($0) == true } ?? false
                if !disposed { view?.evaluateJavaScript("window.nativePreviewReceive({data:{type:'quote-result',accepted:\(accepted)}}); true") }
            default: break
            }
        }
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            let allowed = navigationAction.targetFrame != nil && NativePreviewNavigation.allowed(navigationAction.request.url, document: url, mainFrame: navigationAction.targetFrame?.isMainFrame == true)
            decisionHandler(allowed ? .allow : .cancel)
        }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { if !disposed { parent.state.error = error.localizedDescription } }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { if !disposed { parent.state.error = error.localizedDescription } }
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? { nil }
        func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void) { completionHandler(nil) }
        deinit { server?.stop() }
    }
}
