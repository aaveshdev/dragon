import SwiftUI

struct PluginContext {
    let docsDir: URL
}

protocol DragonPlugin: AnyObject {
    var namespace: String { get }
    func handles(_ method: String) -> Bool
    func handle(_ method: String, args: [String: Any], reply: @escaping (Any?) -> Void) -> Bool
    func makeView(kind: String, id: String, params: [String: Any]) -> AnyView?
    func releaseView(kind: String, id: String)
}

extension DragonPlugin {
    func handles(_ method: String) -> Bool { method.hasPrefix(namespace + ".") }
    func handle(_ method: String, args: [String: Any], reply: @escaping (Any?) -> Void) -> Bool { false }
    func makeView(kind: String, id: String, params: [String: Any]) -> AnyView? { nil }
    func releaseView(kind: String, id: String) {}
}
