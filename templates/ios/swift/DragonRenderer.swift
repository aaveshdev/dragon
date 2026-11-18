import SwiftUI
import UIKit

typealias J = [String: Any]
private let INF: CGFloat = 1.0e9

extension Dictionary where Key == String, Value == Any {
    func str(_ k: String) -> String? { self[k] as? String }
    func num(_ k: String) -> CGFloat? {
        if let d = self[k] as? Double { return CGFloat(d) }
        if let i = self[k] as? Int { return CGFloat(i) }
        if let n = self[k] as? NSNumber { return CGFloat(truncating: n) }
        return nil
    }
    func int(_ k: String) -> Int? { num(k).map { Int($0) } }
    func bool(_ k: String) -> Bool? { self[k] as? Bool }
    func obj(_ k: String) -> J? { self[k] as? J }
    func arr(_ k: String) -> [J]? { self[k] as? [J] }
}

private func isFill(_ v: CGFloat?) -> Bool { (v ?? 0) >= INF }

enum Theme {
    static func color(_ s: String?) -> Color? {
        guard var h = s, h.hasPrefix("#") else { return nil }
        h.removeFirst()
        guard let v = UInt64(h, radix: 16) else { return nil }
        if h.count == 8 {
            return Color(.sRGB, red: Double((v >> 16) & 0xFF) / 255, green: Double((v >> 8) & 0xFF) / 255, blue: Double(v & 0xFF) / 255, opacity: Double((v >> 24) & 0xFF) / 255)
        }
        if h.count == 6 {
            return Color(.sRGB, red: Double((v >> 16) & 0xFF) / 255, green: Double((v >> 8) & 0xFF) / 255, blue: Double(v & 0xFF) / 255, opacity: 1)
        }
        return nil
    }
    static func scheme(_ theme: J?) -> J? { theme?.obj("scheme") }
    static func background(_ theme: J?, _ system: ColorScheme) -> Color {
        color(theme?.str("background")) ?? color(scheme(theme)?.str("background")) ?? color(scheme(theme)?.str("surface"))
            ?? (system == .dark ? Color(red: 0.08, green: 0.08, blue: 0.09) : Color(red: 1, green: 0.98, blue: 0.98))
    }
    static func forcedScheme(_ theme: J?) -> ColorScheme? {
        guard let d = theme?.bool("dark") else { return nil }
        return d ? .dark : .light
    }
    static func primary(_ theme: J?) -> Color { color(scheme(theme)?.str("primary")) ?? .accentColor }
}

private struct ThemeKey: EnvironmentKey { static let defaultValue: J? = nil }
extension EnvironmentValues { var dragonTheme: J? { get { self[ThemeKey.self] } set { self[ThemeKey.self] = newValue } } }

struct Ctx {
    var host: DragonHost
    var topInset: CGFloat
    var bottomInset: CGFloat
    var inColumn = false
    var inRow = false
    var closeSide: String? = nil
    func fire(_ id: String?) {
        guard let id = id else { return }
        if let s = closeSide { host.drawers[s] = false }
        host.dispatch(id)
    }
    func fire(_ id: String?, _ payload: Any) {
        guard let id = id else { return }
        host.dispatch("\(id):\(payload)")
    }
    func col() -> Ctx { var c = self; c.inColumn = true; c.inRow = false; return c }
    func row() -> Ctx { var c = self; c.inRow = true; c.inColumn = false; return c }
    func other() -> Ctx { var c = self; c.inRow = false; c.inColumn = false; return c }
}

struct DragonFrameView: View {
    @ObservedObject var host: DragonHost
    let frame: J
    let topInset: Double
    let bottomInset: Double
    var body: some View {
        let ctx = Ctx(host: host, topInset: CGFloat(topInset), bottomInset: CGFloat(bottomInset))
        ZStack {
            if let root = frame.obj("root") {
                let content = root.obj("content") ?? root
                Node(n: content, c: ctx)
                    .id(root.str("key") ?? "root")
                    .transition(.move(edge: .trailing))
            }
            ForEach(Array((frame.arr("overlays") ?? []).enumerated()), id: \.offset) { _, o in
                OverlayView(o: o, c: ctx)
            }
            if let err = frame.str("error") {
                VStack { Spacer(); Text(err).font(.system(size: 11)).foregroundColor(.white).padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading).background(Color(red: 0.72, green: 0.11, blue: 0.11).opacity(0.9)) }
            }
        }
        .environment(\.dragonTheme, frame.obj("theme"))
        .animation(.easeInOut(duration: 0.25), value: frame.obj("root")?.str("key"))
    }
}

struct OverlayView: View {
    let o: J
    let c: Ctx
    var body: some View {
        let kind = o.str("kind") ?? "dialog"
        let content = o.obj("content")
        let dismissible = o.bool("dismissible") ?? true
        let scrim = Theme.color(o.str("barrierColor")) ?? Color.black.opacity(0.4)
        if kind == "snackbar" {
            VStack { Spacer(); Node(n: content, c: c.other()).padding(.bottom, 80 + c.bottomInset) }
        } else {
            ZStack(alignment: kind == "dialog" ? .center : .bottom) {
                scrim.contentShape(Rectangle()).onTapGesture { if dismissible { c.fire(o.str("onDismiss")) } }
                if kind == "dialog" {
                    Node(n: content, c: c.other()).padding(24).onTapGesture {}
                } else {
                    VStack(spacing: 0) {
                        if o.bool("dragHandle") == true {
                            Capsule().fill(Color.secondary.opacity(0.4)).frame(width: 32, height: 4).padding(.vertical, 12)
                        }
                        Node(n: content, c: c.col())
                    }
                    .frame(maxWidth: .infinity)
                    .background(Theme.color(o.str("bg")) ?? Color(UIColor.systemBackground))
                    .clipShape(RoundedCorners(tl: 28, tr: 28, bl: 0, br: 0))
                    .padding(.bottom, c.bottomInset)
                    .onTapGesture {}
                }
            }
        }
    }
}

struct RoundedCorners: Shape {
    var tl: CGFloat, tr: CGFloat, bl: CGFloat, br: CGFloat
    func path(in r: CGRect) -> Path {
        var p = Path()
        let tl = min(self.tl, min(r.width, r.height) / 2), tr = min(self.tr, min(r.width, r.height) / 2)
        let bl = min(self.bl, min(r.width, r.height) / 2), br = min(self.br, min(r.width, r.height) / 2)
        p.move(to: CGPoint(x: r.minX + tl, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX - tr, y: r.minY))
        p.addArc(center: CGPoint(x: r.maxX - tr, y: r.minY + tr), radius: tr, startAngle: .degrees(-90), endAngle: .degrees(0), clockwise: false)
        p.addLine(to: CGPoint(x: r.maxX, y: r.maxY - br))
        p.addArc(center: CGPoint(x: r.maxX - br, y: r.maxY - br), radius: br, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        p.addLine(to: CGPoint(x: r.minX + bl, y: r.maxY))
        p.addArc(center: CGPoint(x: r.minX + bl, y: r.maxY - bl), radius: bl, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false)
        p.addLine(to: CGPoint(x: r.minX, y: r.minY + tl))
        p.addArc(center: CGPoint(x: r.minX + tl, y: r.minY + tl), radius: tl, startAngle: .degrees(180), endAngle: .degrees(270), clockwise: false)
        p.closeSubpath()
        return p
    }
}

private func cornerShape(_ n: J) -> RoundedCorners? {
    if n.str("shape") == "circle" { return RoundedCorners(tl: 10000, tr: 10000, bl: 10000, br: 10000) }
    guard let r = n.obj("radius") else { return nil }
    return RoundedCorners(tl: r.num("tl") ?? 0, tr: r.num("tr") ?? 0, bl: r.num("bl") ?? 0, br: r.num("br") ?? 0)
}

private func edgeInsets(_ o: J?) -> EdgeInsets? {
    guard let o = o else { return nil }
    return EdgeInsets(top: o.num("t") ?? 0, leading: o.num("l") ?? 0, bottom: o.num("b") ?? 0, trailing: o.num("r") ?? 0)
}

private func align(_ a: J?, _ def: Alignment = .topLeading) -> Alignment {
    guard let a = a else { return def }
    let x = a.num("x") ?? 0, y = a.num("y") ?? 0
    let h: HorizontalAlignment = x < -0.33 ? .leading : (x > 0.33 ? .trailing : .center)
    let v: VerticalAlignment = y < -0.33 ? .top : (y > 0.33 ? .bottom : .center)
    return Alignment(horizontal: h, vertical: v)
}

private func textAlign(_ s: String?) -> TextAlignment {
    switch s { case "center": return .center; case "right", "end": return .trailing; default: return .leading }
}

private func gradient(_ g: J?) -> LinearGradient? {
    guard let g = g, let cs = g["colors"] as? [String], cs.count >= 2 else { return nil }
    let colors = cs.map { Theme.color($0) ?? .clear }
    let b = g.obj("begin"), e = g.obj("end")
    func pt(_ o: J?, _ dx: CGFloat) -> UnitPoint { UnitPoint(x: ((o?.num("x") ?? dx) + 1) / 2, y: ((o?.num("y") ?? 0) + 1) / 2) }
    return LinearGradient(colors: colors, startPoint: pt(b, -1), endPoint: pt(e, 1))
}

struct Node: View {
    let n: J?
    let c: Ctx
    @Environment(\.dragonTheme) private var theme
    var body: some View {
        if let n = n { build(n) }
    }

    @ViewBuilder private func build(_ n: J) -> some View {
        switch n.str("type") ?? "" {
        case "text": NText(n: n)
        case "richText": NRichText(n: n, c: c)
        case "icon": MIcon(name: n.str("name") ?? "help", size: n.num("size") ?? 24, tint: Theme.color(n.str("color")))
        case "image": NImage(n: n)
        case "nativeview": NNativeView(n: n, c: c)
        case "container": NContainer(n: n, c: c)
        case "padding": Node(n: n.obj("child"), c: c.other()).padding(edgeInsets(n.obj("padding")) ?? EdgeInsets())
        case "align":
            let a = align(n.obj("alignment"), .center)
            Node(n: n.obj("child"), c: c.other()).frame(maxWidth: .infinity, maxHeight: c.inColumn ? nil : .infinity, alignment: a)
        case "sizedBox": NSizedBox(n: n, c: c)
        case "constrained":
            Node(n: n.obj("child"), c: c.other())
                .frame(minWidth: n.num("minWidth"), maxWidth: n.num("maxWidth").map { $0 >= INF ? .infinity : $0 },
                       minHeight: n.num("minHeight"), maxHeight: n.num("maxHeight").map { $0 >= INF ? .infinity : $0 })
        case "fractional":
            GeometryReader { g in
                Node(n: n.obj("child"), c: c.other())
                    .frame(width: g.size.width * (n.num("widthFactor") ?? 1), height: n.num("heightFactor").map { g.size.height * $0 })
            }
        case "aspectRatio": Node(n: n.obj("child"), c: c.other()).aspectRatio(n.num("ratio") ?? 1, contentMode: .fit)
        case "opacity": Node(n: n.obj("child"), c: c.other()).opacity(Double(n.num("opacity") ?? 1)).animation(.easeInOut(duration: Double(n.int("duration") ?? 0) / 1000), value: n.num("opacity"))
        case "visibility": EmptyView()
        case "ignorePointer": Node(n: n.obj("child"), c: c.other()).allowsHitTesting(n.bool("ignoring") == false)
        case "clip": NClip(n: n, c: c)
        case "transform": NTransform(n: n, c: c)
        case "safeArea":
            Node(n: n.obj("child"), c: c.other())
                .padding(.top, n.bool("top") == false ? 0 : c.topInset)
                .padding(.bottom, n.bool("bottom") == false ? 0 : c.bottomInset)
        case "flex": NFlex(n: n, c: c)
        case "flexible": NFlexible(n: n, c: c)
        case "wrap": NWrap(n: n, c: c)
        case "stack": NStack(n: n, c: c)
        case "positioned": NPositioned(n: n, c: c)
        case "divider":
            if n.str("dir") == "vertical" { Rectangle().fill(Color.gray.opacity(0.3)).frame(width: n.num("thickness") ?? 1) }
            else { Rectangle().fill(Color.gray.opacity(0.3)).frame(height: n.num("thickness") ?? 1) }
        case "scroll": NScroll(n: n, c: c)
        case "list": NList(n: n, c: c)
        case "grid": NGrid(n: n, c: c)
        case "pageView": NPager(n: n, c: c)
        case "gesture": NGesture(n: n, c: c)
        case "dismissible": NDismissible(n: n, c: c)
        case "canvas": NCanvas(n: n)
        case "animatedSwitcher": Node(n: n.obj("child"), c: c.other())
        case "button": NButton(n: n, c: c)
        case "checkbox": NCheckbox(n: n, c: c)
        case "switch":
            Toggle("", isOn: Binding(get: { n.bool("value") ?? false }, set: { c.fire(n.str("onChanged"), $0 ? "true" : "false") }))
                .labelsHidden().disabled(!(n.bool("enabled") ?? true))
        case "radio":
            Button { c.fire(n.str("onChanged")) } label: {
                Image(systemName: (n.bool("selected") ?? false) ? "largecircle.fill.circle" : "circle")
            }.disabled(!(n.bool("enabled") ?? true))
        case "slider": NSlider(n: n, c: c)
        case "progress": NProgress(n: n)
        case "refresh": NRefresh(n: n, c: c)
        case "dropdown": NDropdown(n: n, c: c)
        case "popupMenu": NPopupMenu(n: n, c: c)
        case "textField": NTextField(n: n, c: c)
        case "scaffold": NScaffold(n: n, c: c)
        case "appBar": NAppBar(n: n, c: c)
        case "drawer": Node(n: n.obj("child"), c: c.col())
        case "navBar": NNavBar(n: n, c: c)
        case "tabBar": NTabBar(n: n, c: c)
        case "page": Node(n: n.obj("content"), c: c)
        case "errorBox":
            Text(n.str("message") ?? "error").font(.system(size: 12)).foregroundColor(.white).padding(8).background(Color.red.opacity(0.8))
        default:
            if let kids = n.arr("children") {
                VStack(spacing: 0) { ForEach(Array(kids.enumerated()), id: \.offset) { _, k in Node(n: k, c: c.col()) } }
            } else if let k = n.obj("child") { Node(n: k, c: c.other()) }
        }
    }
}

struct NText: View {
    let n: J
    var body: some View {
        let st = n.obj("style")
        let scale = n.num("scale") ?? 1
        let size = (st?.num("size") ?? 14) * scale
        var t = Text(n.str("text") ?? "").font(.system(size: size, weight: weight(st?.int("weight"))))
        if st?.bool("italic") == true { t = t.italic() }
        if let sp = st?.num("spacing") { t = t.kerning(sp) }
        let deco = st?.str("decoration") ?? ""
        if deco.contains("underline") { t = t.underline() }
        if deco.contains("lineThrough") || deco.contains("line-through") { t = t.strikethrough() }
        return t
            .foregroundColor(Theme.color(st?.str("color")))
            .multilineTextAlignment(textAlign(n.str("align")))
            .lineLimit(n.int("maxLines") ?? (n.bool("softWrap") == false ? 1 : nil))
            .truncationMode(n.str("overflow") == "ellipsis" ? .tail : .head)
            .lineSpacing(st?.num("lh").map { (size * ($0 - 1)) } ?? 0)
            .background(Theme.color(st?.str("bg")))
            .frame(maxWidth: n.str("align") == nil ? nil : .infinity, alignment: n.str("align") == "center" ? .center : (n.str("align") == "right" ? .trailing : .leading))
    }
    private func weight(_ w: Int?) -> Font.Weight {
        guard let w = w else { return .regular }
        switch w { case ..<150: return .ultraLight; case ..<250: return .thin; case ..<350: return .light
        case ..<450: return .regular; case ..<550: return .medium; case ..<650: return .semibold
        case ..<750: return .bold; case ..<850: return .heavy; default: return .black }
    }
}

struct NRichText: View {
    let n: J
    let c: Ctx
    var body: some View {
        var t = Text("")
        for s in n.arr("spans") ?? [] {
            let st = s.obj("style")
            var part = Text(s.str("text") ?? "").font(.system(size: st?.num("size") ?? n.obj("style")?.num("size") ?? 14, weight: (st?.int("weight") ?? 400) >= 600 ? .bold : .regular))
            if st?.bool("italic") == true { part = part.italic() }
            if let col = Theme.color(st?.str("color")) { part = part.foregroundColor(col) }
            if (st?.str("decoration") ?? "").contains("underline") { part = part.underline() }
            t = t + part
        }
        let firstTap = (n.arr("spans") ?? []).compactMap { $0.str("onTap") }.first
        return t.multilineTextAlignment(textAlign(n.str("align"))).lineLimit(n.int("maxLines"))
            .onTapGesture { c.fire(firstTap) }
    }
}

struct MIcon: View {
    let name: String
    let size: CGFloat
    let tint: Color?
    var body: some View {
        Text(name)
            .font(.custom("Material Icons", size: size))
            .foregroundColor(tint)
            .lineLimit(1)
            .fixedSize()
            .frame(width: size, height: size)
    }
}

enum DragonFonts {
    static func register() {
        if let url = Bundle.main.url(forResource: "MaterialIcons-Regular", withExtension: "ttf") {
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}

struct NImage: View {
    let n: J
    var body: some View {
        let src = n.str("src") ?? ""
        let w = n.num("width"), h = n.num("height")
        let mode: ContentMode = (n.str("fit") == "cover" || n.str("fit") == "fitWidth") ? .fill : .fit
        Group {
            switch n.str("kind") {
            case "asset":
                if let ui = UIImage(contentsOfFile: Bundle.main.bundlePath + "/" + src) { Image(uiImage: ui).resizable().aspectRatio(contentMode: mode) } else { Color.gray.opacity(0.2) }
            case "file":
                if let ui = UIImage(contentsOfFile: src) { Image(uiImage: ui).resizable().aspectRatio(contentMode: mode) } else { Color.gray.opacity(0.2) }
            default:
                AsyncImage(url: URL(string: src)) { ph in
                    if let img = ph.image { img.resizable().aspectRatio(contentMode: mode) } else { Color.gray.opacity(0.15) }
                }
            }
        }
        .frame(width: isFill(w) ? nil : w, height: isFill(h) ? nil : h)
        .frame(maxWidth: isFill(w) ? .infinity : nil, maxHeight: isFill(h) ? .infinity : nil)
        .clipped()
    }
}

struct NNativeView: View {
    let n: J
    let c: Ctx
    var body: some View {
        let kind = n.str("kind") ?? "", id = n.str("id") ?? ""
        let params = n.obj("params") ?? [:]
        if let v = c.host.plugins.lazy.compactMap({ $0.makeView(kind: kind, id: id, params: params) }).first {
            v
        } else {
            Text("No plugin for native view “\(kind)”").font(.caption).padding(8).background(Color.gray.opacity(0.2))
        }
    }
}

struct NContainer: View {
    let n: J
    let c: Ctx
    var body: some View {
        let w = n.num("width"), h = n.num("height")
        let shape = cornerShape(n)
        let grad = gradient(n.obj("gradient"))
        let bg = Theme.color(n.str("color"))
        let pad = edgeInsets(n.obj("padding"))
        let margin = edgeInsets(n.obj("margin"))
        let dur = Double(n.int("duration") ?? 0) / 1000
        let elev = n.num("elevation") ?? 0
        let a = n.obj("alignment")
        var inner = AnyView(Node(n: n.obj("child"), c: c.other()))
        if let p = pad { inner = AnyView(inner.padding(p)) }
        if a != nil || isFill(w) || isFill(h) {
            inner = AnyView(inner.frame(maxWidth: (a != nil && w == nil) || isFill(w) ? .infinity : nil,
                                        maxHeight: (a != nil && h == nil && !c.inColumn) || isFill(h) ? .infinity : nil,
                                        alignment: align(a, .center)))
        }
        if let w = w, !isFill(w) { inner = AnyView(inner.frame(width: w)) }
        if let h = h, !isFill(h) { inner = AnyView(inner.frame(height: h)) }
        if n.num("minWidth") != nil || n.num("minHeight") != nil { inner = AnyView(inner.frame(minWidth: n.num("minWidth"), minHeight: n.num("minHeight"))) }
        if let mw = n.num("maxWidth") { inner = AnyView(inner.frame(maxWidth: mw)) }
        if let mh = n.num("maxHeight") { inner = AnyView(inner.frame(maxHeight: mh)) }
        if let g = grad { inner = AnyView(inner.background(g)) }
        else if let b = bg { inner = AnyView(inner.background(b)) }
        if let s = shape, n.bool("clip") == true || grad != nil || bg != nil || n.str("shape") == "circle" || n.obj("radius") != nil {
            inner = AnyView(inner.clipShape(s))
        }
        if let b = n.obj("border"), b.obj("sides") == nil {
            let col = Theme.color(b.str("color")) ?? .black
            let bw = b.num("width") ?? 1
            if let s = shape { inner = AnyView(inner.overlay(s.stroke(col, lineWidth: bw))) }
            else { inner = AnyView(inner.overlay(Rectangle().stroke(col, lineWidth: bw))) }
        }
        let shadowBlur = (n.arr("shadows")?.first?.num("blur")).map { $0 / 2 } ?? elev
        if shadowBlur > 0 { inner = AnyView(inner.shadow(color: .black.opacity(0.25), radius: shadowBlur, x: 0, y: shadowBlur / 2)) }
        if let m = margin { inner = AnyView(inner.padding(m)) }
        if n.obj("child") == nil && w == nil && h == nil && (c.inColumn || c.inRow) { return AnyView(Color.clear.frame(width: 0, height: 0)) }
        return AnyView(inner.animation(dur > 0 ? .easeInOut(duration: dur) : nil, value: n.str("color") ?? "" + "\(w ?? 0)\(h ?? 0)"))
    }
}

struct NSizedBox: View {
    let n: J
    let c: Ctx
    var body: some View {
        let w = n.num("width"), h = n.num("height")
        Node(n: n.obj("child"), c: c.other())
            .frame(width: isFill(w) ? nil : w, height: isFill(h) ? nil : h)
            .frame(maxWidth: isFill(w) ? .infinity : nil, maxHeight: isFill(h) ? .infinity : nil)
    }
}

struct NClip: View {
    let n: J
    let c: Ctx
    var body: some View {
        let child = Node(n: n.obj("child"), c: c.other())
        if n.str("shape") == "circle" || n.str("shape") == "oval" { child.clipShape(Circle()) }
        else if let s = cornerShape(n) { child.clipShape(s) }
        else { child.clipped() }
    }
}

struct NTransform: View {
    let n: J
    let c: Ctx
    var body: some View {
        let sx = n.num("scaleX") ?? n.num("scale") ?? 1, sy = n.num("scaleY") ?? n.num("scale") ?? 1
        Node(n: n.obj("child"), c: c.other())
            .scaleEffect(x: sx, y: sy)
            .rotationEffect(.radians(Double(n.num("rotate") ?? 0)))
            .offset(x: n.num("dx") ?? 0, y: n.num("dy") ?? 0)
    }
}

private func distribute(_ n: J, vertical: Bool, kids: [J], c: Ctx) -> some View {
    let main = n.str("main") ?? "start"
    let isMax = n.str("mainSize") == "max"
    let lead = main == "end" || main == "center" || main == "spaceEvenly" || main == "spaceAround"
    let between = main == "spaceBetween" || main == "spaceEvenly" || main == "spaceAround"
    let trail = (main == "start" && isMax) || main == "center" || main == "spaceEvenly" || main == "spaceAround"
    return Group {
        if lead { Spacer(minLength: 0) }
        ForEach(Array(kids.enumerated()), id: \.offset) { i, k in
            if i > 0 && between { Spacer(minLength: 0) }
            Node(n: k, c: vertical ? c.col() : c.row())
        }
        if trail { Spacer(minLength: 0) }
    }
}

struct NFlex: View {
    let n: J
    let c: Ctx
    var body: some View {
        let vertical = n.str("dir") == "vertical"
        let cross = n.str("cross") ?? "center"
        let spacing = n.num("spacing") ?? 0
        let kids = n.arr("children") ?? []
        let isMax = n.str("mainSize") == "max"
        if vertical {
            let h: HorizontalAlignment = cross == "start" ? .leading : (cross == "end" ? .trailing : .center)
            VStack(alignment: h, spacing: spacing) { distribute(n, vertical: true, kids: kids, c: c) }
                .frame(maxWidth: cross == "stretch" ? .infinity : nil, maxHeight: isMax && !c.inColumn ? .infinity : nil)
        } else {
            let v: VerticalAlignment = cross == "start" ? .top : (cross == "end" ? .bottom : .center)
            HStack(alignment: v, spacing: spacing) { distribute(n, vertical: false, kids: kids, c: c) }
                .frame(maxWidth: isMax ? .infinity : nil)
        }
    }
}

struct NFlexible: View {
    let n: J
    let c: Ctx
    var body: some View {
        let tight = n.str("fit") == "tight"
        let child = Node(n: n.obj("child"), c: c.other())
        if c.inColumn { child.frame(maxHeight: tight ? .infinity : nil, alignment: .top).layoutPriority(Double(n.num("flex") ?? 1)) }
        else { child.frame(maxWidth: tight ? .infinity : nil, alignment: .leading).layoutPriority(Double(n.num("flex") ?? 1)) }
    }
}

struct FlowLayout: Layout {
    var spacing: CGFloat = 0
    var runSpacing: CGFloat = 0
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxW = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0, width: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > 0 && x + sz.width > maxW { y += rowH + runSpacing; x = 0; rowH = 0 }
            x += sz.width + spacing; rowH = max(rowH, sz.height); width = max(width, x - spacing)
        }
        return CGSize(width: width, height: y + rowH)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowH: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > bounds.minX && x + sz.width > bounds.maxX { y += rowH + runSpacing; x = bounds.minX; rowH = 0 }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sz))
            x += sz.width + spacing; rowH = max(rowH, sz.height)
        }
    }
}

struct NWrap: View {
    let n: J
    let c: Ctx
    var body: some View {
        FlowLayout(spacing: n.num("spacing") ?? 0, runSpacing: n.num("runSpacing") ?? 0) {
            ForEach(Array((n.arr("children") ?? []).enumerated()), id: \.offset) { _, k in Node(n: k, c: c.other()) }
        }
    }
}

struct NStack: View {
    let n: J
    let c: Ctx
    var body: some View {
        ZStack(alignment: align(n.obj("alignment"), .topLeading)) {
            ForEach(Array((n.arr("children") ?? []).enumerated()), id: \.offset) { _, k in Node(n: k, c: c.other()) }
        }
        .frame(maxWidth: n.str("fit") == "expand" ? .infinity : nil, maxHeight: n.str("fit") == "expand" ? .infinity : nil)
    }
}

struct NPositioned: View {
    let n: J
    let c: Ctx
    var body: some View {
        let l = n.num("left"), t = n.num("top"), r = n.num("right"), b = n.num("bottom")
        let a = Alignment(horizontal: l != nil ? .leading : (r != nil ? .trailing : .center), vertical: t != nil ? .top : (b != nil ? .bottom : .center))
        Node(n: n.obj("child"), c: c.other())
            .frame(width: n.num("width"), height: n.num("height"))
            .padding(EdgeInsets(top: t ?? 0, leading: l ?? 0, bottom: b ?? 0, trailing: r ?? 0))
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: a)
    }
}

struct NScroll: View {
    let n: J
    let c: Ctx
    var body: some View {
        let horizontal = n.str("dir") == "horizontal"
        let content = Node(n: n.obj("child"), c: horizontal ? c.row() : c.col()).padding(edgeInsets(n.obj("padding")) ?? EdgeInsets())
        if n.bool("noScroll") == true { content }
        else {
            ScrollViewReader { proxy in
                ScrollView(horizontal ? .horizontal : .vertical, showsIndicators: false) {
                    content.id("dragon-top")
                    Color.clear.frame(width: 1, height: 1).id("dragon-end")
                }
                .onReceive(c.host.$scrollCommands) { cmds in
                    guard let id = n.int("controller"), let cmd = cmds[id] else { return }
                    withAnimation { proxy.scrollTo(cmd.bool("end") == true ? "dragon-end" : "dragon-top", anchor: cmd.bool("end") == true ? .bottom : .top) }
                }
            }
        }
    }
}

struct NList: View {
    let n: J
    let c: Ctx
    var body: some View {
        let horizontal = n.str("dir") == "horizontal"
        let kids = n.arr("children") ?? []
        let pad = edgeInsets(n.obj("padding")) ?? EdgeInsets()
        let extent = n.num("itemExtent")
        let item: (J) -> AnyView = { k in
            let v = Node(n: k, c: horizontal ? c.row() : c.col())
            if let e = extent { return AnyView(horizontal ? v.frame(width: e) : v.frame(height: e)) }
            return AnyView(v)
        }
        let rows = Array(kids.enumerated())
        if n.bool("noScroll") == true || n.bool("shrinkWrap") == true {
            if horizontal { HStack(spacing: 0) { ForEach(rows, id: \.offset) { _, k in item(k) } }.padding(pad) }
            else { VStack(spacing: 0) { ForEach(rows, id: \.offset) { _, k in item(k) } }.padding(pad) }
        } else if horizontal {
            ScrollView(.horizontal, showsIndicators: false) { LazyHStack(spacing: 0) { ForEach(rows, id: \.offset) { _, k in item(k) } }.padding(pad) }
        } else {
            ScrollView(.vertical, showsIndicators: false) { LazyVStack(spacing: 0) { ForEach(rows, id: \.offset) { _, k in item(k) } }.padding(pad) }
        }
    }
}

struct NGrid: View {
    let n: J
    let c: Ctx
    var body: some View {
        let ms = n.num("mainSpacing") ?? 0, cs = n.num("crossSpacing") ?? 0
        let cols: [GridItem] = n.int("columns").map { Array(repeating: GridItem(.flexible(), spacing: cs), count: $0) }
            ?? [GridItem(.adaptive(minimum: (n.num("maxExtent") ?? 120) * 0.75), spacing: cs)]
        ScrollView(showsIndicators: false) {
            LazyVGrid(columns: cols, spacing: ms) {
                ForEach(Array((n.arr("children") ?? []).enumerated()), id: \.offset) { _, k in
                    Node(n: k, c: c.other()).aspectRatio(n.num("ratio") ?? 1, contentMode: .fit)
                }
            }.padding(edgeInsets(n.obj("padding")) ?? EdgeInsets())
        }
    }
}

struct NPager: View {
    let n: J
    let c: Ctx
    var body: some View {
        let kids = n.arr("children") ?? []
        TabView(selection: Binding(get: { n.int("index") ?? 0 }, set: { p in if p != (n.int("index") ?? 0) { c.fire(n.str("onChanged"), p) } })) {
            ForEach(Array(kids.enumerated()), id: \.offset) { i, k in Node(n: k, c: c.other()).tag(i) }
        }.tabViewStyle(.page(indexDisplayMode: .never))
    }
}

struct NRefresh: View {
    let n: J
    let c: Ctx
    var body: some View {
        ScrollView { Node(n: n.obj("child"), c: c.other()) }
            .refreshable { c.fire(n.str("onRefresh")); try? await Task.sleep(nanoseconds: 600_000_000) }
    }
}

struct NGesture: View {
    let n: J
    let c: Ctx
    var body: some View {
        var v = AnyView(Node(n: n.obj("child"), c: c.other()).contentShape(Rectangle()))
        if n.str("onDoubleTap") != nil { v = AnyView(v.onTapGesture(count: 2) { c.fire(n.str("onDoubleTap")) }) }
        if n.str("onTap") != nil { v = AnyView(v.onTapGesture { c.fire(n.str("onTap")) }) }
        if n.str("onLongPress") != nil { v = AnyView(v.onLongPressGesture { c.fire(n.str("onLongPress")) }) }
        if n.str("onPanStart") != nil || n.str("onPanUpdate") != nil || n.str("onPanEnd") != nil {
            v = AnyView(v.gesture(DragGesture(minimumDistance: 1)
                .onChanged { g in
                    if g.translation == .zero || (abs(g.translation.width) < 2 && abs(g.translation.height) < 2) { c.fire(n.str("onPanStart"), "\(g.startLocation.x),\(g.startLocation.y)") }
                    c.fire(n.str("onPanUpdate"), "\(g.location.x),\(g.location.y)")
                }
                .onEnded { _ in c.fire(n.str("onPanEnd")) }))
        }
        if n.str("onHDrag") != nil || n.str("onHDragEnd") != nil || n.str("onVDrag") != nil || n.str("onVDragEnd") != nil {
            v = AnyView(HDragWrapper(content: v, n: n, c: c))
        }
        return v
    }
}

struct HDragWrapper: View {
    let content: AnyView
    let n: J
    let c: Ctx
    @State private var last: CGSize = .zero
    var body: some View {
        content.gesture(DragGesture(minimumDistance: 4)
            .onChanged { g in
                let dx = g.translation.width - last.width, dy = g.translation.height - last.height
                last = g.translation
                if abs(g.translation.width) >= abs(g.translation.height) { if n.str("onHDrag") != nil { c.fire(n.str("onHDrag"), dx) } }
                else if n.str("onVDrag") != nil { c.fire(n.str("onVDrag"), dy) }
            }
            .onEnded { g in
                last = .zero
                if abs(g.translation.width) >= abs(g.translation.height) { c.fire(n.str("onHDragEnd")) } else { c.fire(n.str("onVDragEnd")) }
            })
    }
}

struct NDismissible: View {
    let n: J
    let c: Ctx
    @State private var dx: CGFloat = 0
    var body: some View {
        GeometryReader { g in
            ZStack {
                if let bg = n.obj("background") { Node(n: bg, c: c.other()) } else { Color.red }
                Node(n: n.obj("child"), c: c.other()).frame(width: g.size.width).background(Color(UIColor.systemBackground)).offset(x: dx)
                    .gesture(DragGesture().onChanged { dx = $0.translation.width }
                        .onEnded { v in
                            if abs(dx) > g.size.width * 0.4 { c.fire(n.str("onDismissed"), dx > 0 ? "startToEnd" : "endToStart") }
                            withAnimation { dx = 0 }
                        })
            }
        }.fixedSize(horizontal: false, vertical: true)
    }
}

struct NCanvas: View {
    let n: J
    var body: some View {
        let w = n.num("width") ?? 100, h = n.num("height") ?? 100
        Canvas { ctx, _ in
            for o in n.arr("ops") ?? [] { draw(o, &ctx) }
        }.frame(width: w, height: h)
    }
    private func draw(_ o: J, _ ctx: inout GraphicsContext) {
        let p = o.obj("paint")
        let col = Theme.color(p?.str("color")) ?? .black
        let stroke = p?.str("style") == "stroke"
        let sw = p?.num("width") ?? 1
        func v(_ k: String) -> CGFloat { o.num(k) ?? 0 }
        func paint(_ path: Path) {
            if stroke { ctx.stroke(path, with: .color(col), style: StrokeStyle(lineWidth: sw, lineCap: p?.str("cap") == "round" ? .round : .butt)) }
            else { ctx.fill(path, with: .color(col)) }
        }
        switch o.str("op") ?? "" {
        case "line":
            var path = Path(); path.move(to: CGPoint(x: v("x1"), y: v("y1"))); path.addLine(to: CGPoint(x: v("x2"), y: v("y2")))
            ctx.stroke(path, with: .color(col), style: StrokeStyle(lineWidth: sw, lineCap: p?.str("cap") == "round" ? .round : .butt))
        case "rect": paint(Path(CGRect(x: v("x"), y: v("y"), width: v("w"), height: v("h"))))
        case "rrect":
            let r = o.obj("radius")?.num("tl") ?? 0
            paint(Path(roundedRect: CGRect(x: v("x"), y: v("y"), width: v("w"), height: v("h")), cornerRadius: r))
        case "circle": paint(Path(ellipseIn: CGRect(x: v("x") - v("r"), y: v("y") - v("r"), width: v("r") * 2, height: v("r") * 2)))
        case "oval": paint(Path(ellipseIn: CGRect(x: v("x"), y: v("y"), width: v("w"), height: v("h"))))
        case "text":
            ctx.draw(Text(o.str("text") ?? "").font(.system(size: o.num("size") ?? 14)).foregroundColor(col), at: CGPoint(x: v("x"), y: v("y")), anchor: .topLeading)
        default: break
        }
    }
}

struct NButton: View {
    let n: J
    let c: Ctx
    var body: some View {
        let variant = n.str("variant") ?? "text"
        let enabled = n.bool("enabled") ?? true
        let fg = Theme.color(n.str("fg")), bg = Theme.color(n.str("bg"))
        let action = { c.fire(n.str("onPressed")) }
        let label = HStack(spacing: 8) {
            Node(n: n.obj("child"), c: c.row())
            if n.obj("label") != nil { Node(n: n.obj("label"), c: c.row()) }
        }
        Group {
            switch variant {
            case "filled", "elevated", "fab", "fabSmall", "fabLarge", "fabExtended":
                let isFab = variant.hasPrefix("fab")
                Button(action: action) { label.padding(.horizontal, isFab ? 16 : 20).padding(.vertical, isFab ? 16 : 10) }
                    .foregroundColor(fg ?? .white)
                    .background(bg ?? Theme.primary(nil))
                    .clipShape(isFab ? AnyShape(RoundedRectangle(cornerRadius: 16)) : AnyShape(Capsule()))
                    .shadow(radius: isFab || variant == "elevated" ? 4 : 0, y: 2)
            case "tonal":
                Button(action: action) { label.padding(.horizontal, 20).padding(.vertical, 10) }
                    .foregroundColor(fg ?? .primary).background(bg ?? Color.accentColor.opacity(0.18)).clipShape(Capsule())
            case "outlined":
                Button(action: action) { label.padding(.horizontal, 20).padding(.vertical, 10) }
                    .foregroundColor(fg ?? .accentColor)
                    .overlay(Capsule().stroke(Theme.color(n.str("borderColor")) ?? Color.gray.opacity(0.6), lineWidth: n.num("borderWidth") ?? 1))
            case "icon", "iconFilled", "iconTonal", "iconOutlined":
                Button(action: action) { label.padding(8) }.foregroundColor(fg ?? .primary)
            default:
                Button(action: action) { label.padding(.horizontal, 12).padding(.vertical, 8) }.foregroundColor(fg ?? .accentColor)
            }
        }
        .buttonStyle(.plain)
        .frame(width: isFill(n.num("width")) ? nil : n.num("width"), height: n.num("height"))
        .frame(maxWidth: isFill(n.num("width")) ? .infinity : nil)
        .disabled(!enabled).opacity(enabled ? 1 : 0.4)
    }
}

struct AnyShape: Shape {
    private let make: (CGRect) -> Path
    init<S: Shape>(_ s: S) { make = { s.path(in: $0) } }
    func path(in rect: CGRect) -> Path { make(rect) }
}

struct NCheckbox: View {
    let n: J
    let c: Ctx
    var body: some View {
        let on = n.bool("value") ?? false
        Button { c.fire(n.str("onChanged"), on ? "false" : "true") } label: {
            Image(systemName: on ? "checkmark.square.fill" : "square").foregroundColor(Theme.color(n.str("color")) ?? .accentColor).font(.system(size: 22))
        }.buttonStyle(.plain).disabled(!(n.bool("enabled") ?? true))
    }
}

struct NSlider: View {
    let n: J
    let c: Ctx
    @State private var local: Double? = nil
    var body: some View {
        let lo = Double(n.num("min") ?? 0), hi = Double(n.num("max") ?? 1)
        let value = local ?? Double(n.num("value") ?? CGFloat(lo))
        Slider(value: Binding(get: { value }, set: { local = $0; c.fire(n.str("onChanged"), $0) }), in: lo...hi,
               step: (n.int("divisions") ?? 0) > 1 ? (hi - lo) / Double(n.int("divisions")!) : 0.0001,
               onEditingChanged: { editing in if !editing { c.fire(n.str("onChangeEnd")); local = nil } })
            .disabled(!(n.bool("enabled") ?? true))
    }
}

struct NProgress: View {
    let n: J
    var body: some View {
        let tint = Theme.color(n.str("color"))
        if let v = n.num("value") {
            if n.str("kind") == "linear" { ProgressView(value: Double(min(max(v, 0), 1))).tint(tint) }
            else { ProgressView(value: Double(min(max(v, 0), 1))).progressViewStyle(.circular).tint(tint) }
        } else {
            if n.str("kind") == "linear" { ProgressView().progressViewStyle(.linear).tint(tint) } else { ProgressView().tint(tint) }
        }
    }
}

struct NDropdown: View {
    let n: J
    let c: Ctx
    var body: some View {
        let items = n.arr("items") ?? []
        let sel = n.int("selected") ?? -1
        Menu {
            ForEach(Array(items.enumerated()), id: \.offset) { i, it in
                Button { c.fire(n.str("onChanged"), i) } label: { Node(n: it.obj("label"), c: c.row()) }.disabled(!(it.bool("enabled") ?? true))
            }
        } label: {
            HStack { if sel >= 0 && sel < items.count { Node(n: items[sel].obj("label"), c: c.row()) } else { Node(n: n.obj("hint"), c: c.row()) }
                Spacer(minLength: 0); Image(systemName: "chevron.down").font(.system(size: 12)) }
                .padding(.horizontal, 12).padding(.vertical, 10)
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.gray.opacity(0.6)))
                .frame(maxWidth: n.bool("expanded") == true ? .infinity : nil)
        }.disabled(!(n.bool("enabled") ?? true))
    }
}

struct NPopupMenu: View {
    let n: J
    let c: Ctx
    var body: some View {
        Menu {
            ForEach(Array((n.arr("items") ?? []).enumerated()), id: \.offset) { i, it in
                if it.bool("divider") == true { Divider() }
                else { Button { c.fire(n.str("onSelected"), i) } label: { Node(n: it.obj("label"), c: c.row()) }.disabled(!(it.bool("enabled") ?? true)) }
            }
        } label: { Node(n: n.obj("child"), c: c.row()) }
    }
}

struct NTextField: View {
    let n: J
    let c: Ctx
    @State private var text = ""
    @State private var synced = ""
    @FocusState private var focused: Bool
    var body: some View {
        let server = n.str("value") ?? ""
        let multi = (n.int("maxLines") ?? 1) != 1 || n.str("keyboard") == "multiline"
        let binding = Binding<String>(get: { text }, set: { nv in
            var v = nv
            if let ml = n.int("maxLength"), v.count > ml { v = String(v.prefix(ml)) }
            if v != text { text = v; c.fire(n.str("onChanged"), v) }
        })
        VStack(alignment: .leading, spacing: 4) {
            if let l = n.str("label") { Text(l).font(.caption).foregroundColor(.secondary) }
            HStack {
                if n.obj("prefix") != nil { Node(n: n.obj("prefix"), c: c.row()) }
                Group {
                    if n.bool("obscure") == true { SecureField(n.str("hint") ?? "", text: binding) }
                    else if multi { TextField(n.str("hint") ?? "", text: binding, axis: .vertical).lineLimit((n.int("minLines") ?? 1)...(n.int("maxLines") ?? 5)) }
                    else { TextField(n.str("hint") ?? "", text: binding) }
                }
                .focused($focused)
                .keyboardType(keyboard(n.str("keyboard")))
                .textInputAutocapitalization(cap(n.str("capitalization")))
                .submitLabel(submit(n.str("action")))
                .onSubmit { c.fire(n.str("onSubmitted")) }
                .disabled(!(n.bool("enabled") ?? true) || (n.bool("readOnly") ?? false))
                if n.obj("suffix") != nil { Node(n: n.obj("suffix"), c: c.row()) }
            }
            .padding(.horizontal, 12).padding(.vertical, 10)
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(n.str("error") != nil ? Color.red : Color.gray.opacity(0.6)))
            if let e = n.str("error") { Text(e).font(.caption).foregroundColor(.red) }
            else if let h = n.str("helper") { Text(h).font(.caption).foregroundColor(.secondary) }
        }
        .onAppear { text = server; synced = server; if n.bool("autofocus") == true { focused = true } }
        .onChange(of: server) { s in if s != text { text = s } }
        .onTapGesture { c.fire(n.str("onTap")) }
    }
    private func keyboard(_ k: String?) -> UIKeyboardType {
        switch k { case "number": return .numberPad; case "email": return .emailAddress; case "phone": return .phonePad; case "url": return .URL; default: return .default }
    }
    private func cap(_ s: String?) -> TextInputAutocapitalization {
        switch s { case "words": return .words; case "sentences": return .sentences; case "characters": return .characters; default: return .never }
    }
    private func submit(_ s: String?) -> SubmitLabel {
        switch s { case "next": return .next; case "search": return .search; case "send": return .send; case "go": return .go; default: return .done }
    }
}

struct NScaffold: View {
    let n: J
    let c: Ctx
    @ObservedObject private var host: DragonHost
    init(n: J, c: Ctx) { self.n = n; self.c = c; self.host = c.host }
    var body: some View {
        let bg = Theme.color(n.str("bg")) ?? Color(UIColor.systemBackground)
        let hasBar = n.obj("appBar") != nil, hasNav = n.obj("bottomNav") != nil
        var inner = c.other()
        inner.topInset = hasBar ? 0 : c.topInset
        inner.bottomInset = hasNav ? 0 : c.bottomInset
        return ZStack {
            bg.ignoresSafeArea()
            VStack(spacing: 0) {
                if hasBar { Node(n: n.obj("appBar"), c: c) }
                ZStack {
                    Node(n: n.obj("body"), c: inner).frame(maxWidth: .infinity, maxHeight: .infinity)
                    if let fab = n.obj("fab") {
                        let a: Alignment = n.str("fabLocation") == "center" ? .bottom : (n.str("fabLocation") == "start" ? .bottomLeading : .bottomTrailing)
                        Node(n: fab, c: inner).padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: a)
                    }
                    if let sheet = n.obj("bottomSheet") { VStack { Spacer(); Node(n: sheet, c: inner) } }
                }
                if hasNav { Node(n: n.obj("bottomNav"), c: c).padding(.bottom, c.bottomInset) }
            }
            ForEach(["start", "end"], id: \.self) { side in
                if let d = side == "start" ? n.obj("drawer") : n.obj("endDrawer") {
                    let open = host.drawers[side] == true
                    Color.black.opacity(open ? 0.4 : 0).ignoresSafeArea().allowsHitTesting(open)
                        .onTapGesture { c.host.drawers[side] = false }
                    HStack(spacing: 0) {
                        if side == "end" { Spacer(minLength: 0) }
                        ScrollView { Node(n: d, c: { var x = c.col(); x.closeSide = side; return x }()).padding(.top, c.topInset) }
                            .frame(width: min(304, UIScreen.main.bounds.width * 0.85))
                            .background(Color(UIColor.systemBackground))
                        if side == "start" { Spacer(minLength: 0) }
                    }
                    .offset(x: open ? 0 : (side == "start" ? -320 : 320))
                    .allowsHitTesting(open)
                }
            }
        }
        .animation(.easeInOut(duration: 0.22), value: host.drawers)
    }
}

struct NAppBar: View {
    let n: J
    let c: Ctx
    var body: some View {
        let bg = Theme.color(n.str("bg")) ?? Color(UIColor.secondarySystemBackground)
        let fg = Theme.color(n.str("fg")) ?? .primary
        VStack(spacing: 0) {
            Spacer().frame(height: c.topInset)
            HStack(spacing: 0) {
                if let l = n.obj("leading") { Node(n: l, c: c.row()).padding(.horizontal, 4) }
                else if n.str("autoLeading") == "menu" {
                    Button { c.host.drawers["start"] = true } label: { MIcon(name: "menu", size: 24, tint: fg).padding(12) }
                } else if n.str("autoLeading") == "back" {
                    Button { c.fire(n.str("onBack")) } label: { MIcon(name: "arrow_back", size: 24, tint: fg).padding(12) }
                } else { Spacer().frame(width: 16) }
                Node(n: n.obj("title"), c: c.row())
                    .font(.system(size: 22))
                    .frame(maxWidth: .infinity, alignment: n.bool("centerTitle") == true ? .center : .leading)
                ForEach(Array((n.arr("actions") ?? []).enumerated()), id: \.offset) { _, a in Node(n: a, c: c.row()) }
            }
            .frame(height: n.num("height") ?? 56).padding(.horizontal, 4)
            if let b = n.obj("bottom") { Node(n: b, c: c.col()) }
        }
        .foregroundColor(fg).background(bg.ignoresSafeArea(edges: .top))
        .shadow(color: .black.opacity((n.num("elevation") ?? 0) > 0 ? 0.2 : 0), radius: n.num("elevation") ?? 0, y: 1)
    }
}

struct NNavBar: View {
    let n: J
    let c: Ctx
    var body: some View {
        let items = n.arr("items") ?? []
        let selected = n.int("selected") ?? 0
        HStack {
            ForEach(Array(items.enumerated()), id: \.offset) { i, it in
                let sel = i == selected
                Button { c.fire(n.str("onChanged"), i) } label: {
                    VStack(spacing: 2) {
                        MIcon(name: (sel ? it.str("activeIcon") : nil) ?? it.str("icon") ?? "circle", size: 24, tint: sel ? .accentColor : .secondary)
                        Text(it.str("label") ?? "").font(.system(size: 11)).foregroundColor(sel ? .accentColor : .secondary)
                    }.frame(maxWidth: .infinity)
                }.buttonStyle(.plain)
            }
        }
        .padding(.vertical, 8)
        .background(Theme.color(n.str("bg")) ?? Color(UIColor.secondarySystemBackground))
    }
}

struct NTabBar: View {
    let n: J
    let c: Ctx
    var body: some View {
        let labels = n.arr("labels") ?? []
        let selected = n.int("selected") ?? 0
        let row = HStack(spacing: 0) {
            ForEach(Array(labels.enumerated()), id: \.offset) { i, t in
                Button { c.fire(n.str("onChanged"), i) } label: {
                    VStack(spacing: 4) {
                        HStack(spacing: 4) {
                            if let ic = t.str("icon") { MIcon(name: ic, size: 20, tint: nil) }
                            if let tx = t.str("text") { Text(tx).font(.system(size: 14, weight: .medium)) }
                        }
                        Rectangle().fill(i == selected ? Color.accentColor : Color.clear).frame(height: 2)
                    }
                    .padding(.horizontal, 16).padding(.top, 10)
                    .frame(maxWidth: n.bool("scrollable") == true ? nil : .infinity)
                    .foregroundColor(i == selected ? .accentColor : .secondary)
                }.buttonStyle(.plain)
            }
        }
        if n.bool("scrollable") == true { ScrollView(.horizontal, showsIndicators: false) { row } } else { row }
    }
}
