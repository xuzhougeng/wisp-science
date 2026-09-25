import SwiftUI
import WispProjectBrowser

struct NativeSideChatView: View {
    @ObservedObject var model: NativeSideChatModel
    @State private var quoteText = ""
    @State private var quoteSource = ""
    @State private var quoteEditor = false
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack { Text("侧聊").font(.headline); Spacer(); Button("清空") { model.clear() }.disabled(model.busy || model.rows.isEmpty) }
            ScrollViewReader { scroll in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        if model.rows.isEmpty {
                            Text("关于当前会话的侧聊").font(.headline)
                            Text("只读检索会话记录，不打断主任务，也不会写入主对话。").font(.caption).foregroundStyle(.secondary)
                        }
                        ForEach(model.rows) { row in
                            VStack(alignment: .leading, spacing: 8) {
                                if let reply = row.answer {
                                    if reply.noEvidence { Text("当前会话中没有找到足够的证据来回答这个问题。").foregroundStyle(.secondary) }
                                    else { NativeShareMarkdown(text: reply.answer).textSelection(.enabled) }
                                    if !reply.noEvidence, let name = row.model { Text(name).font(.caption2).foregroundStyle(.secondary) }
                                    if !reply.evidence.isEmpty {
                                        DisclosureGroup("\(reply.evidence.count) 条证据 · 快照 \(reply.snapshotVersion)") {
                                            ForEach(Array(reply.evidence.enumerated()), id: \.offset) { index, source in
                                                VStack(alignment: .leading, spacing: 5) {
                                                    Text("[S\(index + 1)] · 第 \(source.turn) 轮 · \(source.role) · " + (source.eventSeq.map { "event \($0)" } ?? source.messageSeq.map { "message \($0)" } ?? source.sourceId)).font(.caption2).foregroundStyle(.secondary)
                                                    Text(source.excerpt).font(.caption).textSelection(.enabled)
                                                    if !source.relevance.isEmpty { Text(source.relevance).font(.caption2).foregroundStyle(.secondary) }
                                                }.padding(.vertical, 6)
                                            }
                                        }.font(.caption)
                                    }
                                } else if let error = row.error {
                                    Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                                    Button("将问题放回输入框") { model.draft = row.question }
                                } else { Text(row.question).font(.system(size: 13)).textSelection(.enabled) }
                            }.padding(10).frame(maxWidth: .infinity, alignment: .leading)
                                .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8)).id(row.id)
                        }
                        if model.busy { HStack { ProgressView().controlSize(.small); Text("正在检索并回答…").font(.caption) } }
                        Color.clear.frame(height: 1).id("end")
                    }
                }.onChange(of: model.rows.count) { _ in scroll.scrollTo("end", anchor: .bottom) }
            }
            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.orange)
                Button("刷新模型状态") { Task { await model.loadOptions() } }.disabled(model.changingModel)
            }
            ForEach(model.quotes) { quote in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 3) { Text(quote.text).lineLimit(2); if !quote.source.isEmpty { Text(quote.source).foregroundStyle(.secondary).lineLimit(1) } }.font(.caption)
                    Spacer(minLength: 4)
                    Button { model.quotes.removeAll { $0.id == quote.id } } label: { WispIcon(name: "close", size: 12) }.buttonStyle(.plain).help("移除引用")
                }
            }
            DisclosureGroup("添加只读引用", isExpanded: $quoteEditor) {
                TextField("来源（可选）", text: $quoteSource)
                TextEditor(text: $quoteText).font(.system(size: 12)).frame(height: 55)
                Button("添加引用") {
                    model.quotes.append(NativeSideChatQuote(text: quoteText, source: quoteSource))
                    quoteText = ""; quoteSource = ""; quoteEditor = false
                }.disabled(quoteText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.font(.caption)
            NativeMessageInput(text: $model.draft, canSubmit: { model.canSend }, submit: { Task { await model.send() } })
                .frame(minHeight: 55, maxHeight: 95)
            HStack {
                Menu {
                    ForEach(model.options, id: \.key) { option in
                        Button((option.kind == "acp" ? "ACP · " : "") + option.label) { Task { await model.select(option) } }
                    }
                } label: { Text(model.selected?.label ?? "选择模型").lineLimit(1) }.disabled(model.busy || model.changingModel)
                Spacer()
                Button("发送") { Task { await model.send() } }.disabled(!model.canSend)
            }
            Text("Enter 发送 · Shift+Enter 换行").font(.caption2).foregroundStyle(.secondary)
        }.task { await model.loadOptions() }
    }
}
