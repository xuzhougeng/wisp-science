import SwiftUI
import WispProjectBrowser

/// Uses the same palette tokens and stored preferences as the WebView client.
/// Draft changes preview locally; Save commits the complete preference document.
struct NativeAppearanceSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @Environment(\.colorScheme) private var systemScheme
    @State private var cssExpanded = false
    private var prefs: SettingsValue { model.values["get_appearance_prefs"] ?? .null }
    private var previewScheme: ColorScheme { prefs["theme"].string == "dark" ? .dark : prefs["theme"].string == "light" ? .light : systemScheme }
    private func field(_ key: String) -> Binding<SettingsValue> { model.binding("get_appearance_prefs", key) }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            NativeSettingsColumns {
                VStack(spacing: 20) {
                    NativeSettingsGroup(title: "主题") {
                        HStack(spacing: 10) {
                            themeTile("system", "跟随系统", dark: systemScheme == .dark)
                            themeTile("light", "浅色", dark: false)
                            themeTile("dark", "深色", dark: true)
                        }
                    }
                    NativeSettingsGroup(title: "配色") {
                        NativeSettingsField(field: .init(key: "light_palette", label: "浅色配色", kind: .choice([("paper", "Paper"), ("codex", "Codex"), ("github", "GitHub"), ("catppuccin", "Catppuccin"), ("everforest", "Everforest")])), value: field("light_palette"))
                        NativeSettingsField(field: .init(key: "dark_palette", label: "深色配色", kind: .choice([("charcoal", "Charcoal"), ("codex", "Codex"), ("github", "GitHub"), ("catppuccin", "Catppuccin"), ("gruvbox", "Gruvbox")])), value: field("dark_palette"))
                        HStack(spacing: 16) {
                            swatch("强调色", token: "clay")
                            swatch("背景", token: "bg-app")
                            swatch("文字", token: "text")
                        }.padding(.top, 6)
                    }
                    NativeSettingsGroup(title: "字体") {
                        fontSlider("界面字号", key: "ui_font_size", range: 12...20, fallback: 14)
                        fontSlider("代码字号", key: "code_font_size", range: 10...20, fallback: 12)
                        NativeSettingsField(field: .init(key: "ui_font_family", label: "界面字体", hint: "留空使用系统字体。"), value: field("ui_font_family"))
                        NativeSettingsField(field: .init(key: "code_font_family", label: "代码字体", hint: "留空使用系统等宽字体。"), value: field("code_font_family"))
                    }
                }
            } second: {
                VStack(alignment: .leading, spacing: 16) {
                    Text(localized("预览")).font(WispDesign.font(size: 14, weight: .semibold))
                    preview
                    Text(localized("预览展示尚未保存的主题、配色与字体。保存后应用到所有项目；取消恢复已保存的偏好。")).font(WispDesign.font(size: 12)).foregroundStyle(.secondary)
                }
            }
            NativeSettingsGroup(title: "高级") {
                DisclosureGroup(localized("自定义 CSS"), isExpanded: $cssExpanded) {
                    NativeSettingsField(field: .init(key: "custom_css", label: "WebView 自定义样式", kind: .multiline, hint: "此样式表用于保留的 WebView 客户端。"), value: field("custom_css")).padding(.top, 14)
                }
            }
            HStack {
                Spacer()
                Button(localized("取消")) { model.discardDrafts() }
                Button(localized("保存")) { Task {
                    if let saved = await model.run("set_appearance_prefs", ["prefs": prefs]) { WispDesign.apply(saved) }
                } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).disabled(model.loading || model.busy)
            }
        }.disabled(model.loading || model.values["get_appearance_prefs"] == nil)
    }

    private func themeTile(_ value: String, _ title: String, dark: Bool) -> some View {
        Button { field("theme").wrappedValue = .string(value) } label: {
            VStack(spacing: 9) {
                HStack(spacing: 0) {
                    Rectangle().fill(dark ? Color(white: 0.15) : Color(white: 0.91)).frame(width: 24)
                    VStack(alignment: .leading, spacing: 6) {
                        Capsule().fill(dark ? Color(white: 0.5) : Color(white: 0.75)).frame(width: 32, height: 4)
                        Capsule().fill(dark ? Color(white: 0.3) : Color(white: 0.9)).frame(height: 4)
                        Capsule().fill(dark ? Color(white: 0.3) : Color(white: 0.9)).frame(height: 4)
                    }.padding(10).frame(maxWidth: .infinity, maxHeight: .infinity).background(dark ? Color(white: 0.10) : .white)
                }.frame(height: 65).clipShape(RoundedRectangle(cornerRadius: 7))
                Text(localized(title)).font(WispDesign.font(size: 12, weight: prefs["theme"].string == value ? .semibold : .regular))
            }.padding(8).frame(maxWidth: .infinity)
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(prefs["theme"].string == value ? WispDesign.color("clay", systemScheme) : WispDesign.color("border", systemScheme), lineWidth: prefs["theme"].string == value ? 2 : 1))
        }.buttonStyle(.plain).accessibilityAddTraits(prefs["theme"].string == value ? .isSelected : [])
    }

    private func swatch(_ title: String, token: String) -> some View {
        HStack(spacing: 6) { RoundedRectangle(cornerRadius: 4).fill(previewColor(token)).frame(width: 18, height: 18).overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.gray.opacity(0.2))); Text(localized(title)).font(WispDesign.font(size: 11)) }
    }
    private func fontSlider(_ title: String, key: String, range: ClosedRange<Double>, fallback: Double) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack { Text(localized(title)); Spacer(); Text("\(Int(prefs[key] == .null ? fallback : Double(prefs[key].integer))) px").foregroundStyle(.secondary).monospacedDigit() }
            Slider(value: Binding(get: { prefs[key] == .null ? fallback : Double(prefs[key].integer) }, set: { field(key).wrappedValue = .integer(Int64($0)) }), in: range, step: 1).accessibilityLabel(localized(title))
        }
    }
    private func previewColor(_ token: String) -> Color {
        WispDesign.paletteColor(token, scheme: previewScheme, palette: prefs[previewScheme == .dark ? "dark_palette" : "light_palette"].string)
    }
    private func previewFont(code: Bool) -> Font {
        let prefix = code ? "code" : "ui"
        let fallback: Int64 = code ? 12 : 14
        let size = CGFloat(prefs[prefix + "_font_size"] == .null ? fallback : prefs[prefix + "_font_size"].integer)
        let family = prefs[prefix + "_font_family"].string
        return family.isEmpty ? .system(size: size, design: code ? .monospaced : .default) : .custom(family, size: size)
    }
    private var preview: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack { Text("Wisp Science").fontWeight(.semibold); Spacer(); Text(localized("新对话")).foregroundStyle(previewColor("text-muted")) }
            Divider()
            HStack { Spacer(minLength: 30); Text(localized("帮我查看这个项目的数据，整理分析思路。")).padding(14).background(previewColor("bg-sunken"), in: RoundedRectangle(cornerRadius: 12)) }
            VStack(alignment: .leading, spacing: 12) {
                Text(localized("我会先检查数据和项目记录，再列出下一步分析。")).fixedSize(horizontal: false, vertical: true)
                Text(localized("分析计划")).fontWeight(.semibold)
                Text(localized("1. 查看样本和文件\n2. 确认分析目标\n3. 汇总结果与图表")).lineSpacing(7).fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: 9) {
                Text("Python").font(.system(size: 11)).foregroundStyle(previewColor("text-muted"))
                Text("import pandas as pd\ndata = pd.read_csv(\"samples.csv\")\ndata.head()").font(previewFont(code: true)).fixedSize(horizontal: false, vertical: true)
            }.padding(14).frame(maxWidth: .infinity, alignment: .leading).background(previewColor("bg-sunken"), in: RoundedRectangle(cornerRadius: 10))
            Text(localized("输入消息…")).foregroundStyle(previewColor("text-muted")).padding(14).frame(maxWidth: .infinity, alignment: .leading).overlay(RoundedRectangle(cornerRadius: 12).stroke(previewColor("border")))
        }.font(previewFont(code: false))
            .foregroundStyle(previewColor("text")).padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(previewColor("bg-elev"), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).stroke(previewColor("border")))
    }
}
