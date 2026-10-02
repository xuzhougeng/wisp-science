import AppKit
import JavaScriptCore
import SwiftUI

/// The pinned WebView grammar runs without DOM or network access. Only token
/// ranges cross back into AppKit; the original code remains the copy source.
enum NativeCodeHighlight {
    private static let context = makeContext(resources: WispDesign.resources)

    static func makeContext(resources: Bundle) -> JSContext? {
        guard let context = JSContext(),
              let url = resources.url(forResource: "highlight.min", withExtension: "js"),
              let script = try? String(contentsOf: url) else { return nil }
        context.evaluateScript(script)
        context.evaluateScript("""
        function wispTokens(source, language) {
            if (!hljs.getLanguage(language)) return [];
            const root = hljs.highlight(source, {language, ignoreIllegals: true})._emitter.rootNode;
            let offset = 0, tokens = [];
            function visit(node, scope) {
                if (typeof node === 'string') {
                    if (scope) tokens.push({start: offset, length: node.length, scope});
                    offset += node.length;
                } else for (const child of node.children) visit(child, node.scope || scope);
            }
            visit(root, ''); return tokens;
        }
        """)
        return context
    }

    static func apply(to value: NSMutableAttributedString, language: String?, scheme: ColorScheme) {
        guard value.length <= 200_000, let language = language?.split(separator: " ").first,
              let tokens = context?.objectForKeyedSubscript("wispTokens")?.call(withArguments: [value.string, String(language).lowercased()])?.toArray() as? [[String: Any]] else { return }
        for token in tokens {
            guard let start = token["start"] as? Int, let length = token["length"] as? Int,
                  start >= 0, length > 0, start + length <= value.length,
                  let scope = token["scope"] as? String else { continue }
            let category = scope.components(separatedBy: ".").first ?? scope
            let color: NSColor
            switch category {
            case "comment", "quote": color = scheme == .dark ? NSColor(red: 0.55, green: 0.61, blue: 0.68, alpha: 1) : NSColor(red: 0.36, green: 0.41, blue: 0.46, alpha: 1)
            case "keyword", "selector-tag", "literal": color = scheme == .dark ? NSColor(red: 1, green: 0.48, blue: 0.45, alpha: 1) : NSColor(red: 0.81, green: 0.13, blue: 0.18, alpha: 1)
            case "string", "regexp", "addition": color = scheme == .dark ? NSColor(red: 0.65, green: 0.82, blue: 1, alpha: 1) : NSColor(red: 0.04, green: 0.19, blue: 0.41, alpha: 1)
            case "number", "built_in", "attr", "variable": color = scheme == .dark ? NSColor(red: 0.47, green: 0.75, blue: 1, alpha: 1) : NSColor(red: 0.02, green: 0.31, blue: 0.68, alpha: 1)
            case "title", "type": color = scheme == .dark ? NSColor(red: 0.82, green: 0.66, blue: 1, alpha: 1) : NSColor(red: 0.40, green: 0.22, blue: 0.73, alpha: 1)
            default: continue
            }
            value.addAttribute(.foregroundColor, value: color, range: NSRange(location: start, length: length))
        }
    }
}
