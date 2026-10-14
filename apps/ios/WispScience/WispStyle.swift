import SwiftUI
import UIKit

/// Shared surfaces and accents for the transcript, composer and workspace lists.
enum WispStyle {
    static let canvas = Color(uiColor: .systemBackground)
    static let surface = Color(uiColor: .secondarySystemBackground)
    static let border = Color(uiColor: .separator).opacity(0.32)
    static let accent = adaptive(light: 0x176A60, dark: 0x83D5C5)
    static let onAccent = adaptive(light: 0xFFFFFF, dark: 0x102E29)
    static let success = adaptive(light: 0x286A45, dark: 0x8CD5A8)
    static let failure = adaptive(light: 0xB33339, dark: 0xFF999C)
    static let warning = adaptive(light: 0x875710, dark: 0xEFC27B)

    private static func adaptive(light: UInt32, dark: UInt32) -> Color {
        Color(uiColor: UIColor { traits in
            let value = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(red: CGFloat((value >> 16) & 255) / 255,
                           green: CGFloat((value >> 8) & 255) / 255,
                           blue: CGFloat(value & 255) / 255, alpha: 1)
        })
    }
}

/// Assets are exported from compose_icon() by scripts/export_icons.py.
/// Keep the WebView's shared glyphs; do not substitute a second icon family.
struct WispIcon: View {
    enum Kind: String {
        case list, notebook, camera, square, checkSquare = "check-square"
        case image, calendar, researchTrail = "research-trail", arrowUp = "arrow-up"
        case chevronRight = "chevron-right", chevronDown = "chevron-down", chevronLeft = "chevron-left"
    }
    let kind: Kind
    @ScaledMetric private var size: CGFloat

    init(_ kind: Kind, size: CGFloat = 20) {
        self.kind = kind
        _size = ScaledMetric(wrappedValue: size, relativeTo: .body)
    }
    var body: some View {
        Image("wisp-" + kind.rawValue).renderingMode(.template).resizable()
            .scaledToFit().frame(width: size, height: size).accessibilityHidden(true)
    }
}

struct WispRowButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(configuration.isPressed ? Color.primary.opacity(0.05) : .clear)
            .opacity(enabled ? 1 : 0.45)
    }
}

/// A full-width, leading-aligned disclosure independent of OS button styling.
struct WispDisclosureStyle: DisclosureGroupStyle {
    var identifier = "tool-details"
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.18)) {
                    configuration.isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 10) {
                    configuration.label.frame(maxWidth: .infinity, alignment: .leading)
                    WispIcon(.chevronRight, size: 15)
                        .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12).padding(.vertical, 11)
                .frame(minHeight: 48).contentShape(Rectangle())
            }
            .buttonStyle(WispRowButtonStyle())
            .accessibilityIdentifier(identifier)
            .accessibilityValue(configuration.isExpanded ? "已展开" : "已折叠")
            .accessibilityHint(configuration.isExpanded ? "收起详细内容" : "查看详细内容")
            if configuration.isExpanded {
                Divider().padding(.horizontal, 12)
                VStack(alignment: .leading, spacing: 8) { configuration.content }
                    .frame(maxWidth: .infinity, alignment: .leading).padding(12)
            }
        }
        .background(WispStyle.surface, in: RoundedRectangle(cornerRadius: 12))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(WispStyle.border, lineWidth: 0.5) }
    }
}

struct WispSendButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(enabled ? WispStyle.onAccent : Color(uiColor: .tertiaryLabel))
            .padding(.horizontal, 14).frame(minWidth: 44, minHeight: 44)
            .background(enabled ? WispStyle.accent : Color(uiColor: .tertiarySystemFill),
                        in: RoundedRectangle(cornerRadius: 12))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}
