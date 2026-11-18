import SwiftUI

@main
struct DragonIOSApp: App {
    @StateObject private var host = DragonHost()
    init() { DragonFonts.register() }
    var body: some Scene {
        WindowGroup { DragonRoot(host: host) }
    }
}

struct DragonRoot: View {
    @ObservedObject var host: DragonHost
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        GeometryReader { geo in
            let media = mediaDict(geo)
            ZStack {
                if let err = host.fatalError {
                    FatalErrorView(message: err)
                } else if let f = host.frame {
                    DragonFrameView(host: host, frame: f, topInset: Double(geo.safeAreaInsets.top), bottomInset: Double(geo.safeAreaInsets.bottom))
                } else {
                    Color.clear
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.background(host.frame?["theme"] as? [String: Any], scheme))
            .ignoresSafeArea(.container)
            .onAppear { host.start(media: media) }
            .onChange(of: DragonHost.json(media)) { _ in host.updateMedia(media) }
            .gesture(
                DragGesture(minimumDistance: 20).onEnded { v in
                    if v.startLocation.x < 24 && v.translation.width > 80 && host.canHandleBack { _ = host.back() }
                }
            )
        }
        .preferredColorScheme(Theme.forcedScheme(host.frame?["theme"] as? [String: Any]))
    }

    private func mediaDict(_ geo: GeometryProxy) -> [String: Any] {
        [
            "width": Double(geo.size.width), "height": Double(geo.size.height),
            "pixelRatio": Double(UIScreen.main.scale),
            "top": Double(geo.safeAreaInsets.top), "bottom": Double(geo.safeAreaInsets.bottom),
            "left": Double(geo.safeAreaInsets.leading), "right": Double(geo.safeAreaInsets.trailing),
            "keyboard": 0.0,
            "brightness": scheme == .dark ? "dark" : "light",
            "textScale": 1.0,
            "locale": Locale.current.identifier,
        ]
    }
}

struct FatalErrorView: View {
    let message: String
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Dragon error").font(.title2.bold()).foregroundColor(Color(red: 1, green: 0.54, blue: 0.5))
                Text(message).font(.system(size: 13)).foregroundColor(.white)
            }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
        }.background(Color(red: 0.1, green: 0.1, blue: 0.12))
    }
}
