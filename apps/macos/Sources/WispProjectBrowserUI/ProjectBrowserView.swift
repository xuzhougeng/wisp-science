import SwiftUI
import WispProjectBrowser

public struct ProjectBrowserView: View {
    @ObservedObject private var model: ProjectBrowserModel
    @ObservedObject private var library: NativeLibraryModel
    @ObservedObject private var calendar: NativeCalendarModel
    @AppStorage("projectBrowser.appearance") private var appearance = "system"

    public init(model: ProjectBrowserModel) {
        self.model = model
        self.library = model.library
        self.calendar = model.calendar
    }

    public var body: some View {
        Group {
            if model.settingsPresented {
                NativeSettingsView(databaseURL: model.databaseURL, projects: model.projects, projectID: model.projectSettingsID ?? model.activeProjectID, editProject: model.projectSettingsID != nil, initialSection: NativeSettingsSection(rawValue: model.settingsSectionID ?? "") ?? .general) { model.settingsPresented = false; model.projectSettingsID = nil; model.settingsSectionID = nil; Task { await model.refresh() } }
            } else if let project = model.projects.first(where: { $0.id == model.activeProjectID }) {
                ProjectWorkspace(model: model, project: project)
            } else {
                ProjectLanding(model: model, appearance: $appearance)
            }
        }
            .preferredColorScheme(appearance == "system" ? nil : (appearance == "dark" ? .dark : .light))
            .sheet(isPresented: $model.searchPresented) {
                ProjectSearchSheet(model: model, projectID: model.activeProjectID, close: { model.searchPresented = false })
            }
            .sheet(isPresented: $model.createPresented) {
                NewProjectSheet(model: model)
            }
            .sheet(isPresented: $library.presented) {
                NativeLibrarySheet(model: model, library: library)
            }
            .sheet(isPresented: $calendar.presented) {
                NativeCalendarSheet(model: model, calendar: calendar)
            }
    }
}

private struct ProjectLanding: View {
    @ObservedObject var model: ProjectBrowserModel
    @Binding var appearance: String
    @Environment(\.colorScheme) private var scheme

    private var projects: [ProjectSummary] { model.projects }
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    header.padding(.bottom, 32)
                    if let error = model.error { errorBanner(error).padding(.bottom, 20) }
                    if let error = model.importError { errorBanner(error).padding(.bottom, 20) }
                    let columns = geometry.size.width < 820
                        ? AnyLayout(VStackLayout(alignment: .leading, spacing: 26))
                        : AnyLayout(HStackLayout(alignment: .top, spacing: 40))
                    columns {
                        projectList.frame(maxWidth: .infinity, alignment: .topLeading)
                        recentSessions.frame(maxWidth: .infinity, alignment: .topLeading)
                    }
                    Spacer(minLength: 40)
                    footer
                }
                .frame(maxWidth: 1200, minHeight: max(0, geometry.size.height - 80), alignment: .topLeading)
                .padding(.horizontal, min(48, max(24, geometry.size.width * 0.04)))
                .padding(.vertical, 40)
                .frame(maxWidth: .infinity)
            }
            .background {
                color("bg-app")
                    .overlay(alignment: .top) {
                        Ellipse().fill(color("clay").opacity(0.08))
                            .frame(width: geometry.size.width * 0.7, height: 180)
                            .blur(radius: 80).offset(y: -150)
                    }
                    .ignoresSafeArea()
            }
        }
        .font(WispDesign.font(size: 14)).foregroundStyle(color("text"))
        .tint(color("clay"))
    }

    private var header: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 24) { brand; Spacer(minLength: 0); actions }
            VStack(alignment: .leading, spacing: 24) {
                brand
                actions.frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }

    private var brand: some View {
        HStack(spacing: 24) {
            Image(nsImage: WispDesign.image(scheme == .dark ? "wordmark-dark" : "wordmark-light"))
                .resizable().scaledToFit().frame(width: 180, height: 119)
                .accessibilityLabel("Wisp Science")
            VStack(alignment: .leading, spacing: 8) {
                Text("严谨做科研，").foregroundStyle(color("text-muted"))
                Text("Wisp Science 在身边。").foregroundStyle(color("clay-strong"))
            }
            .font(WispDesign.font(size: 16, weight: .medium)).fixedSize()
        }
    }

    private var actions: some View {
        HStack(spacing: 8) {
            Button { model.calendar.presented = true } label: { WispIcon(name: "calendar") }
                .buttonStyle(WispButtonStyle()).help("研究日历").accessibilityLabel("研究日历")
                .accessibilityIdentifier("home-calendar")
            Button { model.library.presented = true } label: { WispIcon(name: "star") }
                .buttonStyle(WispButtonStyle()).help("收藏").accessibilityLabel("收藏")
                .accessibilityIdentifier("home-library")
            searchAction
            Button { model.settingsPresented = true } label: { WispIcon(name: "gear") }.buttonStyle(WispButtonStyle()).help("设置").accessibilityLabel("设置")
            WispUnavailableAction(title: "随手一聊")
            Button { model.chooseProjectArchive() } label: {
                HStack(spacing: 8) { WispIcon(name: "upload", size: 16); Text("导入项目") }
            }
            .buttonStyle(WispButtonStyle())
            .disabled(model.importBusy)
            .help("导入项目")
            .accessibilityLabel("导入项目")
            .accessibilityIdentifier("import-project")
            Button { model.createPresented = true } label: {
                HStack(spacing: 8) { WispIcon(name: "plus", size: 16); Text("新建项目") }
            }
            .buttonStyle(WispButtonStyle(primary: true))
            .disabled(model.createBusy)
            .help("新建项目")
            .accessibilityLabel("新建项目")
            .accessibilityIdentifier("new-project")
        }
    }

    private var searchAction: some View {
        Button { model.searchPresented = true } label: { WispIcon(name: "search") }
            .buttonStyle(WispButtonStyle()).help("搜索 · ⌘K").accessibilityLabel("搜索")

    }

    private var projectList: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Text("项目").font(WispDesign.font(size: 18, weight: .semibold))
                Text("\(projects.count)").font(WispDesign.font(size: 12)).foregroundStyle(color("text-faint"))
            }
            if projects.isEmpty {
                emptyState(model.isLoading ? "正在读取本地项目…" : (model.projects.isEmpty ? "还没有项目记录" : "没有匹配的项目"),
                           model.projects.isEmpty ? "选择已有的 Wisp 数据库，读取你的研究项目。" : "试试其他关键词，或关闭收藏筛选。")
            } else {
                LazyVStack(spacing: 10) {
                    ForEach(projects) { project in
                        ProjectCard(project: project, selected: false, busy: model.isLoading, saving: model.savingProjectID == project.id,
                                    toggleStar: { Task { await model.toggleStar(project.id) } },
                                    settings: { model.openProjectSettings(project.id) },
                                    select: { Task { await model.openProject(project.id) } }, reveal: { model.reveal(project) })
                    }
                }
            }
        }
    }

    private var recentSessions: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("最近会话").font(WispDesign.font(size: 18, weight: .semibold))
            if model.recentSessions.isEmpty {
                emptyState("暂无最近会话", "项目中的研究会话会显示在这里。")
            }
            ForEach(model.recentSessions) { session in
                Button { Task { await model.openProject(session.projectID, sessionID: session.id) } } label: {
                    HStack(spacing: 10) {
                        Text(session.title).font(WispDesign.font(size: 14, weight: .semibold)).lineLimit(1)
                        Spacer(minLength: 0)
                        Text(session.status == "needs_you" ? "待查看" : "已完成")
                            .font(WispDesign.font(size: 11)).foregroundStyle(color("text-faint"))
                    }
                    .padding(18).frame(maxWidth: .infinity, alignment: .leading)
                    .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(color("border")))
                }
                .buttonStyle(.plain).accessibilityIdentifier("recent-session-\(session.id)")
            }
        }
    }

    private func emptyState(_ title: String, _ detail: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(WispDesign.font(size: 14, weight: .medium))
            Text(detail).font(WispDesign.font(size: 13)).foregroundStyle(color("text-faint"))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(20).frame(maxWidth: .infinity, minHeight: 100, alignment: .leading)
        .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(color("border")))
    }

    private func errorBanner(_ error: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("项目操作未完成").font(WispDesign.font(size: 13, weight: .semibold))
            Text(error).font(WispDesign.font(size: 12)).textSelection(.enabled)
            if model.lastLoaded != nil { Text("当前显示上次成功读取的项目。实时数据可能已变化。").font(WispDesign.font(size: 12)) }
        }
        .padding(13).frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
    }

    private var footer: some View {
        VStack(spacing: 10) {
            Text("SwiftUI 原生预览 · 项目与实时会话")
                .multilineTextAlignment(.center)
            HStack(spacing: 12) {
                if let loaded = model.lastLoaded {
                    Text("更新于 \(loaded.formatted(date: .omitted, time: .standard))")
                }
                Button { Task { await model.refresh() } } label: { WispIcon(name: "refresh", size: 14) }
                    .buttonStyle(.plain).disabled(model.isLoading).help("刷新 · ⌘R").accessibilityLabel("刷新")
                Button { model.chooseDatabase() } label: { Text(model.databaseURL.lastPathComponent) }
                    .buttonStyle(.plain).disabled(model.isLoading).help("选择数据库 · ⌘O\n" + model.databaseURL.path)
                    .accessibilityLabel("选择数据库")
                Menu {
                    Picker("外观", selection: $appearance) {
                        Text("跟随系统").tag("system")
                        Text("浅色").tag("light")
                        Text("深色").tag("dark")
                    }
                } label: { Text("外观") }
                .menuStyle(.borderlessButton).fixedSize().accessibilityLabel("外观")
            }
            .font(WispDesign.font(size: 11))
        }
        .font(WispDesign.font(size: 12)).foregroundStyle(color("text-faint"))
        .frame(maxWidth: .infinity)
    }
}

private struct ProjectCard: View {
    let project: ProjectSummary
    let selected: Bool
    let busy: Bool
    let saving: Bool
    let toggleStar: () -> Void
    let settings: () -> Void
    let select: () -> Void
    let reveal: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false
    private func color(_ token: String) -> Color { WispDesign.color(token, scheme) }

    var body: some View {
        HStack(spacing: 6) {
            Button(action: select) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(project.name).font(WispDesign.font(size: 14, weight: .semibold)).lineLimit(1)
                        if project.starred { WispIcon(name: "star-filled", size: 13).foregroundStyle(color("clay")) }
                    }
                    Text(project.workspaceDirectory).font(WispDesign.font(size: 11, design: .monospaced))
                        .foregroundStyle(color("text-faint")).lineLimit(1).truncationMode(.head)
                        .help(project.workspaceDirectory)
                    HStack(spacing: 8) {
                        Text("\(project.sessionCount) 会话 · \(project.artifactCount) 产物")
                        if project.needsYouCount > 0 {
                            Text("\(project.needsYouCount) 待查看").foregroundStyle(color("clay-strong"))
                        }
                        Spacer(minLength: 0)
                    }
                    .font(WispDesign.font(size: 12)).foregroundStyle(color("text-faint"))
                }
                .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 16).padding(.leading, 18)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain).accessibilityIdentifier("project-\(project.id)")
            .accessibilityAddTraits(selected ? [.isSelected] : [])
            HStack(spacing: 2) {
                Button(action: toggleStar) {
                    if saving {
                        ProgressView().controlSize(.small).frame(width: 16, height: 16)
                    } else {
                        WispIcon(name: project.starred ? "star-filled" : "star")
                            .foregroundStyle(color(project.starred ? "clay" : "text-muted"))
                    }
                }
                .buttonStyle(WispButtonStyle())
                .disabled(busy)
                .help(project.starred ? "取消收藏" : "收藏项目")
                .accessibilityLabel("\(project.starred ? "取消收藏" : "收藏项目")：\(project.name)")
                Button(action: settings) { WispIcon(name: "gear") }.buttonStyle(WispButtonStyle()).help("项目设置").accessibilityLabel("项目设置")
            }.padding(.trailing, 10)
        }
        .background(color("bg-elev"), in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(color(selected || hovering ? "clay" : "border")))
        .shadow(color: Color.black.opacity(0.035), radius: 2, y: 1)
        .onHover { hovering = $0 }
        .contextMenu { Button("在 Finder 中显示", action: reveal).disabled(!ProjectBrowserModel.workspaceExists(project)) }
    }
}

private func timestamp(_ seconds: Int64) -> String {
    guard seconds > 0 else { return "暂无记录" }
    return Date(timeIntervalSince1970: TimeInterval(seconds)).formatted(date: .abbreviated, time: .shortened)
}

private func shortTimestamp(_ seconds: Int64) -> String {
    guard seconds > 0 else { return "" }
    return Date(timeIntervalSince1970: TimeInterval(seconds)).formatted(.dateTime.month().day())
}
