import AVFoundation
import Foundation

final class DragonAudioPlugin: DragonPlugin {
    let namespace = "audio"
    private let player = AVPlayer()
    private var items: [[String: Any]] = []
    private var index = -1
    private var repeatMode = "off"
    private var speed: Float = 1.0
    private var wantsPlay = false
    private var endObserver: NSObjectProtocol?

    init(context: PluginContext) {
        try? AVAudioSession.sharedInstance().setCategory(.playback)
        try? AVAudioSession.sharedInstance().setActive(true)
    }

    func handles(_ method: String) -> Bool { method.hasPrefix("audio.") }

    func handle(_ method: String, args a: [String: Any], reply: @escaping (Any?) -> Void) -> Bool {
        DispatchQueue.main.async {
            switch method {
            case "audio.setQueue":
                self.items = a["items"] as? [[String: Any]] ?? []
                self.load(a["index"] as? Int ?? 0, play: a["play"] as? Bool ?? true)
                reply(true)
            case "audio.play": self.player.rate = self.speed; self.wantsPlay = true; reply(true)
            case "audio.pause": self.player.pause(); self.wantsPlay = false; reply(true)
            case "audio.toggle":
                if self.player.rate > 0 { self.player.pause(); self.wantsPlay = false } else { self.player.rate = self.speed; self.wantsPlay = true }
                reply(true)
            case "audio.next": self.load(self.index + 1, play: true); reply(true)
            case "audio.previous":
                if CMTimeGetSeconds(self.player.currentTime()) > 3 { self.player.seek(to: .zero) } else { self.load(max(0, self.index - 1), play: true) }
                reply(true)
            case "audio.seek":
                let ms = a["ms"] as? Int ?? 0
                self.player.seek(to: CMTime(value: CMTimeValue(ms), timescale: 1000)); reply(true)
            case "audio.skipTo": self.load(a["index"] as? Int ?? 0, play: true); reply(true)
            case "audio.repeat": self.repeatMode = a["mode"] as? String ?? "off"; reply(true)
            case "audio.speed":
                self.speed = Float(a["value"] as? Double ?? 1.0)
                if self.player.rate > 0 { self.player.rate = self.speed }
                reply(true)
            case "audio.stop": self.player.pause(); self.player.replaceCurrentItem(with: nil); self.items = []; self.index = -1; self.wantsPlay = false; reply(true)
            case "audio.queue.add":
                if let it = a["item"] as? [String: Any] { self.items.append(it) }
                reply(true)
            case "audio.queue.remove":
                let i = a["index"] as? Int ?? -1
                if i >= 0 && i < self.items.count && i != self.index { self.items.remove(at: i); if i < self.index { self.index -= 1 } }
                reply(true)
            case "audio.state": reply(self.state())
            default: reply(nil)
            }
        }
        return true
    }

    private func load(_ i: Int, play: Bool) {
        if items.isEmpty { return }
        var n = i
        if n >= items.count { if repeatMode == "all" { n = 0 } else { player.pause(); wantsPlay = false; return } }
        n = max(0, n)
        index = n
        guard let s = items[n]["uri"] as? String, let url = (s.contains("://") ? URL(string: s) : URL(fileURLWithPath: s)) else { return }
        let item = AVPlayerItem(url: url)
        player.replaceCurrentItem(with: item)
        if let o = endObserver { NotificationCenter.default.removeObserver(o) }
        endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
            guard let self = self else { return }
            if self.repeatMode == "one" { self.load(self.index, play: true) } else { self.load(self.index + 1, play: true) }
        }
        wantsPlay = play
        if play { player.rate = speed }
    }

    private func state() -> [String: Any] {
        let cur = player.currentItem
        let dur = cur.map { CMTimeGetSeconds($0.duration) } ?? 0
        let pos = CMTimeGetSeconds(player.currentTime())
        return [
            "active": !items.isEmpty, "playing": player.rate > 0, "buffering": false, "ended": false,
            "positionMs": pos.isFinite ? Int(pos * 1000) : 0, "durationMs": dur.isFinite ? Int(dur * 1000) : 0,
            "index": items.isEmpty ? -1 : index, "count": items.count,
            "mediaId": index >= 0 && index < items.count ? "\(items[index]["id"] ?? "")" : "",
            "nextIndex": index + 1 < items.count ? index + 1 : -1,
            "shuffle": false, "repeat": repeatMode, "speed": Double(speed), "sleepLeftMs": 0,
        ]
    }
}
