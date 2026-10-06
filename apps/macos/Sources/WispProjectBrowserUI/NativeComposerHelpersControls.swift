import SwiftUI
import WispProjectBrowser

struct NativeComposerHelpersControls: View {
    @ObservedObject var model: NativeComposerHelpersModel
    @Binding var reviewerPicker: Bool
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(localized("全局设置")).font(.subheadline).bold()
            Text(localized("以下设置影响所有项目和会话。")) .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if model.busy { ProgressView().controlSize(.small) }
            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                Button(localized("重新读取全局设置")) { Task { await model.load() } }.disabled(model.busy)
            }
            if let memory = model.memory {
                Toggle(localized("启用记忆"), isOn: Binding(get: { memory.enabled }, set: { enabled in Task { await model.setMemory(enabled) } }))
                    .disabled(!model.canEdit)
            }
            if let analysis = model.analysis {
                VStack(alignment: .leading, spacing: 12) {
                    Toggle(localized("自动失败分析"), isOn: Binding(get: { analysis.enabled }, set: { enabled in
                        var next = analysis; next.enabled = enabled; Task { await model.setAnalysis(next) }
                    }))
                    if analysis.enabled {
                        Stepper(value: Binding(get: { analysis.failure_rate_threshold }, set: { threshold in
                            var next = analysis; next.failure_rate_threshold = threshold; Task { await model.setAnalysis(next) }
                        }), in: 1...100) { Text("\(localized("失败率阈值")) · \(analysis.failure_rate_threshold)%") }
                        Stepper(value: Binding(get: { analysis.minimum_failures }, set: { failures in
                            var next = analysis; next.minimum_failures = failures; Task { await model.setAnalysis(next) }
                        }), in: 1...100) { Text("\(localized("最少失败次数")) · \(analysis.minimum_failures)") }
                    }
                }.disabled(!model.canEdit)
            }
            if model.reviewer != nil {
                HStack(alignment: .top) {
                    Text(localized("审查模型"))
                    Spacer()
                    Button { reviewerPicker = true } label: {
                        HStack(alignment: .top, spacing: 6) {
                            Text(model.reviewerLabel).multilineTextAlignment(.trailing).fixedSize(horizontal: false, vertical: true)
                            WispIcon(name: "chevron-down", size: 12)
                        }
                    }.buttonStyle(.plain).disabled(!model.canEdit)
                        .popover(isPresented: $reviewerPicker) {
                            NativeReviewerBackendPicker(choices: model.choices, selected: model.reviewerKey, close: { reviewerPicker = false }) { key in
                                reviewerPicker = false; Task { await model.setReviewer(key) }
                            }
                        }
                }
            }
        }.toggleStyle(.switch).onChange(of: model.canEdit) { available in if !available { reviewerPicker = false } }
    }
}

struct NativeReviewerBackendPicker: View {
    let choices: [NativeReviewerChoice]
    let selected: String
    let close: () -> Void
    let select: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(localized("审查模型")).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
            Text(localized("切换审查模型会影响所有项目和会话；审查专家的其他配置会保留。")) .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(choices) { choice in
                        Button { select(choice.id) } label: {
                            HStack(alignment: .top, spacing: 10) {
                                if choice.id == selected { WispIcon(name: "check", size: 14) } else { Color.clear.frame(width: 14, height: 14) }
                                Text(choice.label).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                            }.padding(8)
                        }.buttonStyle(.plain)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 280)
        }.padding(16).frame(width: 350)
            .background(NativeSettingsEscape(close: close))
    }
}
