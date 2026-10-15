import UIKit
import SwiftUI
import WispRemote

@main final class WispAppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: "Wisp", sessionRole: session.role)
        configuration.delegateClass = WispSceneDelegate.self
        return configuration
    }

    /// Hardware keyboard shortcuts, listed in the iPad ⌘ overlay and menu bar.
    /// WispWindow performs and validates them for the current workspace.
    override func buildMenu(with builder: UIMenuBuilder) {
        super.buildMenu(with: builder)
        guard builder.system == .main else { return }
        let sections = WorkspaceTab.allCases.enumerated().map { index, tab in
            UIKeyCommand(title: RemoteWorkspace.sectionTitle(tab, paired: true), action: #selector(WispWindow.showWorkspaceSection(_:)),
                         input: "\(index + 1)", modifierFlags: .command, propertyList: tab.rawValue)
        }
        let refresh = UIKeyCommand(title: "刷新", action: #selector(WispWindow.refreshWorkspace(_:)), input: "r", modifierFlags: .command)
        let workspace = UIMenu(title: "", identifier: UIMenu.Identifier("science.wisp.workspace"), options: .displayInline,
                               children: sections + [refresh])
        let create = UIMenu(title: "", identifier: UIMenu.Identifier("science.wisp.conversation"), options: .displayInline, children: [
            UIKeyCommand(title: "新对话", action: #selector(WispWindow.newConversation(_:)), input: "n", modifierFlags: .command),
        ])
        if builder.menu(for: .view) != nil { builder.insertChild(workspace, atStartOfMenu: .view) }
        if builder.menu(for: .file) != nil { builder.insertChild(create, atStartOfMenu: .file) }
    }
}

final class WispSceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var workspace: RemoteWorkspace?
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        #if DEBUG
        let model = PreviewHost.workspace() ?? RemoteWorkspace()
        let preferences = PreviewHost.preferences() ?? .standard
        #else
        let model = RemoteWorkspace()
        let preferences = UserDefaults.standard
        #endif
        workspace = model
        let window = WispWindow(windowScene: scene)
        window.dismissTopSurface = { [weak model] in model?.dismissTopSurface() }
        window.hasSurface = { [weak model] in model?.hasDismissibleSurface == true }
        window.installPageBackGesture(workspace: model)
        window.rootViewController = UIHostingController(rootView: WorkspaceView(model: model, connection: model.connection)
            .defaultAppStorage(preferences))
        self.window = window
        window.makeKeyAndVisible()
    }
    func sceneDidBecomeActive(_ scene: UIScene) { workspace?.setActive(true) }
    func sceneWillResignActive(_ scene: UIScene) { workspace?.cancelPageBackGesture() }
    func sceneDidEnterBackground(_ scene: UIScene) { workspace?.setActive(false) }
    func sceneDidDisconnect(_ scene: UIScene) { workspace?.setActive(false) }
}

/// A window-level Escape command works immediately after opening a page or sheet,
/// including when focus remains in the conversation's composer underneath.
final class WispWindow: UIWindow, UIGestureRecognizerDelegate {
    var dismissTopSurface: (() -> Void)?
    var hasSurface: (() -> Bool)?
    private weak var workspace: RemoteWorkspace?
    private lazy var pageBackGesture: UIScreenEdgePanGestureRecognizer = {
        let gesture = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(handlePageBack(_:)))
        gesture.edges = .left
        gesture.delegate = self
        return gesture
    }()

    func installPageBackGesture(workspace: RemoteWorkspace) {
        self.workspace = workspace
        addGestureRecognizer(pageBackGesture)
    }

    // MARK: Keyboard and menu commands

    private func section(_ sender: Any?) -> WorkspaceTab? {
        ((sender as? UICommand)?.propertyList as? String).flatMap(WorkspaceTab.init(rawValue:))
    }
    @objc func showWorkspaceSection(_ sender: UIKeyCommand) {
        if let tab = section(sender) { workspace?.showSection(tab) }
    }
    @objc func newConversation(_ sender: UIKeyCommand) {
        guard let workspace, workspace.canCreateConversation else { return }
        Task { await workspace.createConversation() }
    }
    @objc func refreshWorkspace(_ sender: UIKeyCommand) {
        guard let workspace else { return }
        Task { await workspace.refreshVisible() }
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        guard let workspace else { return super.canPerformAction(action, withSender: sender) }
        switch action {
        case #selector(showWorkspaceSection(_:)): return section(sender).map(workspace.canShowSection) ?? false
        case #selector(newConversation(_:)): return workspace.canCreateConversation
        case #selector(refreshWorkspace(_:)): return workspace.canRefresh
        default: return super.canPerformAction(action, withSender: sender)
        }
    }

    override func validate(_ command: UICommand) {
        super.validate(command)
        guard command.action == #selector(showWorkspaceSection(_:)), let tab = section(command),
              let workspace else { return }
        command.title = workspace.sectionTitle(tab)
        command.state = workspace.tab == tab ? .on : .off
    }

    private var canStartPageBack: Bool {
        // Also respect UIKit presentations, such as a date-picker popover,
        // which are not represented by the workspace's sheet state.
        workspace?.canSwipeBackToWorkspace == true && rootViewController?.presentedViewController == nil
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        // Decide at touch-down too: finishing a native detail pop must not turn
        // the same finger movement into a second pop of the workspace page.
        canStartPageBack
    }

    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard gestureRecognizer === pageBackGesture else { return super.gestureRecognizerShouldBegin(gestureRecognizer) }
        let velocity = pageBackGesture.velocity(in: self)
        guard velocity.x > abs(velocity.y) else { return false }
        return canStartPageBack
    }

    @objc private func handlePageBack(_ gesture: UIScreenEdgePanGestureRecognizer) {
        guard let workspace else { return }
        let translation = gesture.translation(in: self).x
        switch gesture.state {
        case .began:
            guard workspace.beginPageBackGesture() else { return }
            workspace.updatePageBackGesture(translation: translation, width: bounds.width)
        case .changed:
            workspace.updatePageBackGesture(translation: translation, width: bounds.width)
        case .ended, .cancelled, .failed:
            withAnimation(UIAccessibility.isReduceMotionEnabled ? nil : .easeOut(duration: 0.25)) {
                workspace.finishPageBackGesture(translation: translation, velocity: gesture.velocity(in: self).x,
                                                width: bounds.width, cancelled: gesture.state != .ended || !canStartPageBack)
            }
        default: break
        }
    }

    override var keyCommands: [UIKeyCommand]? {
        guard hasSurface?() == true else { return super.keyCommands }
        return [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(closeSurface))]
    }
    @objc private func closeSurface() { dismissTopSurface?() }
}
