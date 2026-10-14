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
        window.hasSurface = { [weak model] in model?.sheet != nil }
        window.rootViewController = UIHostingController(rootView: WorkspaceView(model: model, connection: model.connection))
        self.window = window
        window.makeKeyAndVisible()
    }
    func sceneDidBecomeActive(_ scene: UIScene) { workspace?.setActive(true) }
    func sceneDidEnterBackground(_ scene: UIScene) { workspace?.setActive(false) }
    func sceneDidDisconnect(_ scene: UIScene) { workspace?.setActive(false) }
}

/// A window-level Escape command works immediately after opening a sheet,
/// including when focus remains in the conversation's composer underneath.
final class WispWindow: UIWindow {
    var dismissTopSurface: (() -> Void)?
    var hasSurface: (() -> Bool)?
    override var keyCommands: [UIKeyCommand]? {
        guard hasSurface?() == true else { return super.keyCommands }
        return [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(closeSurface))]
    }
    @objc private func closeSurface() { dismissTopSurface?() }
}
