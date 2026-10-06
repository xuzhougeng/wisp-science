import AppKit
import SwiftUI
import WispProjectBrowser

struct SearchResultSelection {
    private(set) var index = 0
    mutating func move(_ delta: Int, count: Int) { index = min(max(0, index + delta), max(0, count - 1)) }
    mutating func reset() { index = 0 }
    func selectedIndex(count: Int) -> Int? { count > 0 ? min(index, count - 1) : nil }
}

struct ProjectSearchSheet: View {
    @ObservedObject var model: ProjectBrowserModel
    var projectID: String? = nil
    let close: () -> Void
    @StateObject private var search: NativeSearchModel
    @State private var query = ""
    @State private var selection = SearchResultSelection()
    @Environment(\.colorScheme) private var scheme
    init(model: ProjectBrowserModel, projectID: String? = nil, close: @escaping () -> Void) {
        self.model = model; self.projectID = projectID; self.close = close
        _search = StateObject(wrappedValue: NativeSearchModel(client: model.calendarClient(), projectID: projectID))
    }
    private var commands: [NativeSearchCommand] { NativeSearchCommand.matching(query, project: projectID != nil, session: model.activeSessionID != nil) }
    private var count: Int { commands.count + search.items.count }
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                WispIcon(name: "search")
                SearchCommandField(text: $query, cancel: close, move: { selection.move($0, count: count) }, submit: {
                    if let index = selection.selectedIndex(count: count) { open(index) }
                })
                .frame(height: 24)
                Button("关闭", action: close).keyboardShortcut(.cancelAction)
            }
            Divider()
            Text(localized("搜索所有项目、产物与会话，包括历史消息。输入 > 搜索命令。"))
                .font(.caption).foregroundStyle(color("text-faint"))
            if search.busy { ProgressView().controlSize(.small) }
            if let error = search.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                Button(localized("重新读取")) { Task { await search.search(query, debounce: 0) } }
            }
            ScrollViewReader { scroll in
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        if !commands.isEmpty {
                            Text(localized("命令")).font(.caption).foregroundStyle(color("text-faint"))
                            ForEach(Array(commands.enumerated()), id: \.element.id) { index, command in
                                Button { open(index) } label: {
                                    HStack { WispIcon(name: command.icon); Text(localized(command.title)); Spacer() }
                                        .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                                        .background(selection.selectedIndex(count: count) == index ? color("surface-hover") : .clear, in: RoundedRectangle(cornerRadius: 8))
                                }.buttonStyle(.plain).id(index)
                                    .accessibilityAddTraits(selection.selectedIndex(count: count) == index ? [.isSelected] : [])
                            }
                        }
                        ForEach(Array(search.items.enumerated()), id: \.element.id) { index, item in
                            result(commands.count + index, item: item)
                        }
                        if count == 0 && !search.busy && search.error == nil {
                            Text(localized("没有匹配的项目、产物或会话")).foregroundStyle(color("text-faint")).padding()
                        }
                    }
                }
                .onChange(of: selection.index) { index in scroll.scrollTo(index) }
            }
            Divider()
            Text("↑↓ 选择    ↵ 打开    esc 关闭").font(.caption).foregroundStyle(color("text-faint"))
        }
        .padding(24).frame(width: 560, height: 460).background(color("bg-app"))
        .onChange(of: query) { _ in selection.reset(); search.invalidate() }
        .onChange(of: search.items) { _ in selection.reset() }
        .task(id: query) { if query.hasPrefix(">") { search.invalidate() } else { await search.search(query) } }
        .onDisappear { search.invalidate() }
        .background(NativeSettingsEscape(close: close))
        .onExitCommand(perform: close)
    }

    private func result(_ index: Int, item: NativeSearchItem) -> some View {
        Button { open(index) } label: {
            HStack {
                WispIcon(name: item.kind == "project" ? "folder" : item.kind == "artifact" ? "doc" : "chat")
                VStack(alignment: .leading, spacing: 3) {
                    Text(item.title).lineLimit(1)
                    Text(item.detail).font(.caption).foregroundStyle(color("text-faint")).lineLimit(1)
                }
                Spacer()
                Text(localized(item.kind == "project" ? "项目" : item.kind == "artifact" ? "产物" : "会话"))
                    .font(.caption).foregroundStyle(color("text-faint"))
            }
                .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                .background(selection.selectedIndex(count: count) == index ? color("surface-hover") : .clear,
                            in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain).id(index)
        .accessibilityAddTraits(selection.selectedIndex(count: count) == index ? [.isSelected] : [])
    }

    private func open(_ index: Int) {
        guard index >= 0 && index < count else { return }
        if index < commands.count {
            let command = commands[index]; close(); model.executeSearchCommand(command); return
        }
        let item = search.items[index - commands.count]
        close()
        Task { await model.openSearchResult(item) }
    }
}

/// AppKit's field editor consumes arrow keys before SwiftUI's onMoveCommand.
/// Handle its navigation commands while preserving IME candidate selection.
private struct SearchCommandField: NSViewRepresentable {
    @Binding var text: String
    let cancel: () -> Void
    let move: (Int) -> Void
    let submit: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.placeholderString = localized("搜索项目、产物、会话与历史消息…")
        field.font = .systemFont(ofSize: 14)
        field.delegate = context.coordinator
        field.setAccessibilityIdentifier("project-search")
        DispatchQueue.main.async { field.window?.makeFirstResponder(field) }
        return field
    }
    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: SearchCommandField
        init(_ parent: SearchCommandField) { self.parent = parent }
        func controlTextDidChange(_ notification: Notification) {
            if let field = notification.object as? NSTextField { parent.text = field.stringValue }
        }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            guard !textView.hasMarkedText() else { return false }
            switch selector {
            case #selector(NSResponder.moveDown(_:)): parent.move(1)
            case #selector(NSResponder.moveUp(_:)): parent.move(-1)
            case #selector(NSResponder.insertNewline(_:)): parent.submit()
            default: return false
            }
            return true
        }
    }
}
