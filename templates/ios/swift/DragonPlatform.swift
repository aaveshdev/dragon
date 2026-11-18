import SwiftUI
import UIKit
import UserNotifications

final class DragonPlatform: NSObject {
    private let reply: (String, Any?) -> Void
    let plugins: [DragonPlugin]

    init(docsDir: URL, reply: @escaping (String, Any?) -> Void) {
        self.reply = reply
        self.plugins = DragonPlugins.create(PluginContext(docsDir: docsDir))
        super.init()
    }

    func handle(_ call: [String: Any]) {
        let id = call["id"] as? String ?? ""
        let method = call["method"] as? String ?? ""
        let args = call["args"] as? [String: Any] ?? [:]
        switch method {
        case "http": http(id, args)
        case "clipboard.set": UIPasteboard.general.string = args["text"] as? String ?? ""
        case "clipboard.get": reply(id, ["text": UIPasteboard.general.string ?? ""])
        case "openUrl":
            if let u = URL(string: args["url"] as? String ?? "") {
                UIApplication.shared.open(u) { ok in self.reply(id, ok) }
            } else { reply(id, false) }
        case "share":
            let items: [Any] = [args["text"] as? String ?? ""]
            let vc = UIActivityViewController(activityItems: items, applicationActivities: nil)
            DragonPlatform.topController()?.present(vc, animated: true)
        case "haptic": UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        case "vibrate": UINotificationFeedbackGenerator().notificationOccurred(.success)
        case "deviceInfo":
            let d = UIDevice.current
            reply(id, ["platform": "ios", "model": d.model, "manufacturer": "Apple", "release": d.systemVersion, "name": d.name])
        case "toast": toast(args["message"] as? String ?? "")
        case "notify": notify(args)
        case "notify.cancel": UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
        case "permission.check": permissionCheck(id, args["name"] as? String ?? "")
        case "permission.request": permissionRequest(id, args["name"] as? String ?? "")
        case "statusBar", "orientation": break
        case "exit": break
        case "datePicker", "timePicker", "pickFile", "takePhoto", "location":
            reply(id, ["error": "\(method) is not implemented in the iOS skeleton yet"])
        default:
            let handled = plugins.contains { $0.handles(method) && $0.handle(method, args: args, reply: { r in self.reply(id, r) }) }
            if !handled { reply(id, nil) }
        }
    }

    private func http(_ id: String, _ a: [String: Any]) {
        guard let s = a["url"] as? String,
              let url = URL(string: s) ?? URL(string: s.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed.union(.urlPathAllowed).union(CharacterSet(charactersIn: "%:/?#[]@!$&'()*+,;=-._~"))) ?? s)
        else { reply(id, ["status": 0, "error": "bad url"]); return }
        var req = URLRequest(url: url)
        req.httpMethod = a["method"] as? String ?? "GET"
        req.timeoutInterval = Double(a["timeout"] as? Int ?? 30000) / 1000.0
        if let hs = a["headers"] as? [String: Any] { for (k, v) in hs { req.setValue("\(v)", forHTTPHeaderField: k) } }
        if let parts = a["bodyParts"] as? [[String: Any]] {
            var d = Data()
            for p in parts {
                if let t = p["text"] as? String { d.append(t.data(using: .utf8) ?? Data()) }
                else if let b = p["base64"] as? String { d.append(Data(base64Encoded: b, options: .ignoreUnknownCharacters) ?? Data()) }
            }
            req.httpBody = d
        } else if let body = a["body"] as? String { req.httpBody = body.data(using: .utf8) }
        let insecure = a["insecure"] as? Bool ?? false
        let session = insecure ? URLSession(configuration: .default, delegate: InsecureSessionDelegate(), delegateQueue: nil) : URLSession.shared
        session.dataTask(with: req) { data, resp, err in
            var out: [String: Any] = [:]
            if let r = resp as? HTTPURLResponse {
                out["status"] = r.statusCode
                out["body"] = String(data: data ?? Data(), encoding: .utf8) ?? ""
                var h: [String: String] = [:]
                for (k, v) in r.allHeaderFields { h["\(k)"] = "\(v)" }
                out["headers"] = h
            } else {
                out["status"] = 0
                out["error"] = err?.localizedDescription ?? "request failed"
            }
            DispatchQueue.main.async { self.reply(id, out) }
        }.resume()
    }

    private func toast(_ message: String) {
        guard let win = DragonPlatform.keyWindow() else { return }
        let l = UILabel()
        l.text = "  \(message)  "
        l.textColor = .white
        l.backgroundColor = UIColor.black.withAlphaComponent(0.8)
        l.layer.cornerRadius = 16; l.clipsToBounds = true
        l.numberOfLines = 0; l.textAlignment = .center
        l.frame = CGRect(x: 24, y: win.bounds.height - win.safeAreaInsets.bottom - 110, width: win.bounds.width - 48, height: 40)
        win.addSubview(l)
        UIView.animate(withDuration: 0.3, delay: 2.0, options: [], animations: { l.alpha = 0 }) { _ in l.removeFromSuperview() }
    }

    private func notify(_ a: [String: Any]) {
        let c = UNMutableNotificationContent()
        c.title = a["title"] as? String ?? ""
        c.body = a["body"] as? String ?? ""
        let req = UNNotificationRequest(identifier: "dragon_\(a["id"] ?? 1)", content: c, trigger: nil)
        UNUserNotificationCenter.current().add(req)
    }

    private func permissionCheck(_ id: String, _ name: String) {
        if name == "notifications" {
            UNUserNotificationCenter.current().getNotificationSettings { s in
                DispatchQueue.main.async { self.reply(id, ["granted": s.authorizationStatus == .authorized]) }
            }
        } else { reply(id, ["granted": false]) }
    }

    private func permissionRequest(_ id: String, _ name: String) {
        if name == "notifications" {
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { ok, _ in
                DispatchQueue.main.async { self.reply(id, ["granted": ok]) }
            }
        } else { reply(id, ["granted": false]) }
    }

    static func keyWindow() -> UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }.first { $0.isKeyWindow }
    }
    static func topController() -> UIViewController? {
        var vc = keyWindow()?.rootViewController
        while let p = vc?.presentedViewController { vc = p }
        return vc
    }
}


final class InsecureSessionDelegate: NSObject, URLSessionDelegate {
    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        if challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust, let trust = challenge.protectionSpace.serverTrust {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            completionHandler(.performDefaultHandling, nil)
        }
    }
}
