import AppKit
import PDFKit
import SwiftUI
import WispProjectBrowser

enum NativeDocumentKind: String {
    case pdf, docx, xlsx, pptx, html, structure, molecule, fasta, msa
    static func detect(_ content: NativePanelFileContent) -> Self? {
        let ext = (content.path as NSString).pathExtension.lowercased()
        switch ext {
        case "pdf": return .pdf
        case "docx": return .docx
        case "xlsx": return .xlsx
        case "pptx": return .pptx
        case "html", "htm": return .html
        case "pdb", "cif", "mmcif", "mol2": return .structure
        case "smi", "smiles", "sdf", "mol": return .molecule
        case "fa", "fasta", "fas", "fna", "faa", "ffn", "frn": return .fasta
        case "aln", "clustal", "clustalw", "sto", "stockholm", "stk", "afa", "mfa": return .msa
        default: return nil
        }
    }
    var textual: Bool { [.html, .structure, .molecule, .fasta, .msa].contains(self) }
    func format(path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext { case "mmcif": return "cif"; case "aln", "clustal", "clustalw": return "clustal"; case "sto", "stockholm", "stk": return "stockholm"; case "afa", "mfa": return "fasta"; default: return ext }
    }
}

/// A mounted renderer supplies position only; the owning model supplies path,
/// project and session. Never accept a path from HTML or a document message.
struct NativeDocumentSelection: Equatable {
    let text: String
    let kind: NativeDocumentKind
    var page: Int? = nil
    var endPage: Int? = nil
    var sheet: String? = nil
    var cells: String? = nil
    var valid: Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.utf8.count <= 65536 else { return false }
        switch kind {
        case .pdf, .docx, .pptx:
            guard let page, (1...1_000_000).contains(page), sheet == nil, cells == nil else { return false }
            return endPage.map { (page...1_000_000).contains($0) } ?? true
        case .xlsx:
            guard let sheet, !sheet.isEmpty, sheet.count <= 256, !sheet.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains), let cells, page == nil, endPage == nil else { return false }
            return cells.range(of: "^[A-Z]{1,3}[1-9][0-9]{0,6}(:[A-Z]{1,3}[1-9][0-9]{0,6})?$", options: .regularExpression) != nil
        case .html, .fasta: return page == nil && endPage == nil && sheet == nil && cells == nil
        default: return false
        }
    }
    var location: String {
        switch kind {
        case .xlsx: return localized("工作表") + " \(sheet ?? "") · \(cells ?? "")"
        case .html: return localized("HTML 阅读视图")
        case .fasta: return localized("序列阅读视图")
        default: return localized(kind == .pptx ? "幻灯片" : "页") + " \(page ?? 0)" + (endPage == nil || endPage == page ? "" : "–\(endPage!)")
        }
    }
    func quote(from original: NativePanelFileContent) -> NativeSideChatQuote? {
        guard valid, NativeDocumentKind.detect(original) == kind, !original.truncated,
              !original.path.isEmpty, !original.path.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else { return nil }
        return NativeSideChatQuote(text: text, source: original.path + " · " + location)
    }
    static func parse(_ data: SettingsValue, expected: NativeDocumentKind) -> Self? {
        let position = data["location"]
        let kind = position["kind"].string.isEmpty ? (data["page"].integer > 0 ? "pdf" : "") : position["kind"].string
        guard kind == expected.rawValue else { return nil }
        func optional(_ value: SettingsValue) -> Int? { value == .null ? nil : Int(exactly: value.integer) }
        let result = Self(text: data["text"].string, kind: expected,
            page: optional(position == .null ? data["page"] : position["page"]), endPage: optional(position["endPage"]),
            sheet: position["sheet"] == .null ? nil : position["sheet"].string, cells: position["cells"] == .null ? nil : position["cells"].string)
        return result.valid ? result : nil
    }
}

@MainActor
final class NativePDFModel: ObservableObject {
    let document: PDFDocument?
    @Published var page = 1
    @Published var query = ""
    @Published var selection: NativeDocumentSelection?
    @Published var error: String?
    @Published var matches: [PDFSelection] = []
    @Published var matchIndex = 0
    weak var view: PDFView?
    init(content: NativePanelFileContent) {
        if !content.truncated, let encoded = content.base64, encoded.utf8.count <= 45 * 1024 * 1024,
           let bytes = Data(base64Encoded: encoded), bytes.count <= 32 * 1024 * 1024,
           let document = PDFDocument(data: bytes), document.pageCount > 0, document.pageCount <= 5000 {
            self.document = document
        } else { document = nil; error = localized("PDF 数据不完整、损坏或超过预览限制。") }
    }
    func navigate(_ page: Int) {
        guard let document, (1...document.pageCount).contains(page), let target = document.page(at: page - 1) else { return }
        view?.clearSelection(); view?.go(to: target); self.page = page; selection = nil
    }
    func search() {
        guard let document, !query.isEmpty, query.utf8.count <= 512 else { matches = []; return }
        matches = Array(document.findString(query, withOptions: [.caseInsensitive]).prefix(100)); matchIndex = 0
        showMatch()
    }
    func nextMatch() { guard !matches.isEmpty else { return }; matchIndex = (matchIndex + 1) % matches.count; showMatch() }
    private func showMatch() { guard matches.indices.contains(matchIndex) else { return }; view?.setCurrentSelection(matches[matchIndex], animate: true); view?.go(to: matches[matchIndex]); selected() }
    func selected() {
        guard let document, let current = view?.currentSelection, let text = current.string else { selection = nil; return }
        let positions = current.pages.map { document.index(for: $0) + 1 }
        guard let first = positions.min(), let last = positions.max(), first > 0, last <= document.pageCount else { selection = nil; return }
        let value = NativeDocumentSelection(text: text, kind: .pdf, page: first, endPage: last)
        selection = value.valid ? value : nil
    }
}

struct NativePDFSurface: NSViewRepresentable {
    @ObservedObject var model: NativePDFModel
    func makeCoordinator() -> Coordinator { Coordinator(model: model) }
    func makeNSView(context: Context) -> PDFView {
        let view = PDFView(); view.document = model.document; view.autoScales = true; view.displayMode = .singlePageContinuous
        view.backgroundColor = .windowBackgroundColor; model.view = view
        context.coordinator.observe(view)
        return view
    }
    func updateNSView(_ view: PDFView, context: Context) {}
    static func dismantleNSView(_ view: PDFView, coordinator: Coordinator) { coordinator.stop(); coordinator.model.view = nil; view.document = nil }
    final class Coordinator {
        let model: NativePDFModel
        var observers: [NSObjectProtocol] = []
        init(model: NativePDFModel) { self.model = model }
        func observe(_ view: PDFView) {
            observers = [NotificationCenter.default.addObserver(forName: .PDFViewPageChanged, object: view, queue: .main) { [weak self, weak view] _ in
                Task { @MainActor [weak self, weak view] in
                    guard let self, let view, self.model.view === view, let page = view.currentPage, let document = self.model.document else { return }; self.model.page = document.index(for: page) + 1
                }
            }, NotificationCenter.default.addObserver(forName: .PDFViewSelectionChanged, object: view, queue: .main) { [weak self, weak view] _ in
                Task { @MainActor [weak self, weak view] in guard let self, let view, self.model.view === view else { return }; self.model.selected() }
            }]
        }
        func stop() { observers.forEach(NotificationCenter.default.removeObserver); observers = [] }
        deinit { stop() }
    }
}

struct NativePDFPreview: View {
    @StateObject private var model: NativePDFModel
    var quote: ((NativeDocumentSelection) -> Void)?
    init(content: NativePanelFileContent, quote: ((NativeDocumentSelection) -> Void)?) { _model = StateObject(wrappedValue: NativePDFModel(content: content)); self.quote = quote }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let error = model.error { Text(error).foregroundStyle(.orange) }
            if let document = model.document {
                ViewThatFits(in: .horizontal) {
                    HStack { navigation(document); Spacer(); search }
                    VStack(alignment: .leading) { navigation(document); search }
                }
                NativePDFSurface(model: model)
                if let quote { Button(localized("引用选中文字")) { if let selection = model.selection { quote(selection) } }.disabled(model.selection == nil) }
            }
        }
    }
    private func navigation(_ document: PDFDocument) -> some View {
        HStack {
            Button { model.navigate(model.page - 1) } label: { WispIcon(name: "chevron-left") }.disabled(model.page <= 1).accessibilityLabel(localized("上一页"))
            Text("\(model.page) / \(document.pageCount)")
            Button { model.navigate(model.page + 1) } label: { WispIcon(name: "chevron-right") }.disabled(model.page >= document.pageCount).accessibilityLabel(localized("下一页"))
            Button(localized("缩小")) { if let view = model.view { view.scaleFactor = max(view.minScaleFactor, view.scaleFactor / 1.2) } }
            Button(localized("放大")) { if let view = model.view { view.scaleFactor = min(view.maxScaleFactor, view.scaleFactor * 1.2) } }
        }.buttonStyle(WispButtonStyle())
    }
    private var search: some View {
        ViewThatFits(in: .horizontal) {
            HStack { searchInput; searchActions }
            VStack(alignment: .leading) { searchInput; searchActions }
        }.textFieldStyle(NativeSettingsTextFieldStyle()).frame(maxWidth: 420)
    }
    private var searchInput: some View {
        TextField(localized("搜索 PDF 文本"), text: $model.query).onSubmit { model.search() }.frame(minWidth: 80, idealWidth: 180, maxWidth: .infinity)
    }
    private var searchActions: some View {
        HStack {
            Button(localized("查找")) { model.search() }
            if !model.matches.isEmpty { Text("\(model.matchIndex + 1)/\(model.matches.count)"); Button(localized("下一个")) { model.nextMatch() } }
        }.fixedSize()
    }
}
