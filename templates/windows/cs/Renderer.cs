using System;
using TextElement = System.Windows.Documents.TextElement;
using Run = System.Windows.Documents.Run;
using Typography = System.Windows.Documents.Typography;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;
using System.Windows.Threading;

namespace DragonRuntime
{
    internal sealed class RenderState
    {
        public readonly Dictionary<string, double> ScrollOffsets = new Dictionary<string, double>();
        public string FocusPath;
        public int CaretStart;
        public int SelectionLength;
        public bool Rebuilding;
        // Drag state lives here, not in the control: every frame rebuilds the whole tree, which would
        // otherwise drop a drag (and its mouse capture) right after it starts.
        public sealed class PanState { public bool Dragging, Moved; public Point Start, Last; }
        readonly Dictionary<string, PanState> pans = new Dictionary<string, PanState>();
        public PanState Pan(string path) { if (!pans.TryGetValue(path, out var p)) pans[path] = p = new PanState(); return p; }
        public readonly List<NativeSlot> Slots = new List<NativeSlot>();
    }

    internal sealed class NativeSlot
    {
        public UIElement View;
        public FrameworkElement Holder;
    }

    internal sealed class NativePlaceholder : FrameworkElement
    {
        protected override Size MeasureOverride(Size availableSize)
        {
            double w = double.IsInfinity(availableSize.Width) ? 0 : availableSize.Width;
            double h = double.IsInfinity(availableSize.Height) ? 0 : availableSize.Height;
            return new Size(w, h);
        }
    }

    internal sealed class Ctx
    {
        public DragonHost Host;
        public RenderState State;
        public string Path = "";
        public bool InCol;
        public bool InRow;
        public string CloseSide;

        public Ctx Sub(string seg, bool col = false, bool row = false)
        {
            return new Ctx { Host = Host, State = State, Path = Path + "/" + seg, InCol = col, InRow = row, CloseSide = CloseSide };
        }

        public Ctx Other(string seg) => Sub(seg);

        public void Fire(string id)
        {
            if (id == null) return;
            if (CloseSide != null) Host.Drawers[CloseSide] = false;
            Host.Dispatch(id);
        }

        public void Fire(string id, object payload)
        {
            if (id == null) return;
            string p = payload is bool b ? (b ? "true" : "false") : Convert.ToString(payload, System.Globalization.CultureInfo.InvariantCulture);
            Host.Dispatch(id + ":" + p);
        }
    }

    internal static partial class R
    {
        public const double INF = 1.0e9;

        public static UIElement Node(JsonElement? n, Ctx c)
        {
            if (!n.HasValue || !n.Value.IsObj()) return null;
            var e = n.Value;
            switch (e.Str("type") ?? "")
            {
                case "text": return Text(e);
                case "richText": return RichText(e, c);
                case "icon": return Icon(e.Str("name") ?? "help", e.Num("size") ?? 24, Theme.BrushOf(e.Str("color")));
                case "image": return Image(e);
                case "nativeview": return NativeView(e, c);
                case "container": return Container(e, c);
                case "padding": return Padding(e, c);
                case "align": return Align(e, c);
                case "sizedBox": return SizedBox(e, c);
                case "constrained": return Constrained(e, c);
                case "fractional": return Fractional(e, c);
                case "aspectRatio": return Aspect(e, c);
                case "opacity": return Decorate(e, c, el => el.Opacity = e.Num("opacity") ?? 1);
                case "visibility": return null;
                case "ignorePointer": return Decorate(e, c, el => el.IsHitTestVisible = e.Bool("ignoring") == false);
                case "clip": return Clip(e, c);
                case "transform": return Transform(e, c);
                case "safeArea": return Child(e, c);
                case "flex": return Flex(e, c);
                case "flexible": return Flexible(e, c);
                case "wrap": return Wrap(e, c);
                case "stack": return Stack(e, c);
                case "positioned": return Positioned(e, c);
                case "divider": return Divider(e);
                case "scroll": return Scroll(e, c);
                case "list": return List(e, c);
                case "grid": return GridView(e, c);
                case "pageView": return Pager(e, c);
                case "refresh": return Child(e, c);
                case "gesture": return Gesture(e, c);
                case "dismissible": return Child(e, c);
                case "canvas": return new CanvasView(e.Arr("ops"), e.Num("width") ?? 100, e.Num("height") ?? 100);
                case "animatedSwitcher": return Child(e, c);
                case "button": return Button(e, c);
                case "checkbox": return Check(e, c);
                case "switch": return Switch(e, c);
                case "radio": return Radio(e, c);
                case "slider": return SliderNode(e, c);
                case "progress": return Progress(e);
                case "dropdown": return Dropdown(e, c);
                case "popupMenu": return PopupMenu(e, c);
                case "textField": return TextField(e, c);
                case "scaffold": return Scaffold(e, c);
                case "appBar": return AppBar(e, c);
                case "drawer": return Child(e, c.Sub("d", col: true));
                case "navBar": return NavBar(e, c);
                case "tabBar": return TabBar(e, c);
                case "page": return Node(e.Obj("content"), c);
                case "errorBox": return ErrorBox(e);
                default:
                    var kids = e.Arr("children");
                    if (kids.Count > 0) return Column(kids, c);
                    return Child(e, c);
            }
        }

        static UIElement Child(JsonElement e, Ctx c) => Node(e.Obj("child"), c.Other("c"));

        static UIElement Column(List<JsonElement> kids, Ctx c)
        {
            var p = new FlexPanel { Vertical = true, Cross = "stretch" };
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Sub(i.ToString(), col: true));
                if (k != null) p.Children.Add(k);
            }
            return p;
        }

        static UIElement Empty() => new Grid();

        static UIElement Decorate(JsonElement e, Ctx c, Action<UIElement> apply)
        {
            var el = Child(e, c) ?? Empty();
            apply(el);
            return el;
        }

        public static HorizontalAlignment HAlign(double x) => x < -0.33 ? HorizontalAlignment.Left : (x > 0.33 ? HorizontalAlignment.Right : HorizontalAlignment.Center);
        public static VerticalAlignment VAlign(double y) => y < -0.33 ? VerticalAlignment.Top : (y > 0.33 ? VerticalAlignment.Bottom : VerticalAlignment.Center);

        static Thickness Insets(JsonElement? o)
        {
            if (!o.HasValue) return new Thickness(0);
            var v = o.Value;
            return new Thickness(v.Num("l") ?? 0, v.Num("t") ?? 0, v.Num("r") ?? 0, v.Num("b") ?? 0);
        }

        static CornerRadius Radius(JsonElement? o)
        {
            if (!o.HasValue) return new CornerRadius(0);
            var v = o.Value;
            return new CornerRadius(v.Num("tl") ?? 0, v.Num("tr") ?? 0, v.Num("br") ?? 0, v.Num("bl") ?? 0);
        }

        static void ClipRounded(Border b, bool circle)
        {
            b.SizeChanged += (s, ev) =>
            {
                var cr = circle ? new CornerRadius(Math.Min(b.ActualWidth, b.ActualHeight) / 2) : b.CornerRadius;
                if (circle) b.CornerRadius = cr;
                b.Clip = new RectangleGeometry(new Rect(0, 0, b.ActualWidth, b.ActualHeight), cr.TopLeft, cr.TopLeft);
            };
        }

        static UIElement Text(JsonElement n)
        {
            var st = n.Obj("style");
            double scale = n.Num("scale") ?? 1;
            double size = (st.HasValue ? st.Value.Num("size") : null) ?? 14;
            var tb = new TextBlock
            {
                Text = n.Str("text") ?? "",
                FontSize = size * scale,
                FontFamily = Theme.Text,
                TextWrapping = n.Bool("softWrap") == false ? TextWrapping.NoWrap : TextWrapping.Wrap,
            };
            if (st.HasValue)
            {
                var s = st.Value;
                var col = Theme.BrushOf(s.Str("color"));
                if (col != null) tb.Foreground = col;
                var bg = Theme.BrushOf(s.Str("bg"));
                if (bg != null) tb.Background = bg;
                if (s.Int("weight").HasValue) tb.FontWeight = FontWeight.FromOpenTypeWeight(Math.Max(1, Math.Min(999, s.Int("weight").Value)));
                if (s.Bool("italic") == true) tb.FontStyle = FontStyles.Italic;
                string deco = s.Str("decoration") ?? "";
                var dec = new TextDecorationCollection();
                if (deco.Contains("underline")) dec.Add(TextDecorations.Underline[0]);
                if (deco.Contains("lineThrough") || deco.Contains("line-through")) dec.Add(TextDecorations.Strikethrough[0]);
                if (dec.Count > 0) tb.TextDecorations = dec;
                if (s.Num("lh").HasValue)
                {
                    tb.LineHeight = size * scale * s.Num("lh").Value;
                    tb.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
                }
            }
            switch (n.Str("align"))
            {
                case "center": tb.TextAlignment = TextAlignment.Center; break;
                case "right":
                case "end": tb.TextAlignment = TextAlignment.Right; break;
                case "justify": tb.TextAlignment = TextAlignment.Justify; break;
            }
            if (n.Str("overflow") == "ellipsis") tb.TextTrimming = TextTrimming.CharacterEllipsis;
            int? lines = n.Int("maxLines");
            if (lines.HasValue && lines.Value > 0)
            {
                double lineH = tb.LineHeight > 0 && !double.IsNaN(tb.LineHeight) ? tb.LineHeight : tb.FontFamily.LineSpacing * tb.FontSize;
                tb.MaxHeight = lineH * lines.Value;
                if (tb.TextTrimming == TextTrimming.None) tb.TextTrimming = TextTrimming.CharacterEllipsis;
            }
            return tb;
        }

        static UIElement RichText(JsonElement n, Ctx c)
        {
            var tb = new TextBlock { TextWrapping = TextWrapping.Wrap, FontFamily = Theme.Text, FontSize = n.Obj("style").HasValue ? n.Obj("style").Value.Num("size") ?? 14 : 14 };
            foreach (var s in n.Arr("spans"))
            {
                var run = new Run(s.Str("text") ?? "");
                var st = s.Obj("style");
                if (st.HasValue)
                {
                    var col = Theme.BrushOf(st.Value.Str("color"));
                    if (col != null) run.Foreground = col;
                    if (st.Value.Num("size").HasValue) run.FontSize = st.Value.Num("size").Value;
                    if (st.Value.Int("weight").HasValue) run.FontWeight = FontWeight.FromOpenTypeWeight(Math.Max(1, Math.Min(999, st.Value.Int("weight").Value)));
                    if (st.Value.Bool("italic") == true) run.FontStyle = FontStyles.Italic;
                    if ((st.Value.Str("decoration") ?? "").Contains("underline")) run.TextDecorations = TextDecorations.Underline;
                }
                string tap = s.Str("onTap");
                if (tap != null)
                {
                    run.Cursor = Cursors.Hand;
                    run.MouseLeftButtonUp += (a, b) => c.Fire(tap);
                }
                tb.Inlines.Add(run);
            }
            int? lines = n.Int("maxLines");
            if (lines.HasValue) tb.MaxHeight = tb.FontFamily.LineSpacing * tb.FontSize * lines.Value;
            return tb;
        }

        public static UIElement Icon(string name, double size, Brush tint)
        {
            var tb = new TextBlock
            {
                Text = name,
                FontFamily = Theme.Icons ?? Theme.Text,
                FontSize = size,
                Width = size,
                Height = size,
                TextAlignment = TextAlignment.Center,
                TextWrapping = TextWrapping.NoWrap,
                VerticalAlignment = VerticalAlignment.Center,
            };
            Typography.SetStandardLigatures(tb, true);
            if (tint != null) tb.Foreground = tint;
            return tb;
        }

        static UIElement Image(JsonElement n)
        {
            string src = n.Str("src") ?? "";
            var img = new System.Windows.Controls.Image();
            try
            {
                Uri uri;
                switch (n.Str("kind"))
                {
                    case "asset": uri = new Uri(System.IO.Path.Combine(AppContext.BaseDirectory, src)); break;
                    case "file": uri = new Uri(src); break;
                    default: uri = new Uri(src); break;
                }
                var bmp = new BitmapImage();
                bmp.BeginInit();
                bmp.UriSource = uri;
                bmp.CacheOption = BitmapCacheOption.OnLoad;
                bmp.EndInit();
                img.Source = bmp;
            }
            catch (Exception) { }
            switch (n.Str("fit"))
            {
                case "cover": img.Stretch = Stretch.UniformToFill; break;
                case "fill": img.Stretch = Stretch.Fill; break;
                case "fitWidth": img.Stretch = Stretch.Uniform; break;
                case "none": img.Stretch = Stretch.None; break;
                default: img.Stretch = Stretch.Uniform; break;
            }
            double? w = n.Num("width"), h = n.Num("height");
            if (w.HasValue && w.Value < INF) img.Width = w.Value;
            if (h.HasValue && h.Value < INF) img.Height = h.Value;
            var host = new Border { Child = img, ClipToBounds = true };
            if (w.HasValue && w.Value >= INF) FlexPanel.SetFillCross(host, true);
            return host;
        }

        static UIElement NativeView(JsonElement n, Ctx c)
        {
            string kind = n.Str("kind") ?? "", id = n.Str("id") ?? "";
            var prm = n.Get("params");
            foreach (var p in c.Host.Plugins)
            {
                var v = p.CreateView(kind, id, prm);
                if (v == null) continue;
                var holder = new NativePlaceholder { HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch };
                c.State.Slots.Add(new NativeSlot { View = v, Holder = holder });
                return holder;
            }
            return new TextBlock { Text = "No plugin for native view '" + kind + "'", Foreground = Theme.Solid(Theme.Outline), FontSize = 12 };
        }

        public static void Detach(UIElement v)
        {
            var parent = VisualTreeHelper.GetParent(v);
            if (parent is Panel p) p.Children.Remove(v);
            else if (parent is Decorator d) d.Child = null;
            else if (parent is ContentControl cc) cc.Content = null;
            else if (parent is ContentPresenter cp && cp.Parent is ContentControl cc2) cc2.Content = null;
        }

        static UIElement Container(JsonElement n, Ctx c)
        {
            var b = new Border();
            double? w = n.Num("width"), h = n.Num("height");
            bool fillW = w.HasValue && w.Value >= INF, fillH = h.HasValue && h.Value >= INF;
            if (w.HasValue && !fillW) { b.Width = w.Value; b.HorizontalAlignment = HorizontalAlignment.Left; }
            if (h.HasValue && !fillH) { b.Height = h.Value; b.VerticalAlignment = VerticalAlignment.Top; }
            if (n.Num("minWidth").HasValue) b.MinWidth = n.Num("minWidth").Value;
            if (n.Num("minHeight").HasValue) b.MinHeight = n.Num("minHeight").Value;
            if (n.Num("maxWidth").HasValue && n.Num("maxWidth").Value < INF) b.MaxWidth = n.Num("maxWidth").Value;
            if (n.Num("maxHeight").HasValue && n.Num("maxHeight").Value < INF) b.MaxHeight = n.Num("maxHeight").Value;
            bool circle = n.Str("shape") == "circle";
            if (n.Obj("radius").HasValue) b.CornerRadius = Radius(n.Obj("radius"));
            var grad = Gradient(n.Obj("gradient"));
            if (grad != null) b.Background = grad;
            else
            {
                var bg = Theme.BrushOf(n.Str("color"));
                if (bg != null) b.Background = bg;
            }
            var border = n.Obj("border");
            if (border.HasValue)
            {
                var sides = border.Value.Obj("sides");
                if (!sides.HasValue)
                {
                    b.BorderBrush = Theme.BrushOf(border.Value.Str("color")) ?? Brushes.Black;
                    b.BorderThickness = new Thickness(border.Value.Num("width") ?? 1);
                }
                else
                {
                    double SideW(string k) => sides.Value.Obj(k).HasValue ? sides.Value.Obj(k).Value.Num("width") ?? 1 : 0;
                    b.BorderThickness = new Thickness(SideW("l"), SideW("t"), SideW("r"), SideW("b"));
                    foreach (var k in new[] { "t", "b", "l", "r" })
                        if (sides.Value.Obj(k).HasValue) { b.BorderBrush = Theme.BrushOf(sides.Value.Obj(k).Value.Str("color")) ?? Brushes.Black; break; }
                }
            }
            double elevation = n.Num("elevation") ?? 0;
            var shadows = n.Arr("shadows");
            double blur = shadows.Count > 0 ? (shadows[0].Num("blur") ?? 0) : elevation * 2;
            if (blur > 0) b.Effect = new DropShadowEffect { BlurRadius = blur, ShadowDepth = Math.Max(1, blur / 4), Opacity = 0.28, Direction = 270 };
            if (n.Obj("padding").HasValue) b.Padding = Insets(n.Obj("padding"));
            if (n.Obj("margin").HasValue) b.Margin = Insets(n.Obj("margin"));
            if (n.Bool("clip") == true || n.Obj("radius").HasValue || circle) ClipRounded(b, circle);
            else if (circle) ClipRounded(b, true);
            var child = Node(n.Obj("child"), c.Other("c"));
            var align = n.Obj("alignment");
            if (child == null)
            {
                if (!w.HasValue && !h.HasValue && (c.InCol || c.InRow)) { b.Width = 0; b.Height = 0; }
            }
            else
            {
                if (align.HasValue && child is FrameworkElement fe)
                {
                    fe.HorizontalAlignment = HAlign(align.Value.Num("x") ?? 0);
                    fe.VerticalAlignment = VAlign(align.Value.Num("y") ?? 0);
                }
                b.Child = child;
            }
            if (align.HasValue && !w.HasValue) FlexPanel.SetFillCross(b, true);
            if (fillW || fillH) FlexPanel.SetFillCross(b, true);
            var bgImage = n.Obj("bgImage");
            if (bgImage.HasValue && child != null)
            {
                var g = new Grid();
                g.Children.Add(Image(bgImage.Value));
                b.Child = null;
                g.Children.Add(child);
                b.Child = g;
            }
            return b;
        }

        static Brush Gradient(JsonElement? g)
        {
            if (!g.HasValue) return null;
            var cs = g.Value.Arr("colors");
            if (cs.Count < 2) return null;
            var stops = new GradientStopCollection();
            for (int i = 0; i < cs.Count; i++)
                stops.Add(new GradientStop(Theme.ParseColor(cs[i].GetString()) ?? Colors.Transparent, i / (double)(cs.Count - 1)));
            if (g.Value.Str("type") == "radial") return new RadialGradientBrush(stops);
            var b = g.Value.Obj("begin");
            var e = g.Value.Obj("end");
            double bx = ((b.HasValue ? b.Value.Num("x") : null) ?? -1) + 1, by = ((b.HasValue ? b.Value.Num("y") : null) ?? 0) + 1;
            double ex = ((e.HasValue ? e.Value.Num("x") : null) ?? 1) + 1, ey = ((e.HasValue ? e.Value.Num("y") : null) ?? 0) + 1;
            return new LinearGradientBrush(stops, new Point(bx / 2, by / 2), new Point(ex / 2, ey / 2));
        }

        static UIElement Padding(JsonElement n, Ctx c)
        {
            var b = new Border { Padding = Insets(n.Obj("padding")), Child = Child(n, c) };
            return b;
        }

        static UIElement Align(JsonElement n, Ctx c)
        {
            var g = new Grid();
            var child = Child(n, c);
            var a = n.Obj("alignment");
            if (child is FrameworkElement fe)
            {
                fe.HorizontalAlignment = HAlign(a.HasValue ? a.Value.Num("x") ?? 0 : 0);
                fe.VerticalAlignment = VAlign(a.HasValue ? a.Value.Num("y") ?? 0 : 0);
            }
            if (child != null) g.Children.Add(child);
            FlexPanel.SetFillCross(g, true);
            return g;
        }

        static UIElement SizedBox(JsonElement n, Ctx c)
        {
            var b = new Border { Child = Child(n, c) };
            double? w = n.Num("width"), h = n.Num("height");
            if (w.HasValue && w.Value < INF) { b.Width = w.Value; b.HorizontalAlignment = HorizontalAlignment.Left; }
            if (h.HasValue && h.Value < INF) { b.Height = h.Value; b.VerticalAlignment = VerticalAlignment.Top; }
            if (w.HasValue && w.Value >= INF) FlexPanel.SetFillCross(b, true);
            if (h.HasValue && h.Value >= INF) FlexPanel.SetFillCross(b, true);
            return b;
        }

        static UIElement Constrained(JsonElement n, Ctx c)
        {
            var b = new Border { Child = Child(n, c) };
            if (n.Num("minWidth").HasValue) b.MinWidth = n.Num("minWidth").Value;
            if (n.Num("minHeight").HasValue) b.MinHeight = n.Num("minHeight").Value;
            if (n.Num("maxWidth").HasValue && n.Num("maxWidth").Value < INF) b.MaxWidth = n.Num("maxWidth").Value;
            if (n.Num("maxHeight").HasValue && n.Num("maxHeight").Value < INF) b.MaxHeight = n.Num("maxHeight").Value;
            return b;
        }

        static UIElement Fractional(JsonElement n, Ctx c)
        {
            var p = new FractionalPanel { WidthFactor = n.Num("widthFactor") ?? 1, HeightFactor = n.Num("heightFactor") };
            var child = Child(n, c);
            if (child != null) p.Children.Add(child);
            return p;
        }

        static UIElement Aspect(JsonElement n, Ctx c)
        {
            var p = new AspectPanel { Ratio = n.Num("ratio") ?? 1 };
            var child = Child(n, c);
            if (child != null) p.Children.Add(child);
            return p;
        }

        static UIElement Clip(JsonElement n, Ctx c)
        {
            var b = new Border { Child = Child(n, c) };
            bool circle = n.Str("shape") == "circle" || n.Str("shape") == "oval";
            if (n.Obj("radius").HasValue) b.CornerRadius = Radius(n.Obj("radius"));
            ClipRounded(b, circle);
            return b;
        }

        static UIElement Transform(JsonElement n, Ctx c)
        {
            var el = Child(n, c) ?? Empty();
            double sx = n.Num("scaleX") ?? n.Num("scale") ?? 1, sy = n.Num("scaleY") ?? n.Num("scale") ?? 1;
            var tg = new TransformGroup();
            tg.Children.Add(new ScaleTransform(sx, sy));
            tg.Children.Add(new RotateTransform((n.Num("rotate") ?? 0) * 180 / Math.PI));
            tg.Children.Add(new TranslateTransform(n.Num("dx") ?? 0, n.Num("dy") ?? 0));
            el.RenderTransform = tg;
            var a = n.Obj("alignment");
            el.RenderTransformOrigin = a.HasValue ? new Point(((a.Value.Num("x") ?? 0) + 1) / 2, ((a.Value.Num("y") ?? 0) + 1) / 2) : new Point(0.5, 0.5);
            return el;
        }

        static UIElement Flex(JsonElement n, Ctx c)
        {
            bool vertical = n.Str("dir") == "vertical";
            var p = new FlexPanel
            {
                Vertical = vertical,
                Main = n.Str("main") ?? "start",
                Cross = n.Str("cross") ?? "center",
                Spacing = n.Num("spacing") ?? 0,
                FillMain = n.Str("mainSize") == "max",
            };
            var kids = n.Arr("children");
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Sub(i.ToString(), col: vertical, row: !vertical));
                if (k != null) p.Children.Add(k);
            }
            return p;
        }

        static UIElement Flexible(JsonElement n, Ctx c)
        {
            var child = Node(n.Obj("child"), c.Other("f")) ?? Empty();
            FlexPanel.SetFlex(child, Math.Max(0.0001, n.Num("flex") ?? 1));
            FlexPanel.SetTight(child, n.Str("fit") != "loose");
            return child;
        }

        static UIElement Wrap(JsonElement n, Ctx c)
        {
            double sp = n.Num("spacing") ?? 0, rsp = n.Num("runSpacing") ?? 0;
            bool vertical = n.Str("dir") == "vertical";
            var p = new WrapPanel { Orientation = vertical ? Orientation.Vertical : Orientation.Horizontal };
            var kids = n.Arr("children");
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Other(i.ToString()));
                if (k == null) continue;
                var holder = new Border { Child = k, Margin = vertical ? new Thickness(0, 0, rsp, sp) : new Thickness(0, 0, sp, rsp) };
                p.Children.Add(holder);
            }
            return p;
        }

        static UIElement Stack(JsonElement n, Ctx c)
        {
            var g = new Grid();
            var a = n.Obj("alignment");
            double ax = a.HasValue ? a.Value.Num("x") ?? -1 : -1, ay = a.HasValue ? a.Value.Num("y") ?? -1 : -1;
            var kids = n.Arr("children");
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Other(i.ToString()));
                if (k == null) continue;
                bool positioned = kids[i].Str("type") == "positioned";
                if (!positioned && k is FrameworkElement fe)
                {
                    if (fe.HorizontalAlignment == HorizontalAlignment.Stretch) fe.HorizontalAlignment = HAlign(ax);
                    if (fe.VerticalAlignment == VerticalAlignment.Stretch) fe.VerticalAlignment = VAlign(ay);
                }
                g.Children.Add(k);
            }
            FlexPanel.SetFillCross(g, n.Str("fit") == "expand");
            return g;
        }

        static UIElement Positioned(JsonElement n, Ctx c)
        {
            var child = Child(n, c) ?? Empty();
            var fe = child as FrameworkElement;
            if (fe == null) { var b = new Border { Child = child }; fe = b; child = b; }
            double? l = n.Num("left"), t = n.Num("top"), r = n.Num("right"), bt = n.Num("bottom"), w = n.Num("width"), h = n.Num("height");
            if (w.HasValue) fe.Width = w.Value;
            if (h.HasValue) fe.Height = h.Value;
            fe.Margin = new Thickness(l ?? 0, t ?? 0, r ?? 0, bt ?? 0);
            fe.HorizontalAlignment = l.HasValue && r.HasValue && !w.HasValue ? HorizontalAlignment.Stretch : (l.HasValue ? HorizontalAlignment.Left : (r.HasValue ? HorizontalAlignment.Right : HorizontalAlignment.Center));
            fe.VerticalAlignment = t.HasValue && bt.HasValue && !h.HasValue ? VerticalAlignment.Stretch : (t.HasValue ? VerticalAlignment.Top : (bt.HasValue ? VerticalAlignment.Bottom : VerticalAlignment.Center));
            return child;
        }

        static UIElement Divider(JsonElement n)
        {
            double th = n.Num("thickness") ?? 1;
            var brush = Theme.Solid(Color.FromArgb(70, Theme.Outline.R, Theme.Outline.G, Theme.Outline.B));
            if (n.Str("dir") == "vertical") return new Border { Width = th, Background = brush, VerticalAlignment = VerticalAlignment.Stretch };
            var d = new Border { Height = th, Background = brush };
            FlexPanel.SetFillCross(d, true);
            return d;
        }

        static UIElement Scroll(JsonElement n, Ctx c)
        {
            bool horizontal = n.Str("dir") == "horizontal";
            var inner = new Border { Padding = Insets(n.Obj("padding")), Child = Node(n.Obj("child"), c.Sub("s", col: !horizontal, row: horizontal)) };
            if (n.Bool("noScroll") == true) return inner;
            var sv = new ScrollViewer
            {
                Content = inner,
                VerticalScrollBarVisibility = horizontal ? ScrollBarVisibility.Disabled : ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility = horizontal ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled,
            };
            TrackScroll(sv, c, n.Int("controller"), null);
            return sv;
        }

        static void TrackScroll(ScrollViewer sv, Ctx c, int? controller, double? extent)
        {
            string path = c.Path;
            if (c.State.ScrollOffsets.TryGetValue(path, out var saved))
                sv.Loaded += (s, e) => sv.ScrollToVerticalOffset(saved);
            sv.ScrollChanged += (s, e) => { if (!c.State.Rebuilding) c.State.ScrollOffsets[path] = sv.VerticalOffset; };
            if (controller.HasValue)
            {
                c.Host.Scrollers[controller.Value] = sv;
                if (extent.HasValue) c.Host.ScrollExtents[controller.Value] = extent.Value;
            }
        }

        static UIElement List(JsonElement n, Ctx c)
        {
            bool horizontal = n.Str("dir") == "horizontal";
            var kids = n.Arr("children");
            double? extent = n.Num("itemExtent");
            var panel = new StackPanel { Orientation = horizontal ? Orientation.Horizontal : Orientation.Vertical };
            int total = n.Int("total") ?? -1;
            int offset = n.Int("offset") ?? 0;
            string onWindow = n.Str("onWindow");
            bool virtualised = total >= 0 && onWindow != null && extent.HasValue && !horizontal;
            if (virtualised && offset > 0) panel.Children.Add(new Border { Height = offset * extent.Value });
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Sub(i.ToString(), col: !horizontal, row: horizontal));
                if (k == null) continue;
                if (extent.HasValue)
                {
                    var holder = new Border { Child = k };
                    if (horizontal) holder.Width = extent.Value; else holder.Height = extent.Value;
                    panel.Children.Add(holder);
                }
                else panel.Children.Add(k);
            }
            if (virtualised)
            {
                int after = total - offset - kids.Count;
                if (after > 0) panel.Children.Add(new Border { Height = after * extent.Value });
            }
            var content = new Border { Padding = Insets(n.Obj("padding")), Child = panel };
            if (n.Bool("noScroll") == true || n.Bool("shrinkWrap") == true) return content;
            var sv = new ScrollViewer
            {
                Content = content,
                VerticalScrollBarVisibility = horizontal ? ScrollBarVisibility.Disabled : ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility = horizontal ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled,
            };
            TrackScroll(sv, c, n.Int("controller"), extent);
            if (virtualised)
            {
                int count = kids.Count;
                double ext = extent.Value;
                string lastFired = null;
                sv.ScrollChanged += (s, e) =>
                {
                    int first = (int)(sv.VerticalOffset / ext);
                    int last = (int)((sv.VerticalOffset + sv.ViewportHeight) / ext);
                    bool needs = (first < offset + 4 && offset > 0) || (last > offset + count - 5 && offset + count < total);
                    if (!needs) { lastFired = null; return; }
                    string key = onWindow + ":" + first;
                    if (key == lastFired) return;
                    lastFired = key;
                    c.Host.Dispatch(key);
                };
            }
            string onEnd = n.Str("onEnd");
            if (onEnd != null && !virtualised)
            {
                bool fired = false;
                sv.ScrollChanged += (s, e) =>
                {
                    bool atEnd = sv.VerticalOffset >= sv.ScrollableHeight - 2 * (extent ?? 40);
                    if (atEnd && !fired) { fired = true; c.Host.Dispatch(onEnd); }
                    if (!atEnd) fired = false;
                };
            }
            return sv;
        }

        static UIElement GridView(JsonElement n, Ctx c)
        {
            var p = new GridPanel
            {
                Columns = n.Int("columns") ?? 0,
                MaxExtent = n.Num("maxExtent") ?? 120,
                MainSpacing = n.Num("mainSpacing") ?? 0,
                CrossSpacing = n.Num("crossSpacing") ?? 0,
                Ratio = n.Num("ratio") ?? 1,
            };
            var kids = n.Arr("children");
            for (int i = 0; i < kids.Count; i++)
            {
                var k = Node(kids[i], c.Other(i.ToString()));
                if (k != null) p.Children.Add(k);
            }
            var sv = new ScrollViewer
            {
                Content = new Border { Padding = Insets(n.Obj("padding")), Child = p },
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            };
            TrackScroll(sv, c, null, null);
            return sv;
        }

        static UIElement Pager(JsonElement n, Ctx c)
        {
            var kids = n.Arr("children");
            if (kids.Count == 0) return Empty();
            int index = Math.Max(0, Math.Min(kids.Count - 1, n.Int("index") ?? 0));
            var host = new Grid();
            var page = Node(kids[index], c.Other("p" + index));
            if (page != null) host.Children.Add(page);
            string onChanged = n.Str("onChanged");
            host.MouseWheel += (s, e) =>
            {
                if (n.Bool("noScroll") == true || onChanged == null || !(Keyboard.Modifiers.HasFlag(ModifierKeys.Shift))) return;
                int next = Math.Max(0, Math.Min(kids.Count - 1, index + (e.Delta < 0 ? 1 : -1)));
                if (next != index) c.Fire(onChanged, next);
                e.Handled = true;
            };
            return host;
        }
    }
}
