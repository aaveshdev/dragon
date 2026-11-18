import SwiftUI
import UIKit

final class DragonHost: ObservableObject {
    @Published var frame: [String: Any]? = nil
    @Published var fatalError: String? = nil
    @Published var drawers: [String: Bool] = [:]
    @Published var scrollCommands: [Int: [String: Any]] = [:]

    private var started = false
    private var mediaJson = ""
    private var timer: Timer?
    private var platform: DragonPlatform!
    var plugins: [DragonPlugin] { platform.plugins }

    func start(media: [String: Any]) {
        if started { return }
        started = true
        mediaJson = DragonHost.json(media)
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("dragon_docs")
        try? FileManager.default.createDirectory(at: docs, withIntermediateDirectories: true)
        platform = DragonPlatform(docsDir: docs) { [weak self] id, value in
            DispatchQueue.main.async {
                guard let self = self else { return }
                let js: String
                if value == nil { js = "" }
                else if let b = value as? Bool { js = b ? "true" : "false" }
                else if let s = value as? String { js = DragonHost.json(s) }
                else { js = DragonHost.json(value!) }
                DragonNative.platformResult(id, js)
                self.refresh()
                self.drainPlatform()
            }
        }
        let err = DragonNative.start(docsDir: docs.path, media: mediaJson)
        if !err.isEmpty { fatalError = err; return }
        NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            DragonNative.lifecycle("resumed"); self?.refresh()
        }
        NotificationCenter.default.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { _ in
            DragonNative.lifecycle("inactive")
        }
        NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            DragonNative.lifecycle("paused")
        }
        refresh()
        drainPlatform()
        scheduleTick(after: 0.016)
    }

    private func scheduleTick(after seconds: TimeInterval) {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
            guard let self = self, self.fatalError == nil else { return }
            let next = DragonNative.tick(Int64(Date().timeIntervalSince1970 * 1000))
            if DragonNative.isDirty() { self.refresh() }
            self.drainPlatform()
            let ms: Double = (next >= 0 && next < 100) ? Double(max(next, 8)) : 100
            self.scheduleTick(after: ms / 1000.0)
        }
    }

    func updateMedia(_ media: [String: Any]) {
        let s = DragonHost.json(media)
        if s == mediaJson || !started || fatalError != nil { return }
        mediaJson = s
        DragonNative.setMedia(s)
        refresh()
    }

    func dispatch(_ evt: String?) {
        guard let evt = evt, !evt.isEmpty else { return }
        DragonNative.event(evt)
        refresh()
        drainPlatform()
    }

    func back() -> Bool {
        let open = drawers.filter { $0.value }.keys
        if !open.isEmpty { for k in open { drawers[k] = false }; return true }
        let handled = DragonNative.back()
        refresh()
        return handled
    }

    var canHandleBack: Bool {
        guard let f = frame else { return false }
        let ov = f["overlays"] as? [Any] ?? []
        return drawers.values.contains(true) || (f["canPop"] as? Bool ?? false) || (f["hasBack"] as? Bool ?? false) || !ov.isEmpty
    }

    private func refresh() {
        guard let data = DragonNative.render().data(using: .utf8),
              let f = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            fatalError = "Bad frame from the Dragon runtime"
            return
        }
        frame = f
        for c in (f["commands"] as? [[String: Any]]) ?? [] { handleCommand(c) }
    }

    private func handleCommand(_ c: [String: Any]) {
        switch c["cmd"] as? String ?? "" {
        case "drawer": drawers[c["side"] as? String ?? "start"] = (c["op"] as? String) == "open"
        case "scroll": scrollCommands[c["id"] as? Int ?? -1] = c
        default: break
        }
    }

    private func drainPlatform() {
        let txt = DragonNative.platformCalls()
        if txt.count <= 2 { return }
        guard let data = txt.data(using: .utf8),
              let arr = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else { return }
        for call in arr { platform.handle(call) }
    }

    static func json(_ v: Any) -> String {
        if let s = v as? String {
            if let d = try? JSONSerialization.data(withJSONObject: [s]), let t = String(data: d, encoding: .utf8) {
                return String(t.dropFirst().dropLast())
            }
            return "\"\""
        }
        if JSONSerialization.isValidJSONObject(v), let d = try? JSONSerialization.data(withJSONObject: v), let t = String(data: d, encoding: .utf8) { return t }
        return "null"
    }
}
