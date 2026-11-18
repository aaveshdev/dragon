using System;
using TextElement = System.Windows.Documents.TextElement;
using Run = System.Windows.Documents.Run;
using Typography = System.Windows.Documents.Typography;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Shapes;
using System.Windows.Threading;

namespace DragonRuntime
{
    internal static partial class R
    {
        static UIElement Gesture(JsonElement n, Ctx c)
        {
            string onTap = n.Str("onTap"), onDouble = n.Str("onDoubleTap"), onLong = n.Str("onLongPress"), onTapDown = n.Str("onTapDown");
            string onPanStart = n.Str("onPanStart"), onPanUpdate = n.Str("onPanUpdate"), onPanEnd = n.Str("onPanEnd");
            string onH = n.Str("onHDrag"), onHEnd = n.Str("onHDragEnd"), onV = n.Str("onVDrag"), onVEnd = n.Str("onVDragEnd");
            var hit = new Border { Background = Brushes.Transparent, Child = Child(n, c) };
            if (n.Obj("radius").HasValue) { hit.CornerRadius = Radius(n.Obj("radius")); ClipRounded(hit, false); }
            if (onTap != null || onDouble != null || onLong != null) hit.Cursor = Cursors.Hand;
            var g = c.State.Pan(c.Path);
            var inv0 = System.Globalization.CultureInfo.InvariantCulture;
            DispatcherTimer longTimer = null;
            bool longFired = false;
            bool wantsDrag = onPanStart != null || onPanUpdate != null || onPanEnd != null || onH != null || onHEnd != null || onV != null || onVEnd != null;
            Action finishDrag = () =>
            {
                bool wasDragging = g.Dragging && g.Moved;
                g.Dragging = false;
                if (hit.IsMouseCaptured) hit.ReleaseMouseCapture();
                if (wasDragging)
                {
                    if (onPanEnd != null) c.Fire(onPanEnd);
                    if (onHEnd != null) c.Fire(onHEnd);
                    if (onVEnd != null) c.Fire(onVEnd);
                }
            };
            hit.MouseLeftButtonDown += (s, e) =>
            {
                var start = e.GetPosition(hit);
                g.Start = start; g.Last = start; g.Moved = false;
                longFired = false;
                if (onTapDown != null) c.Fire(onTapDown, start.X.ToString(inv0) + "," + start.Y.ToString(inv0));
                if (onLong != null)
                {
                    longTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(500) };
                    longTimer.Tick += (a2, b2) => { longTimer.Stop(); if (!g.Moved) { longFired = true; c.Fire(onLong); } };
                    longTimer.Start();
                }
                if (wantsDrag) { g.Dragging = true; hit.CaptureMouse(); }
            };
            hit.MouseMove += (s, e) =>
            {
                if (!g.Dragging) return;
                // the control may have been rebuilt since the press: carry on with the new one
                if (e.LeftButton != MouseButtonState.Pressed) { finishDrag(); return; }
                if (!hit.IsMouseCaptured) hit.CaptureMouse();
                var p = e.GetPosition(hit);
                if (!g.Moved && (Math.Abs(p.X - g.Start.X) > 4 || Math.Abs(p.Y - g.Start.Y) > 4))
                {
                    g.Moved = true;
                    longTimer?.Stop();
                    if (onPanStart != null) c.Fire(onPanStart, g.Start.X.ToString(inv0) + "," + g.Start.Y.ToString(inv0));
                }
                if (!g.Moved) return;
                if (onPanUpdate != null) c.Fire(onPanUpdate, p.X.ToString(inv0) + "," + p.Y.ToString(inv0));
                if (onH != null) c.Fire(onH, (p.X - g.Last.X).ToString(inv0));
                if (onV != null) c.Fire(onV, (p.Y - g.Last.Y).ToString(inv0));
                g.Last = p;
            };
            hit.MouseLeftButtonUp += (s, e) =>
            {
                longTimer?.Stop();
                bool wasDragging = g.Dragging && g.Moved;
                if (g.Dragging) finishDrag();
                if (wasDragging) return;
                if (longFired) return;
                if (e.ClickCount >= 2 && onDouble != null) c.Fire(onDouble);
                else if (onTap != null) c.Fire(onTap);
            };
            return hit;
        }

        static UIElement Button(JsonElement n, Ctx c)
        {
            string variant = n.Str("variant") ?? "text";
            bool enabled = n.Bool("enabled") ?? true;
            string onPressed = n.Str("onPressed");
            var fgOverride = Theme.BrushOf(n.Str("fg"));
            var bgOverride = Theme.BrushOf(n.Str("bg"));
            bool isFab = variant.StartsWith("fab", StringComparison.Ordinal);
            bool isIcon = variant.StartsWith("icon", StringComparison.Ordinal);

            Brush bg = bgOverride, fg = fgOverride;
            Brush border = Theme.BrushOf(n.Str("borderColor"));
            switch (variant)
            {
                case "filled":
                case "elevated":
                    bg = bg ?? (variant == "filled" ? Theme.Solid(Theme.Primary) : Theme.Solid(Theme.SurfaceVariant));
                    fg = fg ?? (variant == "filled" ? Theme.Solid(Theme.OnPrimary) : Theme.Solid(Theme.Primary));
                    break;
                case "tonal":
                    bg = bg ?? Theme.Solid(Theme.PrimaryContainer);
                    fg = fg ?? Theme.Solid(Theme.OnSurface);
                    break;
                case "outlined":
                    border = border ?? Theme.Solid(Theme.Outline);
                    fg = fg ?? Theme.Solid(Theme.Primary);
                    break;
                default:
                    if (isFab) { bg = bg ?? Theme.Solid(Theme.PrimaryContainer); fg = fg ?? Theme.Solid(Theme.OnSurface); }
                    else fg = fg ?? Theme.Solid(Theme.Primary);
                    break;
            }
            var content = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
            var child = Node(n.Obj("child"), c.Sub("b", row: true));
            if (child != null) content.Children.Add(child);
            var label = Node(n.Obj("label"), c.Sub("l", row: true));
            if (label != null)
            {
                if (child != null && label is FrameworkElement lf) lf.Margin = new Thickness(8, 0, 0, 0);
                content.Children.Add(label);
            }
            var box = new Border
            {
                Child = content,
                Background = bg ?? Brushes.Transparent,
                CornerRadius = new CornerRadius(isFab ? 16 : (isIcon ? 20 : 20)),
                Padding = n.Obj("padding").HasValue ? Insets(n.Obj("padding")) : (isIcon ? new Thickness(8) : (isFab ? new Thickness(16) : new Thickness(20, 10, 20, 10))),
                Cursor = enabled ? Cursors.Hand : Cursors.Arrow,
                Opacity = enabled ? 1 : 0.4,
            };
            if (fg != null) TextElement.SetForeground(box, fg);
            if (border != null) { box.BorderBrush = border; box.BorderThickness = new Thickness(n.Num("borderWidth") ?? 1); }
            if (n.Obj("shape").HasValue)
            {
                var shape = n.Obj("shape").Value;
                if (shape.Str("kind") == "circle" || shape.Str("kind") == "stadium") box.CornerRadius = new CornerRadius(1000);
                else if (shape.Obj("radius").HasValue) box.CornerRadius = Radius(shape.Obj("radius"));
            }
            if (variant == "elevated" || isFab) box.Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 8, ShadowDepth = 2, Opacity = 0.25, Direction = 270 };
            double? w = n.Num("width"), h = n.Num("height");
            if (w.HasValue && w.Value < INF) box.Width = w.Value;
            if (w.HasValue && w.Value >= INF) FlexPanel.SetFillCross(box, true);
            if (h.HasValue) box.Height = h.Value;
            if (n.Num("minWidth").HasValue) box.MinWidth = n.Num("minWidth").Value;
            if (n.Num("minHeight").HasValue) box.MinHeight = n.Num("minHeight").Value;
            box.HorizontalAlignment = w.HasValue && w.Value >= INF ? HorizontalAlignment.Stretch : HorizontalAlignment.Left;
            box.VerticalAlignment = VerticalAlignment.Center;
            if (enabled)
            {
                var hover = Theme.Solid(Color.FromArgb(28, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B));
                Brush normal = box.Background;
                box.MouseEnter += (s, e) => { if (bg == null) box.Background = hover; else box.Opacity = 0.92; };
                box.MouseLeave += (s, e) => { box.Background = normal; box.Opacity = 1; };
                box.MouseLeftButtonUp += (s, e) => { c.Fire(onPressed); e.Handled = true; };
            }
            return box;
        }

        static UIElement Check(JsonElement n, Ctx c)
        {
            var cb = new CheckBox { IsChecked = n.Bool("value") ?? false, IsEnabled = n.Bool("enabled") ?? true, VerticalAlignment = VerticalAlignment.Center };
            string onChanged = n.Str("onChanged");
            cb.Click += (s, e) => c.Fire(onChanged, cb.IsChecked == true);
            return cb;
        }

        static UIElement Switch(JsonElement n, Ctx c)
        {
            bool on = n.Bool("value") ?? false;
            bool enabled = n.Bool("enabled") ?? true;
            var track = new Border
            {
                Width = 44,
                Height = 24,
                CornerRadius = new CornerRadius(12),
                Background = on ? Theme.Solid(Theme.Primary) : Theme.Solid(Theme.SurfaceVariant),
                BorderBrush = Theme.Solid(Theme.Outline),
                BorderThickness = new Thickness(on ? 0 : 2),
                Cursor = enabled ? Cursors.Hand : Cursors.Arrow,
                Opacity = enabled ? 1 : 0.4,
                VerticalAlignment = VerticalAlignment.Center,
                HorizontalAlignment = HorizontalAlignment.Left,
            };
            var thumb = new Ellipse
            {
                Width = on ? 18 : 12,
                Height = on ? 18 : 12,
                Fill = on ? Theme.Solid(Theme.OnPrimary) : Theme.Solid(Theme.Outline),
                HorizontalAlignment = on ? HorizontalAlignment.Right : HorizontalAlignment.Left,
                Margin = new Thickness(on ? 0 : 4, 0, on ? 3 : 0, 0),
                VerticalAlignment = VerticalAlignment.Center,
            };
            track.Child = thumb;
            string onChanged = n.Str("onChanged");
            if (enabled) track.MouseLeftButtonUp += (s, e) => c.Fire(onChanged, !on);
            return track;
        }

        static UIElement Radio(JsonElement n, Ctx c)
        {
            var rb = new RadioButton { IsChecked = n.Bool("selected") ?? false, IsEnabled = n.Bool("enabled") ?? true, GroupName = c.Path, VerticalAlignment = VerticalAlignment.Center };
            string onChanged = n.Str("onChanged");
            rb.Click += (s, e) => c.Fire(onChanged);
            return rb;
        }

        static UIElement SliderNode(JsonElement n, Ctx c)
        {
            double min = n.Num("min") ?? 0, max = n.Num("max") ?? 1;
            var sl = new Slider { Minimum = min, Maximum = max, Value = Math.Max(min, Math.Min(max, n.Num("value") ?? min)), IsEnabled = n.Bool("enabled") ?? true, MinWidth = 120 };
            int div = n.Int("divisions") ?? 0;
            if (div > 1) { sl.TickFrequency = (max - min) / div; sl.IsSnapToTickEnabled = true; }
            string onChanged = n.Str("onChanged"), onEnd = n.Str("onChangeEnd");
            bool ready = false;
            sl.Loaded += (s, e) => ready = true;
            sl.ValueChanged += (s, e) => { if (ready && !c.State.Rebuilding) c.Fire(onChanged, e.NewValue); };
            sl.PreviewMouseLeftButtonUp += (s, e) => { if (onEnd != null) c.Fire(onEnd); };
            FlexPanel.SetFillCross(sl, false);
            return sl;
        }

        static UIElement Progress(JsonElement n)
        {
            double? v = n.Num("value");
            var col = Theme.BrushOf(n.Str("color")) ?? Theme.Solid(Theme.Primary);
            if (n.Str("kind") == "linear")
            {
                var pb = new ProgressBar { Minimum = 0, Maximum = 1, Height = 4, Foreground = col, IsIndeterminate = !v.HasValue };
                if (v.HasValue) pb.Value = Math.Max(0, Math.Min(1, v.Value));
                FlexPanel.SetFillCross(pb, true);
                return pb;
            }
            double size = 32, stroke = n.Num("stroke") ?? 4;
            var ring = new Ellipse { Width = size, Height = size, Stroke = col, StrokeThickness = stroke, StrokeDashArray = new DoubleCollection { v.HasValue ? v.Value * Math.PI * (size - stroke) / stroke : 2.2, 100 }, StrokeDashCap = PenLineCap.Round, RenderTransformOrigin = new Point(0.5, 0.5) };
            if (v.HasValue) { ring.RenderTransform = new RotateTransform(-90); return ring; }
            var rot = new RotateTransform();
            ring.RenderTransform = rot;
            rot.BeginAnimation(RotateTransform.AngleProperty, new DoubleAnimation(0, 360, TimeSpan.FromSeconds(1)) { RepeatBehavior = RepeatBehavior.Forever });
            return ring;
        }

        static UIElement Dropdown(JsonElement n, Ctx c)
        {
            var items = n.Arr("items");
            int selected = n.Int("selected") ?? -1;
            var cb = new ComboBox { IsEnabled = n.Bool("enabled") ?? true, MinWidth = 120 };
            for (int i = 0; i < items.Count; i++)
            {
                var content = Node(items[i].Obj("label"), c.Sub("i" + i, row: true)) ?? new TextBlock();
                cb.Items.Add(new ComboBoxItem { Content = content, IsEnabled = items[i].Bool("enabled") ?? true });
            }
            cb.SelectedIndex = selected;
            string onChanged = n.Str("onChanged");
            cb.SelectionChanged += (s, e) =>
            {
                if (c.State.Rebuilding || !cb.IsLoaded) return;
                if (cb.SelectedIndex >= 0 && cb.SelectedIndex != selected) c.Fire(onChanged, cb.SelectedIndex);
            };
            var grid = new Grid();
            grid.Children.Add(cb);
            var hint = Node(n.Obj("hint"), c.Sub("h", row: true));
            if (hint != null && selected < 0)
            {
                if (hint is FrameworkElement hf) { hf.IsHitTestVisible = false; hf.Margin = new Thickness(8, 0, 24, 0); hf.VerticalAlignment = VerticalAlignment.Center; hf.HorizontalAlignment = HorizontalAlignment.Left; }
                grid.Children.Add(hint);
            }
            if (n.Bool("expanded") == true) FlexPanel.SetFillCross(grid, true);
            else grid.HorizontalAlignment = HorizontalAlignment.Left;
            return grid;
        }

        static UIElement PopupMenu(JsonElement n, Ctx c)
        {
            var child = Node(n.Obj("child"), c.Sub("m", row: true)) ?? new TextBlock { Text = "..." };
            var host = new Border { Child = child, Background = Brushes.Transparent, Cursor = Cursors.Hand };
            string onSelected = n.Str("onSelected");
            var menu = new ContextMenu();
            var items = n.Arr("items");
            for (int i = 0; i < items.Count; i++)
            {
                if (items[i].Bool("divider") == true) { menu.Items.Add(new Separator()); continue; }
                int idx = i;
                var mi = new MenuItem { Header = Node(items[i].Obj("label"), c.Sub("mi" + i, row: true)), IsEnabled = items[i].Bool("enabled") ?? true };
                mi.Click += (s, e) => c.Fire(onSelected, idx);
                menu.Items.Add(mi);
            }
            host.MouseLeftButtonUp += (s, e) =>
            {
                if (!(n.Bool("enabled") ?? true)) return;
                menu.PlacementTarget = host;
                menu.Placement = PlacementMode.Bottom;
                menu.IsOpen = true;
            };
            return host;
        }

        static UIElement TextField(JsonElement n, Ctx c)
        {
            string path = c.Path;
            string value = n.Str("value") ?? "";
            bool obscure = n.Bool("obscure") == true;
            bool multi = (n.Int("maxLines") ?? 1) != 1 || n.Str("keyboard") == "multiline";
            string onChanged = n.Str("onChanged"), onSubmitted = n.Str("onSubmitted"), onTap = n.Str("onTap");
            int? maxLength = n.Int("maxLength");
            bool enabled = n.Bool("enabled") ?? true, readOnly = n.Bool("readOnly") ?? false;
            string hintText = n.Str("hint");
            bool noBorder = n.Str("border") == "none";
            bool dense = n.Bool("dense") == true;
            FrameworkElement input;
            TextBox tb = null;
            PasswordBox pw = null;
            if (obscure)
            {
                pw = new PasswordBox { Password = value, BorderThickness = new Thickness(0), Background = Brushes.Transparent, VerticalContentAlignment = VerticalAlignment.Center, IsEnabled = enabled };
                pw.PasswordChanged += (s, e) =>
                {
                    if (c.State.Rebuilding || pw.Password == value) return;
                    c.Fire(onChanged, pw.Password);
                };
                input = pw;
            }
            else
            {
                tb = new TextBox
                {
                    Text = value,
                    BorderThickness = new Thickness(0),
                    Background = Brushes.Transparent,
                    AcceptsReturn = multi,
                    TextWrapping = multi ? TextWrapping.Wrap : TextWrapping.NoWrap,
                    VerticalContentAlignment = VerticalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                    Padding = new Thickness(0),
                    IsEnabled = enabled,
                    IsReadOnly = readOnly,
                    MinLines = n.Int("minLines") ?? 1,
                    MaxLines = multi ? (n.Int("maxLines") ?? 5) : 1,
                    FontFamily = Theme.Text,
                };
                if (maxLength.HasValue) tb.MaxLength = maxLength.Value;
                var st = n.Obj("style");
                if (st.HasValue)
                {
                    var col = Theme.BrushOf(st.Value.Str("color"));
                    if (col != null) tb.Foreground = col;
                    if (st.Value.Num("size").HasValue) tb.FontSize = st.Value.Num("size").Value;
                }
                tb.TextChanged += (s, e) =>
                {
                    if (c.State.Rebuilding || tb.Text == value) return;
                    c.State.CaretStart = tb.CaretIndex;
                    c.Fire(onChanged, tb.Text);
                };
                tb.KeyDown += (s, e) =>
                {
                    if (e.Key == Key.Enter && !multi && onSubmitted != null) { c.Fire(onSubmitted); e.Handled = true; }
                };
                tb.SelectionChanged += (s, e) => { if (!c.State.Rebuilding) { c.State.CaretStart = tb.CaretIndex; c.State.SelectionLength = tb.SelectionLength; } };
                input = tb;
            }
            input.GotKeyboardFocus += (s, e) => { c.State.FocusPath = path; if (onTap != null) c.Fire(onTap); };
            input.LostKeyboardFocus += (s, e) => { if (!c.State.Rebuilding && c.State.FocusPath == path) c.State.FocusPath = null; };
            bool autofocus = n.Bool("autofocus") == true;
            input.Loaded += (s, e) =>
            {
                if (c.State.FocusPath == path || (autofocus && c.State.FocusPath == null && !c.State.ScrollOffsets.ContainsKey("af:" + path)))
                {
                    if (autofocus) c.State.ScrollOffsets["af:" + path] = 1;
                    input.Focus();
                    Keyboard.Focus(input);
                    if (pw != null)
                    {
                        // PasswordBox has no public caret API; without this the caret jumps back to the start
                        // each time the view is rebuilt after a keystroke.
                        try
                        {
                            var sel = typeof(PasswordBox).GetMethod("Select", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
                            sel?.Invoke(pw, new object[] { pw.Password.Length, 0 });
                        }
                        catch { }
                    }
                    if (tb != null)
                    {
                        tb.CaretIndex = Math.Min(c.State.CaretStart, tb.Text.Length);
                        if (c.State.SelectionLength > 0) tb.Select(tb.CaretIndex, Math.Min(c.State.SelectionLength, tb.Text.Length - tb.CaretIndex));
                    }
                }
            };

            var inner = new Grid();
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            var prefix = Node(n.Obj("prefix"), c.Sub("pre", row: true));
            var suffix = Node(n.Obj("suffix"), c.Sub("suf", row: true));
            var middle = new Grid { MinWidth = 100 };
            middle.Children.Add(input);
            if (!string.IsNullOrEmpty(hintText))
            {
                var hint = new TextBlock { Text = hintText, Foreground = Theme.Solid(Color.FromArgb(150, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B)), IsHitTestVisible = false, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(noBorder ? 0 : 4, 0, 0, 0), Visibility = value.Length == 0 ? Visibility.Visible : Visibility.Collapsed };
                middle.Children.Add(hint);
            }
            var dock = new DockPanel();
            if (prefix != null) { DockPanel.SetDock(prefix, Dock.Left); dock.Children.Add(prefix); }
            if (suffix != null) { DockPanel.SetDock(suffix, Dock.Right); dock.Children.Add(suffix); }
            dock.Children.Add(middle);

            string error = n.Str("error");
            bool filled = n.Bool("filled") == true;
            var box = new Border
            {
                Child = dock,
                Padding = noBorder ? new Thickness(0) : dense ? new Thickness(8, 4, 8, 4) : new Thickness(10, 6, 10, 6),
                CornerRadius = new CornerRadius(4),
                BorderThickness = new Thickness(filled || noBorder ? 0 : 1),
                BorderBrush = error != null ? Theme.Solid(Theme.Error) : Theme.Solid(Theme.Outline),
                Background = filled ? (Theme.BrushOf(n.Str("fillColor")) ?? Theme.Solid(Theme.SurfaceVariant)) : Brushes.Transparent,
                MinHeight = noBorder || dense ? 0 : 40,
            };
            var col2 = new StackPanel();
            string label = n.Str("label");
            if (label != null) col2.Children.Add(new TextBlock { Text = label, FontSize = 12, Margin = new Thickness(2, 0, 0, 2), Foreground = Theme.Solid(Color.FromArgb(190, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B)) });
            col2.Children.Add(box);
            string below = error ?? n.Str("helper");
            if (below != null) col2.Children.Add(new TextBlock { Text = below, FontSize = 12, Margin = new Thickness(2, 2, 0, 0), Foreground = error != null ? Theme.Solid(Theme.Error) : Theme.Solid(Theme.Outline) });
            FlexPanel.SetFillCross(col2, true);
            return col2;
        }

        static UIElement Scaffold(JsonElement n, Ctx c)
        {
            var root = new Grid { Background = Theme.BrushOf(n.Str("bg")) ?? Theme.Solid(Theme.Background) };
            var main = new Grid();
            main.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            main.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            main.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var bar = Node(n.Obj("appBar"), c.Sub("bar"));
            if (bar != null) { Grid.SetRow(bar, 0); main.Children.Add(bar); }
            var bodyHost = new Grid { ClipToBounds = true };
            var body = Node(n.Obj("body"), c.Sub("body"));
            if (body != null) bodyHost.Children.Add(body);
            var fab = Node(n.Obj("fab"), c.Sub("fab"));
            if (fab != null && fab is FrameworkElement ff)
            {
                ff.Margin = new Thickness(16);
                switch (n.Str("fabLocation"))
                {
                    case "center": ff.HorizontalAlignment = HorizontalAlignment.Center; break;
                    case "start": ff.HorizontalAlignment = HorizontalAlignment.Left; break;
                    default: ff.HorizontalAlignment = HorizontalAlignment.Right; break;
                }
                ff.VerticalAlignment = VerticalAlignment.Bottom;
                bodyHost.Children.Add(fab);
            }
            var sheet = Node(n.Obj("bottomSheet"), c.Sub("sheet"));
            if (sheet is FrameworkElement sf) { sf.VerticalAlignment = VerticalAlignment.Bottom; bodyHost.Children.Add(sheet); }
            Grid.SetRow(bodyHost, 1);
            main.Children.Add(bodyHost);
            var nav = Node(n.Obj("bottomNav"), c.Sub("nav"));
            if (nav != null) { Grid.SetRow(nav, 2); main.Children.Add(nav); }
            root.Children.Add(main);

            foreach (var side in new[] { "start", "end" })
            {
                var d = side == "start" ? n.Obj("drawer") : n.Obj("endDrawer");
                if (!d.HasValue) continue;
                if (!c.Host.Drawers.TryGetValue(side, out var open) || !open) continue;
                string sideName = side;
                var scrim = new Border { Background = Theme.Solid(Color.FromArgb(100, 0, 0, 0)) };
                scrim.MouseLeftButtonUp += (s, e) => c.Host.SetDrawer(sideName, false);
                root.Children.Add(scrim);
                var dc = c.Sub("drawer" + side);
                dc.CloseSide = side;
                var content = Node(d, dc.Sub("x", col: true));
                var panel = new Border
                {
                    Width = 304,
                    Background = Theme.Solid(Theme.Surface),
                    HorizontalAlignment = side == "start" ? HorizontalAlignment.Left : HorizontalAlignment.Right,
                    Child = new ScrollViewer { Content = content, VerticalScrollBarVisibility = ScrollBarVisibility.Auto },
                    Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 16, ShadowDepth = 0, Opacity = 0.35 },
                };
                var tt = new TranslateTransform(side == "start" ? -304 : 304, 0);
                panel.RenderTransform = tt;
                tt.BeginAnimation(TranslateTransform.XProperty, new DoubleAnimation(0, TimeSpan.FromMilliseconds(180)));
                root.Children.Add(panel);
            }
            return root;
        }

        static UIElement AppBar(JsonElement n, Ctx c)
        {
            var bg = Theme.BrushOf(n.Str("bg")) ?? Theme.Solid(Theme.Surface);
            var fg = Theme.BrushOf(n.Str("fg")) ?? Theme.Solid(Theme.OnSurface);
            var outer = new StackPanel();
            var row = new Grid { Height = n.Num("height") ?? 56 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            UIElement lead = Node(n.Obj("leading"), c.Sub("lead", row: true));
            if (lead == null && n.Str("autoLeading") == "menu") lead = IconButton("menu", fg, () => c.Host.SetDrawer("start", true));
            if (lead == null && n.Str("autoLeading") == "back") { string onBack = n.Str("onBack"); lead = IconButton("arrow_back", fg, () => c.Fire(onBack)); }
            if (lead == null) lead = new Border { Width = 12 };
            Grid.SetColumn(lead, 0);
            row.Children.Add(lead);
            var title = Node(n.Obj("title"), c.Sub("title", row: true));
            if (title is TextBlock tt) { tt.FontSize = 20; tt.VerticalAlignment = VerticalAlignment.Center; }
            if (title is FrameworkElement tf)
            {
                tf.VerticalAlignment = VerticalAlignment.Center;
                tf.HorizontalAlignment = n.Bool("centerTitle") == true ? HorizontalAlignment.Center : HorizontalAlignment.Left;
                tf.Margin = new Thickness(4, 0, 4, 0);
            }
            if (title != null) { Grid.SetColumn(title, 1); row.Children.Add(title); }
            var actions = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
            var acts = n.Arr("actions");
            for (int i = 0; i < acts.Count; i++)
            {
                var a = Node(acts[i], c.Sub("act" + i, row: true));
                if (a != null) actions.Children.Add(a);
            }
            Grid.SetColumn(actions, 2);
            row.Children.Add(actions);
            outer.Children.Add(row);
            var bottom = Node(n.Obj("bottom"), c.Sub("bottom", col: true));
            if (bottom != null) outer.Children.Add(bottom);
            var b = new Border { Child = outer, Background = bg };
            TextElement.SetForeground(b, fg);
            double elev = n.Num("elevation") ?? 0;
            if (elev > 0) b.Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = elev * 2, ShadowDepth = 1, Opacity = 0.25, Direction = 270 };
            Panel.SetZIndex(b, 5);
            return b;
        }

        static UIElement IconButton(string icon, Brush tint, Action click)
        {
            var b = new Border { Child = Icon(icon, 24, tint), Padding = new Thickness(12), Background = Brushes.Transparent, Cursor = Cursors.Hand, CornerRadius = new CornerRadius(20) };
            b.MouseLeftButtonUp += (s, e) => click();
            return b;
        }

        static UIElement NavBar(JsonElement n, Ctx c)
        {
            var items = n.Arr("items");
            int selected = n.Int("selected") ?? 0;
            string onChanged = n.Str("onChanged");
            var row = new UniformGrid { Rows = 1 };
            for (int i = 0; i < items.Count; i++)
            {
                int idx = i;
                bool sel = i == selected;
                var tint = sel ? Theme.Solid(Theme.Primary) : Theme.Solid(Color.FromArgb(170, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B));
                string icon = sel ? (items[i].Str("activeIcon") ?? items[i].Str("icon")) : items[i].Str("icon");
                var col = new StackPanel { HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 8, 0, 8) };
                var ic = Icon(icon ?? "circle", 24, tint);
                if (ic is FrameworkElement fi) fi.HorizontalAlignment = HorizontalAlignment.Center;
                col.Children.Add(ic);
                col.Children.Add(new TextBlock { Text = items[i].Str("label") ?? "", FontSize = 11, Foreground = tint, HorizontalAlignment = HorizontalAlignment.Center });
                var cell = new Border { Child = col, Background = Brushes.Transparent, Cursor = Cursors.Hand };
                cell.MouseLeftButtonUp += (s, e) => c.Fire(onChanged, idx);
                row.Children.Add(cell);
            }
            var bar = new Border { Child = row, Background = Theme.BrushOf(n.Str("bg")) ?? Theme.Solid(Theme.SurfaceVariant) };
            return bar;
        }

        static UIElement TabBar(JsonElement n, Ctx c)
        {
            var labels = n.Arr("labels");
            int selected = n.Int("selected") ?? 0;
            string onChanged = n.Str("onChanged");
            bool scrollable = n.Bool("scrollable") == true;
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            var grid = new UniformGrid { Rows = 1 };
            for (int i = 0; i < labels.Count; i++)
            {
                int idx = i;
                bool sel = i == selected;
                var tint = sel ? Theme.Solid(Theme.Primary) : Theme.Solid(Color.FromArgb(170, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B));
                var inner = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(16, 10, 16, 8) };
                if (labels[i].Str("icon") != null) inner.Children.Add(Icon(labels[i].Str("icon"), 20, tint));
                if (labels[i].Str("text") != null) inner.Children.Add(new TextBlock { Text = labels[i].Str("text"), Foreground = tint, FontWeight = FontWeights.Medium, Margin = new Thickness(labels[i].Str("icon") != null ? 6 : 0, 0, 0, 0) });
                var tab = new Border { Child = inner, BorderBrush = sel ? Theme.Solid(Theme.Primary) : Brushes.Transparent, BorderThickness = new Thickness(0, 0, 0, 2), Background = Brushes.Transparent, Cursor = Cursors.Hand };
                tab.MouseLeftButtonUp += (s, e) => c.Fire(onChanged, idx);
                if (scrollable) row.Children.Add(tab); else grid.Children.Add(tab);
            }
            if (scrollable) return new ScrollViewer { Content = row, HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
            return grid;
        }

        static UIElement ErrorBox(JsonElement n)
        {
            return new Border
            {
                Background = Theme.Solid(Color.FromArgb(210, 183, 28, 28)),
                Padding = new Thickness(8),
                Child = new TextBlock { Text = n.Str("message") ?? "error", Foreground = Brushes.White, FontSize = 12, TextWrapping = TextWrapping.Wrap },
            };
        }

        public static UIElement Overlay(JsonElement o, Ctx c)
        {
            string kind = o.Str("kind") ?? "dialog";
            var content = Node(o.Obj("content"), c.Sub("ov" + (o.Int("id") ?? 0)));
            bool dismissible = o.Bool("dismissible") ?? true;
            string onDismiss = o.Str("onDismiss");
            var root = new Grid();
            if (kind == "snackbar")
            {
                if (content is FrameworkElement sf) { sf.VerticalAlignment = VerticalAlignment.Bottom; sf.HorizontalAlignment = HorizontalAlignment.Center; sf.Margin = new Thickness(16, 16, 16, 32); sf.MaxWidth = 560; }
                if (content != null) root.Children.Add(content);
                return root;
            }
            var scrim = new Border { Background = Theme.BrushOf(o.Str("barrierColor")) ?? Theme.Solid(Color.FromArgb(102, 0, 0, 0)) };
            scrim.MouseLeftButtonUp += (s, e) => { if (dismissible) c.Fire(onDismiss); };
            root.Children.Add(scrim);
            if (content == null) return root;
            if (kind == "dialog")
            {
                var holder = new Border { Child = content, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, MaxWidth = 560, Margin = new Thickness(24), Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 24, ShadowDepth = 4, Opacity = 0.4 } };
                holder.MouseLeftButtonUp += (s, e) => e.Handled = true;
                root.Children.Add(holder);
            }
            else
            {
                var col = new StackPanel();
                if (o.Bool("dragHandle") == true)
                    col.Children.Add(new Border { Width = 32, Height = 4, CornerRadius = new CornerRadius(2), Background = Theme.Solid(Color.FromArgb(100, Theme.OnSurface.R, Theme.OnSurface.G, Theme.OnSurface.B)), Margin = new Thickness(0, 12, 0, 12), HorizontalAlignment = HorizontalAlignment.Center });
                col.Children.Add(content);
                var sheet = new Border
                {
                    Child = col,
                    Background = Theme.BrushOf(o.Str("bg")) ?? Theme.Solid(Theme.Surface),
                    CornerRadius = new CornerRadius(28, 28, 0, 0),
                    VerticalAlignment = VerticalAlignment.Bottom,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    MaxWidth = 640,
                    MaxHeight = 560,
                };
                sheet.MouseLeftButtonUp += (s, e) => e.Handled = true;
                root.Children.Add(sheet);
            }
            return root;
        }
    }
}
