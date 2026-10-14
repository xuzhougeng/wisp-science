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
}

final class WispSceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var workspace: RemoteWorkspace?
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        #if DEBUG
        let model = PreviewHost.workspace() ?? RemoteWorkspace()
        #else
        let model = RemoteWorkspace()
        #endif
        workspace = model
        let window = WispWindow(windowScene: scene)
        window.dismissTopSurface = { [weak model] in model?.dismissTopSurface() }
        window.hasSurface = { [weak model] in model?.hasDismissibleSurface == true }
        window.installPageBackGesture(workspace: model)
        window.rootViewController = UIHostingController(rootView: WorkspaceView(model: model, connection: model.connection))
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
    private weak var pageBackWorkspace: RemoteWorkspace?
    private lazy var pageBackGesture: UIScreenEdgePanGestureRecognizer = {
        let gesture = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(handlePageBack(_:)))
        gesture.edges = .left
        gesture.delegate = self
        return gesture
    }()

    func installPageBackGesture(workspace: RemoteWorkspace) {
        pageBackWorkspace = workspace
        addGestureRecognizer(pageBackGesture)
    }

    private var canStartPageBack: Bool {
        // Also respect UIKit presentations, such as a date-picker popover,
        // which are not represented by the workspace's sheet state.
        pageBackWorkspace?.canSwipeBackToWorkspace == true && rootViewController?.presentedViewController == nil
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
        guard let workspace = pageBackWorkspace else { return }
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
