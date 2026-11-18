import Foundation

enum DragonNative {
    private static func s(_ p: UnsafePointer<CChar>?) -> String { p == nil ? "" : String(cString: p!) }

    static func start(docsDir: String, media: String) -> String { s(dgn_app_init(docsDir, media)) }
    static func render() -> String { s(dgn_app_render()) }
    @discardableResult static func event(_ e: String) -> Bool { dgn_app_event(e) != 0 }
    static func isDirty() -> Bool { dgn_app_is_dirty() != 0 }
    static func tick(_ nowMs: Int64) -> Int64 { dgn_app_tick(nowMs) }
    static func platformCalls() -> String { s(dgn_app_platform_calls()) }
    @discardableResult static func platformResult(_ id: String, _ json: String) -> Bool { dgn_app_platform_result(id, json) != 0 }
    static func back() -> Bool { dgn_app_back() != 0 }
    @discardableResult static func setMedia(_ m: String) -> Bool { dgn_app_set_media(m) != 0 }
    @discardableResult static func lifecycle(_ state: String) -> Bool { dgn_app_lifecycle(state) != 0 }
    static func lastError() -> String { s(dgn_app_last_error()) }
}
