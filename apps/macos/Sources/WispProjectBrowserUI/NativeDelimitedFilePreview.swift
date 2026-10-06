import AppKit
import SwiftUI

struct NativeDelimitedFilePreview: View {
    let table: NativeDelimitedTable
    let quote: ((String) -> Void)?
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("\(table.rows.count) " + localized("行") + " · \(table.columnCount) " + localized("列")).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(localized("复制表格 TSV")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(table.copyText, forType: .string) }
            }
            if table.rows.count > NativeDelimitedTable.visibleRows { Text(localized("仅显示前 500 行；复制包含已读取的全部行。")).font(.caption).foregroundStyle(.secondary) }
            if table.columnCount > NativeDelimitedTable.visibleColumns { Text(localized("仅显示前 128 列；复制包含已读取的全部列。")).font(.caption).foregroundStyle(.secondary) }
            if table.omittedPartialRecord { Text(localized("文件开头的最后一条不完整记录未显示；可在源文本中查看。")).font(.caption).foregroundStyle(.orange) }
            if table.columnCount == 0 { Text(localized("表格为空。")); Spacer() }
            else { NativeDelimitedTableView(table: table, quote: quote).frame(maxWidth: .infinity, maxHeight: .infinity) }
        }
    }
}

struct NativeDelimitedTableView: NSViewRepresentable {
    let table: NativeDelimitedTable
    let quote: ((String) -> Void)?
    @Environment(\.colorScheme) private var scheme
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasHorizontalScroller = true; scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.borderType = .bezelBorder
        let view = NativeDelimitedTableControl()
        view.usesAlternatingRowBackgroundColors = true; view.allowsMultipleSelection = true; view.allowsEmptySelection = true
        view.columnAutoresizingStyle = .noColumnAutoresizing; view.rowHeight = 30
        view.dataSource = context.coordinator; view.delegate = context.coordinator
        scroll.documentView = view
        updateNSView(scroll, context: context)
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? NativeDelimitedTableControl else { return }
        let changed = context.coordinator.table != table
        context.coordinator.table = table; context.coordinator.scheme = scheme; context.coordinator.quote = quote
        view.quote = quote; view.table = table
        scroll.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        if changed || view.tableColumns.isEmpty {
            for column in view.tableColumns { view.removeTableColumn(column) }
            for index in 0..<min(table.columnCount, NativeDelimitedTable.visibleColumns) {
                let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(String(index)))
                column.title = index < table.headers.count && !table.headers[index].isEmpty ? NativeDelimitedTable.tsv([table.headers[index]]) : localized("列") + " \(index + 1)"
                let length = ([column.title] + table.rows.prefix(40).map { index < $0.count ? $0[index] : "" }).map(\.utf16.count).max() ?? 8
                column.width = min(240, max(90, CGFloat(length) * 7 + 20)); column.minWidth = 60; column.maxWidth = 800
                view.addTableColumn(column)
            }
            view.deselectAll(nil)
        }
        view.reloadData()
    }
    static func dismantleNSView(_ scroll: NSScrollView, coordinator: Coordinator) {
        if let view = scroll.documentView as? NativeDelimitedTableControl { view.quote = nil; view.table = nil; view.dataSource = nil; view.delegate = nil }
    }
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var table: NativeDelimitedTable?
        var scheme = ColorScheme.light
        var quote: ((String) -> Void)?
        func numberOfRows(in tableView: NSTableView) -> Int { min(table?.rows.count ?? 0, NativeDelimitedTable.visibleRows) }
        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let table, let id = tableColumn?.identifier, let column = Int(id.rawValue), table.rows.indices.contains(row) else { return nil }
            let value = column < table.rows[row].count ? table.rows[row][column] : ""
            let cell = tableView.makeView(withIdentifier: id, owner: self) as? NativeDelimitedCellView ?? NativeDelimitedCellView()
            cell.identifier = id; cell.content.tableView = tableView as? NativeDelimitedTableControl
            cell.content.apply(NSAttributedString(string: NativeDelimitedTable.tsv([value]), attributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor(WispDesign.color("text", scheme))]))
            cell.content.toolTip = value; cell.content.quote = quote
            cell.setAccessibilityLabel(value)
            return cell
        }
    }
}

final class NativeDelimitedCellView: NSTableCellView {
    let content = NativeDelimitedCellTextView(frame: .zero)
    override init(frame: NSRect) {
        super.init(frame: frame)
        content.translatesAutoresizingMaskIntoConstraints = false
        content.isEditable = false; content.isSelectable = true; content.isRichText = false; content.drawsBackground = false
        content.textContainerInset = NSSize(width: 6, height: 6)
        content.textContainer?.lineFragmentPadding = 0
        content.textContainer?.widthTracksTextView = true; content.textContainer?.heightTracksTextView = true
        content.textContainer?.maximumNumberOfLines = 1; content.textContainer?.lineBreakMode = .byTruncatingTail
        addSubview(content)
        NSLayoutConstraint.activate([content.leadingAnchor.constraint(equalTo: leadingAnchor), content.trailingAnchor.constraint(equalTo: trailingAnchor), content.topAnchor.constraint(equalTo: topAnchor), content.bottomAnchor.constraint(equalTo: bottomAnchor)])
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}

final class NativeDelimitedCellTextView: NativeMessageTextView {
    weak var tableView: NativeDelimitedTableControl?
    override func mouseDown(with event: NSEvent) {
        if event.clickCount < 2, let tableView { tableView.mouseDown(with: event) }
        else { super.mouseDown(with: event) }
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        selectedRange().length > 0 ? super.menu(for: event) : tableView?.menu(for: event)
    }
}

final class NativeDelimitedTableControl: NSTableView {
    var table: NativeDelimitedTable?
    var quote: ((String) -> Void)?
    var copyText: (String) -> Void = { NSPasteboard.general.clearContents(); NSPasteboard.general.setString($0, forType: .string) }
    func selectionActions() -> [NSMenuItem] {
        guard let text = table?.selectedText(selectedRowIndexes), !text.isEmpty else { return [] }
        return [("复制所选行", "copy", Optional(copyText)), ("引用到侧聊", "chat", quote)].compactMap { label, icon, callback in
            guard let callback else { return nil }
            let action = NativeSelectionAction { callback(text) }
            let item = NSMenuItem(title: localized(label), action: #selector(NativeSelectionAction.invoke(_:)), keyEquivalent: "")
            item.target = action; item.representedObject = action
            let image = WispDesign.image("icon-" + icon); image.size = NSSize(width: 16, height: 16); item.image = image
            return item
        }
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        let row = self.row(at: convert(event.locationInWindow, from: nil))
        if row >= 0, !selectedRowIndexes.contains(row) { selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        let menu = NSMenu(); selectionActions().forEach(menu.addItem); return menu
    }
    @objc func copy(_ sender: Any?) {
        guard let text = table?.selectedText(selectedRowIndexes), !text.isEmpty else { return }
        copyText(text)
    }
}
