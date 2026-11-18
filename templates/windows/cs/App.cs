using System;
using System.Collections.Generic;
using TextElement = System.Windows.Documents.TextElement;
using Run = System.Windows.Documents.Run;
using Typography = System.Windows.Documents.Typography;
using System.Globalization;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

namespace DragonRuntime
{
    public static class Program
    {
        [STAThread]
        public static int Main(string[] args)
        {
            var app = new Application { ShutdownMode = ShutdownMode.OnMainWindowClose };
            app.DispatcherUnhandledException += (s, e) =>
            {
                MessageBox.Show(e.Exception.ToString(), "Unexpected error");
                e.Handled = true;
            };
            return app.Run(new DragonWindow());
        }
    }

    internal sealed class DragonWindow : Window
    {
        readonly DragonHost host;
        readonly RenderState state = new RenderState();
        readonly Grid shell = new Grid();
        readonly Grid page = new Grid();
        readonly Grid nativeLayer = new Grid();
        readonly HashSet<UIElement> nativeKnown = new HashSet<UIElement>();
        readonly Dictionary<UIElement, Rect> nativeRects = new Dictionary<UIElement, Rect>();
        List<NativeSlot> slots = new List<NativeSlot>();
        bool hideNative;
        double nativeBottomInset;
        bool rebuildQueued;

        public DragonWindow()
        {
            Title = DragonAppInfo.Title;
            Width = DragonAppInfo.Width;
            Height = DragonAppInfo.Height;
            MinWidth = Math.Min(320, DragonAppInfo.Width);
            MinHeight = Math.Min(480, DragonAppInfo.Height);
            UseLayoutRounding = true;
            SnapsToDevicePixels = true;
            TextOptions.SetTextFormattingMode(this, TextFormattingMode.Ideal);
            try { Theme.Icons = new FontFamily(new Uri("pack://application:,,,/"), "./Fonts/#Material Icons"); }
            catch (Exception) { Theme.Icons = null; }
            FontFamily = Theme.Text;
            shell.Children.Add(page);
            shell.Children.Add(nativeLayer);
            Content = shell;
            LayoutUpdated += (s, e) => PositionNative();
            host = new DragonHost(Dispatcher);
            host.FrameChanged += QueueRebuild;
            Loaded += (s, e) =>
            {
                Theme.Apply(default, SystemIsDark());
                host.Start(MediaJson());
            };
            SizeChanged += (s, e) => host.UpdateMedia(MediaJson());
            Activated += (s, e) => host.Lifecycle("resumed");
            Deactivated += (s, e) => host.Lifecycle("paused");
            Closing += (s, e) => host.Lifecycle("detached");
            PreviewKeyDown += OnKey;
            PreviewMouseDown += (s, e) =>
            {
                if (e.ChangedButton == MouseButton.XButton1 && host.CanHandleBack) { host.Back(); e.Handled = true; }
            };
        }

        void OnKey(object sender, KeyEventArgs e)
        {
            bool altLeft = (e.Key == Key.System && e.SystemKey == Key.Left && Keyboard.Modifiers == ModifierKeys.Alt) && host.Plugins.Count == 0;
            bool back = e.Key == Key.Escape || altLeft;
            if (back && host.CanHandleBack) { host.Back(); e.Handled = true; }
        }

        static bool SystemIsDark()
        {
            try
            {
                using (var k = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"))
                {
                    var v = k?.GetValue("AppsUseLightTheme");
                    if (v is int i) return i == 0;
                }
            }
            catch (Exception) { }
            return false;
        }

        string MediaJson()
        {
            var inv = CultureInfo.InvariantCulture;
            // Report the usable client area, not the outer window: the window frame and the vertical
            // scrollbar both take width that the layout cannot use (this made width-derived widgets overflow).
            double w = ActualWidth > 0 ? ActualWidth : Width, h = ActualHeight > 0 ? ActualHeight : Height;
            try
            {
                if (shell.ActualWidth > 0) w = shell.ActualWidth;
                if (shell.ActualHeight > 0) h = shell.ActualHeight;
                w = Math.Max(100, w - SystemParameters.VerticalScrollBarWidth);
            }
            catch (Exception) { }
            var src = PresentationSource.FromVisual(this);
            double ratio = src?.CompositionTarget != null ? src.CompositionTarget.TransformToDevice.M11 : 1.0;
            return "{\"width\":" + w.ToString("0.##", inv) + ",\"height\":" + h.ToString("0.##", inv) + ",\"pixelRatio\":" + ratio.ToString("0.##", inv)
                + ",\"top\":0,\"bottom\":0,\"left\":0,\"right\":0,\"keyboard\":0,\"brightness\":\"" + (SystemIsDark() ? "dark" : "light")
                + "\",\"textScale\":1,\"locale\":\"" + CultureInfo.CurrentUICulture.Name + "\"}";
        }

        void QueueRebuild()
        {
            if (rebuildQueued) return;
            rebuildQueued = true;
            Dispatcher.BeginInvoke(new Action(() => { rebuildQueued = false; Rebuild(); }), DispatcherPriority.Background);
        }

        void Rebuild()
        {
            state.Rebuilding = true;
            try
            {
                if (host.FatalError != null) { ShowFatal(host.FatalError); return; }
                var frame = host.Frame;
                if (frame.ValueKind != JsonValueKind.Object) return;
                var theme = frame.Obj("theme");
                Theme.Apply(theme ?? default, SystemIsDark());
                string title = frame.Str("title");
                if (!string.IsNullOrEmpty(title) && title != Title) Title = title;
                host.Scrollers.Clear();
                host.ScrollExtents.Clear();
                var ctx = new Ctx { Host = host, State = state };
                var root = frame.Obj("root");
                state.Slots.Clear();
                var pg = new Grid { Background = Theme.Solid(Theme.Background) };
                if (root.HasValue)
                {
                    var content = R.Node(root.Value.Obj("content"), ctx.Sub("page" + (root.Value.Str("key") ?? "")));
                    if (content != null)
                    {
                        R.Detach(content);
                        pg.Children.Add(content);
                    }
                }
                var ovs = frame.Arr("overlays");
                bool modal = false, snack = false;
                for (int i = 0; i < ovs.Count; i++)
                {
                    var ov = R.Overlay(ovs[i], ctx.Sub("o" + i));
                    if (ov != null) pg.Children.Add(ov);
                    if ((ovs[i].Str("kind") ?? "dialog") == "snackbar") snack = true; else modal = true;
                }
                hideNative = modal;
                nativeBottomInset = snack ? 96 : 0;
                var err = frame.Str("error");
                if (!string.IsNullOrEmpty(err))
                {
                    var eb = R.Node(JsonDocument.Parse("{\"type\":\"error\",\"message\":" + JsonSerializer.Serialize(err) + "}").RootElement, ctx.Sub("err"));
                    if (eb is FrameworkElement ef) ef.VerticalAlignment = VerticalAlignment.Top;
                    if (eb != null) pg.Children.Add(eb);
                }
                TextElement.SetForeground(pg, Theme.Solid(Theme.OnSurface));
                TextElement.SetFontFamily(pg, Theme.Text);
                page.Children.Clear();
                page.Children.Add(pg);
                slots = new List<NativeSlot>(state.Slots);
                SyncNative();
            }
            finally
            {
                Dispatcher.BeginInvoke(new Action(() => state.Rebuilding = false), DispatcherPriority.ContextIdle);
            }
        }

        void SyncNative()
        {
            var keep = new HashSet<UIElement>();
            foreach (var sl in slots) keep.Add(sl.View);
            foreach (var k in new List<UIElement>(nativeKnown))
                if (!keep.Contains(k)) { k.Visibility = Visibility.Collapsed; }
            foreach (var sl in slots)
            {
                if (!nativeKnown.Contains(sl.View))
                {
                    R.Detach(sl.View);
                    if (sl.View is FrameworkElement fe) { fe.HorizontalAlignment = HorizontalAlignment.Left; fe.VerticalAlignment = VerticalAlignment.Top; }
                    nativeLayer.Children.Add(sl.View);
                    nativeKnown.Add(sl.View);
                }
            }
            nativeRects.Clear();
            Dispatcher.BeginInvoke(new Action(PositionNative), DispatcherPriority.Loaded);
        }

        void PositionNative()
        {
            foreach (var sl in slots)
            {
                var fe = sl.View as FrameworkElement;
                if (fe == null) continue;
                Rect r;
                bool ok = false;
                r = Rect.Empty;
                try
                {
                    if (sl.Holder.IsLoaded && sl.Holder.ActualWidth > 0 && sl.Holder.ActualHeight > 0)
                    {
                        var t = sl.Holder.TransformToVisual(nativeLayer);
                        r = t.TransformBounds(new Rect(0, 0, sl.Holder.ActualWidth, sl.Holder.ActualHeight));
                        double bottom = nativeLayer.ActualHeight - nativeBottomInset;
                        if (r.Bottom > bottom) r = new Rect(r.X, r.Y, r.Width, Math.Max(0, bottom - r.Y));
                        ok = r.Width > 0 && r.Height > 0;
                    }
                }
                catch (InvalidOperationException) { ok = false; }
                if (!ok || hideNative)
                {
                    if (fe.Visibility != Visibility.Collapsed) fe.Visibility = Visibility.Collapsed;
                    continue;
                }
                Rect old;
                if (nativeRects.TryGetValue(fe, out old) && old == r && fe.Visibility == Visibility.Visible) continue;
                nativeRects[fe] = r;
                fe.Margin = new Thickness(r.X, r.Y, 0, 0);
                fe.Width = r.Width;
                fe.Height = r.Height;
                fe.Visibility = Visibility.Visible;
            }
        }

        void ShowFatal(string message)
        {
            page.Children.Clear();
            page.Children.Add(new ScrollViewer
            {
                Content = new TextBlock { Text = "Dragon could not start:\n\n" + message, Margin = new Thickness(24), TextWrapping = TextWrapping.Wrap, FontSize = 14 },
            });
        }
    }
}
