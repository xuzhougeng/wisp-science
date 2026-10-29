import AppKit
import SwiftUI
import WispProjectBrowser

/// Services, think modes and both locales' strings for Settings → 模型 → 决策模型,
/// exported from wisp-dto and the WebView i18n by `scripts/sync_native_design.py`.
struct NativeDecisionCatalog: Decodable {
    struct Preset: Decodable, Equatable {
        let id: String
        let base_url: String
        let model: String
        let timeout_secs: Int64
        let key_required: Bool
        let key_url: String
    }
    let presets: [Preset]
    let think_modes: [String]
    let strings: [String: [String: String]]

    static let shared: NativeDecisionCatalog = {
        let url = WispDesign.resources.url(forResource: "decision-model", withExtension: "json")!
        return try! JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    }()
    func text(_ key: String) -> String {
        strings[key]?[UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? "en" : "zh"] ?? key
    }
    func preset(_ id: String) -> Preset? { presets.first { $0.id == id } }
}

/// `DecisionModelSettings` (wisp-dto); the API key never crosses this boundary.
struct NativeDecisionSettings: Codable, Equatable {
    var enabled = false
    var preset = ""
    var base_url = ""
    var model = ""
    var think_mode = ""
    var timeout_secs: Int64 = 0
    var stored_keys: [String] = []

    func hasKey(_ preset: String) -> Bool { stored_keys.contains(preset) }
    /// `usable` in ui/src/decision_model_settings.rs.
    func usable(in catalog: NativeDecisionCatalog) -> Bool {
        guard let preset = catalog.preset(preset) else { return false }
        return !preset.key_required || hasKey(preset.id)
    }
    /// `with_preset`: the preset's defaults, keeping the switch and stored keys.
    func switched(to preset: NativeDecisionCatalog.Preset) -> Self {
        Self(enabled: enabled, preset: preset.id, base_url: preset.base_url, model: preset.model, think_mode: "", timeout_secs: preset.timeout_secs, stored_keys: stored_keys)
    }
    func status(in catalog: NativeDecisionCatalog) -> String {
        let name = catalog.text("decision.preset.\(preset)")
        if !enabled { return catalog.text("decision.status_off") }
        if !usable(in: catalog) { return catalog.text("decision.status_no_key").replacingOccurrences(of: "{preset}", with: name) }
        return catalog.text("decision.status_on").replacingOccurrences(of: "{preset}", with: name)
            .replacingOccurrences(of: "{model}", with: model.isEmpty ? base_url : model)
    }
    var value: SettingsValue { (try? JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(self))) ?? .null }
    static func decode(_ value: SettingsValue) throws -> Self { try JSONDecoder().decode(Self.self, from: JSONEncoder().encode(value)) }
}

extension NativeDecisionSettings {
    // `#[serde(default)]` on the Rust side: absent fields keep their defaults.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? false
        preset = try c.decodeIfPresent(String.self, forKey: .preset) ?? ""
        base_url = try c.decodeIfPresent(String.self, forKey: .base_url) ?? ""
        model = try c.decodeIfPresent(String.self, forKey: .model) ?? ""
        think_mode = try c.decodeIfPresent(String.self, forKey: .think_mode) ?? ""
        timeout_secs = try c.decodeIfPresent(Int64.self, forKey: .timeout_secs) ?? 0
        stored_keys = try c.decodeIfPresent([String].self, forKey: .stored_keys) ?? []
    }
}

/// Settings → 模型 → 决策模型, the same form as `DecisionModelSettingsView`.
struct NativeDecisionModelSettings: View {
    let client: any NativeSettingsQuerying
    private let catalog = NativeDecisionCatalog.shared
    @State private var saved: NativeDecisionSettings?
    @State private var draft = NativeDecisionSettings()
    @State private var apiKey = ""
    @State private var busy = false
    @State private var message: (ok: Bool, text: String)?
    @Environment(\.colorScheme) private var scheme

    private var preset: NativeDecisionCatalog.Preset { catalog.preset(draft.preset) ?? catalog.presets[0] }
    private var keyStored: Bool { draft.hasKey(draft.preset) }
    // Hosted services reject keyless calls; say so before the round trip.
    private var missingKey: Bool { preset.key_required && !keyStored && apiKey.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(catalog.text("decision.desc")).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let saved {
                Text(saved.status(in: catalog)).font(.callout.weight(.medium))
                    .foregroundStyle(saved.enabled && saved.usable(in: catalog) ? WispDesign.color("clay", scheme) : .secondary)
                    .accessibilityIdentifier("decision-status")
                form
            } else if let message {
                Text(message.text).foregroundStyle(.red).textSelection(.enabled)
            } else {
                ProgressView()
            }
        }
        .task { await load() }
    }

    private var form: some View {
        NativeSettingsGroup(title: "决策模型") {
            NativePreferenceRow(title: catalog.text("decision.enabled")) {
                Toggle("", isOn: Binding(get: { draft.enabled }, set: { draft.enabled = $0; message = nil }))
                    .toggleStyle(.switch).labelsHidden().disabled(busy).accessibilityIdentifier("decision-enabled")
            }
            field(catalog.text("decision.preset")) {
                Picker("", selection: Binding(get: { draft.preset }, set: { id in
                    guard let next = catalog.preset(id) else { return }
                    draft = draft.switched(to: next); apiKey = ""; message = nil
                })) {
                    ForEach(catalog.presets, id: \.id) { Text(catalog.text("decision.preset.\($0.id)")).tag($0.id) }
                }.labelsHidden().disabled(busy).accessibilityIdentifier("decision-preset")
                HStack(alignment: .firstTextBaseline) {
                    Text(catalog.text("decision.preset_hint.\(preset.id)")).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    if let url = URL(string: preset.key_url), !preset.key_url.isEmpty {
                        Button(catalog.text("decision.link.\(preset.id)")) { NSWorkspace.shared.open(url) }.buttonStyle(.link).font(.caption)
                    }
                }
            }
            field(catalog.text("decision.base_url")) {
                TextField("", text: Binding(get: { draft.base_url }, set: { draft.base_url = $0; message = nil }),
                          prompt: Text(preset.base_url.isEmpty ? "https://decide.example.org" : preset.base_url))
                    .textFieldStyle(NativeSettingsTextFieldStyle()).disabled(busy)
            }
            HStack(alignment: .top, spacing: 16) {
                field(catalog.text("decision.model")) {
                    TextField("", text: Binding(get: { draft.model }, set: { draft.model = $0; message = nil }), prompt: Text(catalog.text("decision.model_placeholder")))
                        .textFieldStyle(NativeSettingsTextFieldStyle()).disabled(busy)
                }
                field(catalog.text("decision.think")) {
                    Picker("", selection: Binding(get: { draft.think_mode }, set: { draft.think_mode = $0; message = nil })) {
                        ForEach(catalog.think_modes, id: \.self) { Text(catalog.text("decision.think.\($0.isEmpty ? "default" : $0)")).tag($0) }
                    }.labelsHidden().disabled(busy)
                }
                field(catalog.text("decision.timeout")) {
                    TextField("", text: Binding(get: { String(draft.timeout_secs) }, set: { draft.timeout_secs = Int64($0.trimmingCharacters(in: .whitespaces)) ?? 0; message = nil }))
                        .textFieldStyle(NativeSettingsTextFieldStyle()).frame(width: 90).disabled(busy)
                }
            }
            Text(catalog.text("decision.think_hint")).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            field(catalog.text("decision.api_key") + " — " + (keyStored ? localized("已配置") : preset.key_required ? localized("未配置") : catalog.text("decision.key_optional"))) {
                HStack {
                    SecureField("", text: $apiKey, prompt: keyStored ? Text(localized("（已保存 — 留空则保持不变）")) : nil)
                        .textFieldStyle(NativeSettingsTextFieldStyle()).disabled(busy)
                    if keyStored { Button(localized("清除")) { Task { await clearKey() } }.disabled(busy).accessibilityIdentifier("decision-clear-key") }
                }
            }
            Text(catalog.text("decision.egress")).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let message {
                Text(message.text).font(.caption).foregroundStyle(message.ok ? Color.secondary : Color.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button(catalog.text("decision.test")) { Task { await test() } }.disabled(busy).accessibilityIdentifier("decision-test")
                Button(localized("保存")) { Task { await save() } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).disabled(busy).accessibilityIdentifier("decision-save")
            }
        }
    }

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(WispDesign.font(size: 12, weight: .medium))
            content()
        }
    }

    private var args: [String: SettingsValue] { ["settings": draft.value, "apiKey": .string(apiKey)] }

    private func load() async {
        do {
            let value = try NativeDecisionSettings.decode(try await client.invoke("get_decision_model", args: [:], projectID: nil))
            saved = value; draft = value
        } catch { message = (false, error.localizedDescription) }
    }
    private func save() async {
        guard !busy, saved != nil else { return }
        if draft.enabled && missingKey { message = (false, catalog.text("decision.key_required")); return }
        busy = true; message = nil
        defer { busy = false }
        do {
            let value = try NativeDecisionSettings.decode(try await client.invoke("save_decision_model", args: args, projectID: nil))
            saved = value; draft = value; apiKey = ""
            message = (true, catalog.text("decision.saved"))
        } catch { message = (false, error.localizedDescription) }
    }
    private func test() async {
        guard !busy, saved != nil else { return }
        if missingKey { message = (false, catalog.text("decision.key_required")); return }
        busy = true; message = (true, catalog.text("decision.testing"))
        defer { busy = false }
        do {
            let probe = try await client.invoke("test_decision_model", args: args, projectID: nil)
            let model = probe["model"].string
            message = (true, catalog.text("decision.test_ok").replacingOccurrences(of: "{model}", with: model.isEmpty ? "?" : model)
                .replacingOccurrences(of: "{ms}", with: String(probe["elapsed_ms"].integer)))
        } catch {
            message = (false, catalog.text("decision.test_failed").replacingOccurrences(of: "{error}", with: error.localizedDescription))
        }
    }
    private func clearKey() async {
        guard !busy else { return }
        busy = true; message = nil
        defer { busy = false }
        do {
            let value = try NativeDecisionSettings.decode(try await client.invoke("clear_decision_model_key", args: ["preset": .string(draft.preset)], projectID: nil))
            // Only the stored keys changed; keep unsaved form edits.
            draft.stored_keys = value.stored_keys; saved = value
            message = (true, catalog.text("decision.key_cleared"))
        } catch { message = (false, error.localizedDescription) }
    }
}
