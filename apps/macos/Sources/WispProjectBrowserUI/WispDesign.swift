import AppKit
import SwiftUI
import WispProjectBrowser

/// Generated resources come from base.css, compose_icon(), and the WebView wordmarks.
enum WispDesign {
    // SwiftPM's generated lookup differs across Swift releases. Packaged apps
    // always put resources in Contents/Resources; `swift test/run` uses module.
    private static let resources: Bundle = {
        if let url = Bundle.main.url(forResource: "WispSciencePreview_WispProjectBrowserUI", withExtension: "bundle"),
           let bundle = Bundle(url: url) { return bundle }
        return Bundle.module
    }()
    static let english: [String: String] = {
        let url = resources.url(forResource: "native-english", withExtension: "json")!
        return (try? JSONDecoder().decode([String: String].self, from: Data(contentsOf: url))) ?? [:]
    }()
    static let settingsNavigation: [String: [String: String]] = {
        let url = resources.url(forResource: "settings-navigation", withExtension: "json")!
        return (try? JSONDecoder().decode([String: [String: String]].self, from: Data(contentsOf: url))) ?? [:]
    }()
    static let modelPresets: [[String: String]] = {
        let url = resources.url(forResource: "model-presets", withExtension: "json")!
        return (try? JSONDecoder().decode([[String: String]].self, from: Data(contentsOf: url))) ?? []
    }()
    static let palettes: [String: [String: String]] = {
        let url = resources.url(forResource: "palette", withExtension: "json")!
        return try! JSONDecoder().decode([String: [String: String]].self, from: Data(contentsOf: url))
    }()

    static func color(_ token: String, _ scheme: ColorScheme) -> Color {
        let theme = scheme == .dark ? "dark" : "light"
        let selected = UserDefaults.standard.string(forKey: "nativeSettings." + theme + "_palette") ?? (scheme == .dark ? "charcoal" : "paper")
        return paletteColor(token, scheme: scheme, palette: selected)
    }

    static func paletteColor(_ token: String, scheme: ColorScheme, palette: String) -> Color {
        let theme = scheme == .dark ? "dark" : "light"
        let value = (palettes[theme + "-" + palette] ?? palettes[theme])![token]!
        if value.hasPrefix("#") {
            let hex = String(value.dropFirst())
            let expanded = hex.count == 3 ? hex.map { "\($0)\($0)" }.joined() : hex
            let rgb = UInt32(expanded, radix: 16)!
            return Color(.sRGB, red: Double((rgb >> 16) & 255) / 255,
                         green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255, opacity: 1)
        }
        // The exported --border token uses rgba in both WebView themes.
        let components = value.dropFirst(5).dropLast().split(separator: ",").map {
            Double($0.trimmingCharacters(in: .whitespaces))!
        }
        return Color(.sRGB, red: components[0] / 255, green: components[1] / 255,
                     blue: components[2] / 255, opacity: components[3])
    }

    static func font(size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default) -> Font {
        let code = design == .monospaced
        let key = code ? "code" : "ui"
        let configured = UserDefaults.standard.double(forKey: "nativeSettings." + key + "_font_size")
        let scaled = size * (configured > 0 ? configured / (code ? 12 : 14) : 1)
        let family = UserDefaults.standard.string(forKey: "nativeSettings." + key + "_font_family") ?? ""
        return family.isEmpty ? .system(size: scaled, weight: weight, design: design) : .custom(family, size: scaled).weight(weight)
    }

    static func apply(_ prefs: WispProjectBrowser.SettingsValue, defaults: UserDefaults = .standard) {
        if case .bool(let value) = prefs["send_with_modifier"] {
            defaults.set(value, forKey: "nativeSettings.send_with_modifier")
        }
        guard prefs["theme"] != .null else { return }
        for key in ["light_palette", "dark_palette", "ui_font_family", "code_font_family"] { defaults.set(prefs[key].string, forKey: "nativeSettings." + key) }
        for key in ["ui_font_size", "code_font_size"] { defaults.set(prefs[key].integer, forKey: "nativeSettings." + key) }
        defaults.set(prefs["theme"].string, forKey: "projectBrowser.appearance")
    }

    static func image(_ name: String) -> NSImage {
        let image = NSImage(contentsOf: resources.url(forResource: name, withExtension: "svg")!)!
        if name.hasPrefix("icon-") { image.isTemplate = true }
        return image
    }
}

struct WispIcon: View {
    let name: String
    var size: CGFloat = 18

    var body: some View {
        Image(nsImage: WispDesign.image("icon-\(name)"))
            .renderingMode(.template).resizable().frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

struct WispButtonStyle: ButtonStyle {
    @Environment(\.colorScheme) private var scheme
    @Environment(\.isEnabled) private var enabled
    var primary = false
    var compact = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(WispDesign.font(size: 13, weight: .medium))
            .foregroundStyle(primary ? Color.white : WispDesign.color("text", scheme))
            .padding(.horizontal, compact ? 8 : 12).frame(height: compact ? 30 : 38)
            .background(compact && !primary ? Color.clear : WispDesign.color(primary ? "clay" : "bg-elev", scheme), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(compact ? Color.clear : WispDesign.color("border", scheme)))
            .opacity(!enabled ? 0.45 : (configuration.isPressed ? 0.7 : 1))
            .contentShape(RoundedRectangle(cornerRadius: 10))
    }
}

/// Preserve the WebView's action positions without assigning unrelated behavior
/// to controls whose native service is not connected yet.
struct WispUnavailableAction: View {
    let title: String
    var icon: String? = nil
    var iconOnly = false
    var compact = false
    var primary = false
    var expanded = false

    var body: some View {
        Button {} label: {
            HStack(spacing: 8) {
                if let icon { WispIcon(name: icon, size: 16) }
                if !iconOnly { Text(title) }
                if expanded { Spacer(minLength: 0) }
            }
        }
        .buttonStyle(WispButtonStyle(primary: primary, compact: expanded || compact)).disabled(true)
        .help("\(title) · 原生预览尚未接入")
        .accessibilityLabel("\(title)（尚未接入）")
    }
}

func localized(_ text: String) -> String {
    UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? (WispDesign.english[text] ?? text) : text
}
