using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace DragonRuntime
{
    internal sealed class FlexPanel : Panel
    {
        public static readonly DependencyProperty FlexProperty = DependencyProperty.RegisterAttached("Flex", typeof(double), typeof(FlexPanel), new PropertyMetadata(0.0));
        public static readonly DependencyProperty TightProperty = DependencyProperty.RegisterAttached("Tight", typeof(bool), typeof(FlexPanel), new PropertyMetadata(true));
        public static readonly DependencyProperty FillCrossProperty = DependencyProperty.RegisterAttached("FillCross", typeof(bool), typeof(FlexPanel), new PropertyMetadata(false));

        public static double GetFlex(DependencyObject o) => (double)o.GetValue(FlexProperty);
        public static void SetFlex(DependencyObject o, double v) => o.SetValue(FlexProperty, v);
        public static bool GetTight(DependencyObject o) => (bool)o.GetValue(TightProperty);
        public static void SetTight(DependencyObject o, bool v) => o.SetValue(TightProperty, v);
        public static bool GetFillCross(DependencyObject o) => (bool)o.GetValue(FillCrossProperty);
        public static void SetFillCross(DependencyObject o, bool v) => o.SetValue(FillCrossProperty, v);

        public bool Vertical { get; set; }
        public string Main { get; set; } = "start";
        public string Cross { get; set; } = "center";
        public double Spacing { get; set; }
        public bool FillMain { get; set; }

        double MainOf(Size s) => Vertical ? s.Height : s.Width;
        double CrossOf(Size s) => Vertical ? s.Width : s.Height;
        Size Make(double main, double cross) => Vertical ? new Size(cross, main) : new Size(main, cross);

        List<UIElement> Visible()
        {
            var list = new List<UIElement>();
            foreach (UIElement c in InternalChildren) if (c.Visibility != Visibility.Collapsed) list.Add(c);
            return list;
        }

        protected override Size MeasureOverride(Size available)
        {
            var kids = Visible();
            double mainAvail = MainOf(available), crossAvail = CrossOf(available);
            double used = 0, maxCross = 0, flexSum = 0;
            int flexCount = 0;
            foreach (var k in kids)
            {
                double f = GetFlex(k);
                if (f > 0) { flexSum += f; flexCount++; continue; }
                k.Measure(Make(double.PositiveInfinity, crossAvail));
                used += MainOf(k.DesiredSize);
                maxCross = Math.Max(maxCross, CrossOf(k.DesiredSize));
            }
            used += Spacing * Math.Max(0, kids.Count - 1);
            double flexUsed = 0;
            foreach (var k in kids)
            {
                double f = GetFlex(k);
                if (f <= 0) continue;
                if (double.IsInfinity(mainAvail))
                {
                    k.Measure(Make(double.PositiveInfinity, crossAvail));
                }
                else
                {
                    double share = Math.Max(0, mainAvail - used) * f / flexSum;
                    k.Measure(Make(share, crossAvail));
                }
                flexUsed += MainOf(k.DesiredSize);
                maxCross = Math.Max(maxCross, CrossOf(k.DesiredSize));
            }
            double main = used + flexUsed;
            if (!double.IsInfinity(mainAvail) && (FillMain || flexCount > 0 && HasTight(kids))) main = mainAvail;
            double cross = maxCross;
            if (!double.IsInfinity(crossAvail) && (Cross == "stretch" || AnyFillCross(kids))) cross = crossAvail;
            return Make(double.IsInfinity(main) ? 0 : main, cross);
        }

        static bool HasTight(List<UIElement> kids)
        {
            foreach (var k in kids) if (GetFlex(k) > 0 && GetTight(k)) return true;
            return false;
        }

        static bool AnyFillCross(List<UIElement> kids)
        {
            foreach (var k in kids) if (GetFillCross(k)) return true;
            return false;
        }

        protected override Size ArrangeOverride(Size final)
        {
            var kids = Visible();
            double mainFinal = MainOf(final), crossFinal = CrossOf(final);
            double fixedSum = 0, flexSum = 0;
            foreach (var k in kids)
            {
                double f = GetFlex(k);
                if (f > 0) flexSum += f; else fixedSum += MainOf(k.DesiredSize);
            }
            double spacingTotal = Spacing * Math.Max(0, kids.Count - 1);
            double remaining = Math.Max(0, mainFinal - fixedSum - spacingTotal);
            var sizes = new double[kids.Count];
            double total = spacingTotal;
            for (int i = 0; i < kids.Count; i++)
            {
                double f = GetFlex(kids[i]);
                if (f > 0)
                {
                    double share = remaining * f / flexSum;
                    sizes[i] = GetTight(kids[i]) ? share : Math.Min(MainOf(kids[i].DesiredSize), share);
                }
                else sizes[i] = MainOf(kids[i].DesiredSize);
                total += sizes[i];
            }
            double free = Math.Max(0, mainFinal - total);
            double offset = 0, gap = Spacing;
            bool hasFlex = flexSum > 0;
            if (!hasFlex)
            {
                int n = kids.Count;
                switch (Main)
                {
                    case "end": offset = free; break;
                    case "center": offset = free / 2; break;
                    case "spaceBetween": if (n > 1) gap += free / (n - 1); break;
                    case "spaceAround": if (n > 0) { double s = free / n; offset = s / 2; gap += s; } break;
                    case "spaceEvenly": if (n > 0) { double s = free / (n + 1); offset = s; gap += s; } break;
                }
            }
            for (int i = 0; i < kids.Count; i++)
            {
                var k = kids[i];
                double want = CrossOf(k.DesiredSize);
                double cs = (Cross == "stretch" || GetFillCross(k)) ? crossFinal : Math.Min(want, crossFinal);
                double co = 0;
                if (cs < crossFinal)
                {
                    if (Cross == "center") co = (crossFinal - cs) / 2;
                    else if (Cross == "end") co = crossFinal - cs;
                }
                if (GetFlex(k) > 0) k.Measure(Make(sizes[i], crossFinal));
                var rect = Vertical ? new Rect(co, offset, cs, sizes[i]) : new Rect(offset, co, sizes[i], cs);
                k.Arrange(rect);
                offset += sizes[i] + gap;
            }
            return final;
        }
    }

    internal sealed class FractionalPanel : Panel
    {
        public double WidthFactor { get; set; } = 1;
        public double? HeightFactor { get; set; }

        protected override Size MeasureOverride(Size available)
        {
            double w = double.IsInfinity(available.Width) ? double.PositiveInfinity : available.Width * WidthFactor;
            double h = HeightFactor.HasValue && !double.IsInfinity(available.Height) ? available.Height * HeightFactor.Value : available.Height;
            double maxW = 0, maxH = 0;
            foreach (UIElement c in InternalChildren)
            {
                c.Measure(new Size(w, h));
                maxW = Math.Max(maxW, c.DesiredSize.Width);
                maxH = Math.Max(maxH, c.DesiredSize.Height);
            }
            return new Size(double.IsInfinity(w) ? maxW : w, maxH);
        }

        protected override Size ArrangeOverride(Size final)
        {
            foreach (UIElement c in InternalChildren) c.Arrange(new Rect(0, 0, final.Width, final.Height));
            return final;
        }
    }

    internal sealed class AspectPanel : Panel
    {
        public double Ratio { get; set; } = 1;

        protected override Size MeasureOverride(Size available)
        {
            double w = available.Width, h = available.Height;
            if (double.IsInfinity(w) && double.IsInfinity(h)) { w = 100; }
            if (double.IsInfinity(w)) w = h * Ratio;
            double height = w / Ratio;
            if (!double.IsInfinity(h) && height > h) { height = h; w = h * Ratio; }
            foreach (UIElement c in InternalChildren) c.Measure(new Size(w, height));
            return new Size(w, height);
        }

        protected override Size ArrangeOverride(Size final)
        {
            foreach (UIElement c in InternalChildren) c.Arrange(new Rect(0, 0, final.Width, final.Height));
            return final;
        }
    }

    internal sealed class GridPanel : Panel
    {
        public int Columns { get; set; }
        public double MaxExtent { get; set; } = 120;
        public double MainSpacing { get; set; }
        public double CrossSpacing { get; set; }
        public double Ratio { get; set; } = 1;

        int ColumnsFor(double width)
        {
            if (Columns > 0) return Columns;
            double min = MaxExtent * 0.75;
            return Math.Max(1, (int)Math.Floor((width + CrossSpacing) / (min + CrossSpacing)));
        }

        protected override Size MeasureOverride(Size available)
        {
            double width = double.IsInfinity(available.Width) ? 360 : available.Width;
            int cols = ColumnsFor(width);
            double cell = (width - CrossSpacing * (cols - 1)) / cols;
            double cellH = cell / Ratio;
            int rows = (int)Math.Ceiling(InternalChildren.Count / (double)cols);
            foreach (UIElement c in InternalChildren) c.Measure(new Size(cell, cellH));
            return new Size(width, rows * cellH + Math.Max(0, rows - 1) * MainSpacing);
        }

        protected override Size ArrangeOverride(Size final)
        {
            int cols = ColumnsFor(final.Width);
            double cell = (final.Width - CrossSpacing * (cols - 1)) / cols;
            double cellH = cell / Ratio;
            int i = 0;
            foreach (UIElement c in InternalChildren)
            {
                int r = i / cols, col = i % cols;
                c.Arrange(new Rect(col * (cell + CrossSpacing), r * (cellH + MainSpacing), cell, cellH));
                i++;
            }
            return final;
        }
    }

    internal sealed class CanvasView : FrameworkElement
    {
        readonly List<JsonElement> ops;
        public CanvasView(List<JsonElement> ops, double w, double h)
        {
            this.ops = ops;
            Width = w;
            Height = h;
        }

        static double N(JsonElement e, string k) => e.Num(k) ?? 0;

        protected override void OnRender(DrawingContext dc)
        {
            foreach (var o in ops)
            {
                var paint = o.Obj("paint");
                Color col = paint.HasValue ? Theme.ParseColor(paint.Value.Str("color")) ?? Colors.Black : Colors.Black;
                bool stroke = paint.HasValue && paint.Value.Str("style") == "stroke";
                double sw = paint.HasValue ? paint.Value.Num("width") ?? 1 : 1;
                var brush = new SolidColorBrush(col);
                var pen = new Pen(brush, sw);
                if (paint.HasValue && paint.Value.Str("cap") == "round") { pen.StartLineCap = PenLineCap.Round; pen.EndLineCap = PenLineCap.Round; }
                Brush fill = stroke ? null : brush;
                Pen line = stroke ? pen : null;
                switch (o.Str("op"))
                {
                    case "line": dc.DrawLine(pen, new Point(N(o, "x1"), N(o, "y1")), new Point(N(o, "x2"), N(o, "y2"))); break;
                    case "rect": dc.DrawRectangle(fill, line, new Rect(N(o, "x"), N(o, "y"), N(o, "w"), N(o, "h"))); break;
                    case "rrect":
                        double r = o.Obj("radius").HasValue ? N(o.Obj("radius").Value, "tl") : 0;
                        dc.DrawRoundedRectangle(fill, line, new Rect(N(o, "x"), N(o, "y"), N(o, "w"), N(o, "h")), r, r);
                        break;
                    case "circle": dc.DrawEllipse(fill, line, new Point(N(o, "x"), N(o, "y")), N(o, "r"), N(o, "r")); break;
                    case "oval":
                        dc.DrawEllipse(fill, line, new Point(N(o, "x") + N(o, "w") / 2, N(o, "y") + N(o, "h") / 2), N(o, "w") / 2, N(o, "h") / 2);
                        break;
                    case "path": dc.DrawGeometry(fill, line, PathGeometry(o)); break;
                    case "text":
                        var st = o.Obj("style");
                        var tcol = st.HasValue ? Theme.ParseColor(st.Value.Str("color")) ?? col : col;
                        var ft = new FormattedText(o.Str("text") ?? "", System.Globalization.CultureInfo.CurrentUICulture, FlowDirection.LeftToRight,
                            new Typeface("Segoe UI"), st.HasValue ? st.Value.Num("size") ?? 14 : 14, new SolidColorBrush(tcol), VisualTreeHelper.GetDpi(this).PixelsPerDip);
                        dc.DrawText(ft, new Point(N(o, "x"), N(o, "y")));
                        break;
                }
            }
        }

        static Geometry PathGeometry(JsonElement o)
        {
            var g = new StreamGeometry();
            using (var ctx = g.Open())
            {
                bool open = false;
                foreach (var cmd in o.Arr("cmds"))
                {
                    if (cmd.ValueKind != JsonValueKind.Array) continue;
                    var a = new List<JsonElement>();
                    foreach (var x in cmd.EnumerateArray()) a.Add(x);
                    if (a.Count == 0) continue;
                    double V(int i) => a[i].GetDouble();
                    switch (a[0].GetString())
                    {
                        case "M": ctx.BeginFigure(new Point(V(1), V(2)), true, false); open = true; break;
                        case "L": if (open) ctx.LineTo(new Point(V(1), V(2)), true, false); break;
                        case "Q": if (open) ctx.QuadraticBezierTo(new Point(V(1), V(2)), new Point(V(3), V(4)), true, false); break;
                        case "C": if (open) ctx.BezierTo(new Point(V(1), V(2)), new Point(V(3), V(4)), new Point(V(5), V(6)), true, false); break;
                    }
                }
            }
            return g;
        }
    }
}
