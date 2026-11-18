import SwiftUI
import WebKit

final class DragonWebTab: NSObject, WKNavigationDelegate {
    let id: String
    let isPrivate: Bool
    var desktop = false
    let web: WKWebView
    var loadingError: String? = nil
    var findCount = 0
    private var obs: [NSKeyValueObservation] = []

    init(id: String, isPrivate: Bool, desktop: Bool) {
        self.id = id; self.isPrivate = isPrivate; self.desktop = desktop
        let cfg = WKWebViewConfiguration()
        if isPrivate { cfg.websiteDataStore = .nonPersistent() }
        cfg.allowsInlineMediaPlayback = true
        web = WKWebView(frame: .zero, configuration: cfg)
        super.init()
        web.navigationDelegate = self
        web.allowsBackForwardNavigationGestures = true
        applyDesktop()
    }

    func applyDesktop() {
        web.customUserAgent = desktop
            ? "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
            : nil
    }

    func navigate(_ raw: String) {
        var s = raw.trimmingCharacters(in: .whitespaces)
        if s.isEmpty { return }
        if !s.contains("://") && !s.hasPrefix("about:") && !s.hasPrefix("data:") { s = "https://" + s }
        if let u = URL(string: s) { loadingError = nil; web.load(URLRequest(url: u)) }
    }

    func webView(_ w: WKWebView, didFail n: WKNavigation!, withError e: Error) { loadingError = e.localizedDescription }
    func webView(_ w: WKWebView, didFailProvisionalNavigation n: WKNavigation!, withError e: Error) { loadingError = e.localizedDescription }

    var state: [String: Any] {
        let url = web.url?.absoluteString ?? ""
        return [
            "url": url, "title": web.title ?? "", "progress": Int(web.estimatedProgress * 100), "loading": web.isLoading,
            "canBack": web.canGoBack, "canForward": web.canGoForward, "secure": web.hasOnlySecureContent && url.hasPrefix("https"),
            "counts": [String: Int](), "favicon": NSNull(), "private": isPrivate, "desktop": desktop,
            "findCount": findCount, "findIdx": 0, "error": loadingError.map { $0 as Any } ?? NSNull(),
        ]
    }
}

final class DragonWebPlugin: DragonPlugin {
    let namespace = "web"
    private var tabs: [String: DragonWebTab] = [:]
    init(context: PluginContext) {}

    func handle(_ method: String, args a: [String: Any], reply: @escaping (Any?) -> Void) -> Bool {
        let id = a["tab"] as? String ?? ""
        let run: () -> Void = {
            switch method {
            case "web.config": reply(true)
            case "web.state":
                var ts: [String: Any] = [:]
                for (k, t) in self.tabs { ts[k] = t.state }
                reply(["tabs": ts, "totals": ["upgrades": 0], "privateIsolated": false, "events": [Any]()])
            case "web.open":
                let t = self.tabs[id] ?? self.make(id, a["private"] as? Bool ?? false, a["desktop"] as? Bool ?? false)
                if let u = a["url"] as? String, !u.isEmpty { t.navigate(u) }
                reply(true)
            case "web.load": self.tabs[id]?.navigate(a["url"] as? String ?? ""); reply(true)
            case "web.back": self.tabs[id]?.web.goBack(); reply(true)
            case "web.forward": self.tabs[id]?.web.goForward(); reply(true)
            case "web.reload": self.tabs[id]?.web.reload(); reply(true)
            case "web.stop": self.tabs[id]?.web.stopLoading(); reply(true)
            case "web.close": self.tabs[id]?.web.stopLoading(); self.tabs[id] = nil; reply(true)
            case "web.desktop":
                if let t = self.tabs[id] { t.desktop = a["on"] as? Bool ?? false; t.applyDesktop(); t.web.reload() }
                reply(true)
            case "web.eval":
                guard let t = self.tabs[id] else { reply(nil); return }
                t.web.evaluateJavaScript(a["code"] as? String ?? "") { v, _ in reply(["value": v.map { "\($0)" } ?? "null"]) }
            case "web.find", "web.findNext", "web.findClear": reply(true)
            case "web.clear":
                WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) { reply(true) }
            default: reply(nil)
            }
        }
        if Thread.isMainThread { run() } else { DispatchQueue.main.async(execute: run) }
        return true
    }

    private func make(_ id: String, _ priv: Bool, _ desktop: Bool) -> DragonWebTab {
        let t = DragonWebTab(id: id, isPrivate: priv, desktop: desktop)
        tabs[id] = t
        return t
    }

    func makeView(kind: String, id: String, params: [String: Any]) -> AnyView? {
        guard kind == "web" else { return nil }
        let tabId = params["tab"] as? String ?? id
        let tab = tabs[tabId] ?? make(tabId, false, false)
        return AnyView(WebTabView(web: tab.web))
    }
}

struct WebTabView: UIViewRepresentable {
    let web: WKWebView
    func makeUIView(context: Context) -> WKWebView { web }
    func updateUIView(_ v: WKWebView, context: Context) {}
}
