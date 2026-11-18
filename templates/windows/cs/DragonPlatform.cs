using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;

namespace DragonRuntime
{
    internal sealed class DragonPlatform
    {
        static readonly HttpClient Http = new HttpClient(new HttpClientHandler { AutomaticDecompression = System.Net.DecompressionMethods.All });
        static readonly HttpClient InsecureHttp = new HttpClient(new HttpClientHandler { AutomaticDecompression = System.Net.DecompressionMethods.All, ServerCertificateCustomValidationCallback = (m, c, ch, e) => true });
        readonly Action<string, object> reply;
        readonly Dispatcher ui;
        public List<IDragonPlugin> Plugins { get; }

        public DragonPlatform(string docsDir, Dispatcher dispatcher, Action<string, object> reply)
        {
            this.reply = reply;
            ui = dispatcher;
            Plugins = DragonPlugins.Create(new PluginContext(docsDir, dispatcher));
        }

        public void Handle(JsonElement call)
        {
            string id = call.Str("id") ?? "";
            string method = call.Str("method") ?? "";
            JsonElement args = call.Get("args");
            try
            {
                switch (method)
                {
                    case "http": RunHttp(id, args); break;
                    case "clipboard.set": Clipboard.SetText(args.Str("text") ?? ""); break;
                    case "clipboard.get": reply(id, new Dictionary<string, object> { ["text"] = Clipboard.ContainsText() ? Clipboard.GetText() : "" }); break;
                    case "openUrl":
                        Process.Start(new ProcessStartInfo(args.Str("url") ?? "") { UseShellExecute = true });
                        reply(id, true);
                        break;
                    case "share": Clipboard.SetText(args.Str("text") ?? ""); Toast("Copied to clipboard"); break;
                    case "toast": Toast(args.Str("message") ?? ""); break;
                    case "haptic":
                    case "vibrate": break;
                    case "deviceInfo":
                        reply(id, new Dictionary<string, object>
                        {
                            ["platform"] = "windows",
                            ["model"] = Environment.MachineName,
                            ["manufacturer"] = "Microsoft",
                            ["release"] = Environment.OSVersion.VersionString,
                        });
                        break;
                    case "permission.check":
                    case "permission.request": reply(id, new Dictionary<string, object> { ["granted"] = true }); break;
                    case "notify": Toast((args.Str("title") ?? "") + " " + (args.Str("body") ?? "")); break;
                    case "notify.cancel":
                    case "statusBar":
                    case "orientation": break;
                    case "pickFile": PickFile(id); break;
                    case "datePicker": PickDate(id, args.Str("initial")); break;
                    case "timePicker": PickTime(id, args.Str("initial")); break;
                    case "takePhoto":
                    case "location": reply(id, new Dictionary<string, object> { ["error"] = method + " is not available on Windows yet" }); break;
                    case "exit": Application.Current.Shutdown(); break;
                    default:
                        bool handled = Plugins.Any(p => p.Handles(method) && p.Handle(method, args, r => reply(id, r)));
                        if (!handled) reply(id, null);
                        break;
                }
            }
            catch (Exception e)
            {
                reply(id, new Dictionary<string, object> { ["error"] = e.Message });
            }
        }

        async void RunHttp(string id, JsonElement a)
        {
            var result = new Dictionary<string, object>();
            try
            {
                var req = new HttpRequestMessage(new HttpMethod(a.Str("method") ?? "GET"), a.Str("url") ?? "");
                var headers = a.Get("headers");
                if (headers.IsObj())
                    foreach (var h in headers.EnumerateObject()) req.Headers.TryAddWithoutValidation(h.Name, h.Value.ToString());
                var body = a.Str("body");
                var bodyParts = a.Get("bodyParts");
                if (bodyParts.ValueKind == JsonValueKind.Array)
                {
                    var ms = new System.IO.MemoryStream();
                    foreach (var p in bodyParts.EnumerateArray())
                    {
                        var t = p.Str("text");
                        var b = p.Str("base64");
                        if (t != null) { var tb = Encoding.UTF8.GetBytes(t); ms.Write(tb, 0, tb.Length); }
                        else if (b != null) { var bb = Convert.FromBase64String(b); ms.Write(bb, 0, bb.Length); }
                    }
                    var bc = new ByteArrayContent(ms.ToArray());
                    var ct = headers.IsObj() && headers.TryGetProperty("Content-Type", out var cth) ? cth.ToString() : null;
                    if (ct != null && System.Net.Http.Headers.MediaTypeHeaderValue.TryParse(ct, out var bmt)) bc.Headers.ContentType = bmt;
                    req.Content = bc;
                }
                else if (body != null)
                {
                    // StringContent defaults to text/plain, which makes servers ignore form and JSON bodies.
                    // Content-Type is a content header, so it is set here rather than on the request.
                    req.Content = new StringContent(body, Encoding.UTF8);
                    if (headers.IsObj() && headers.TryGetProperty("Content-Type", out var ctv)
                        && System.Net.Http.Headers.MediaTypeHeaderValue.TryParse(ctv.ToString(), out var mt))
                        req.Content.Headers.ContentType = mt;
                }
                using var cts = new System.Threading.CancellationTokenSource(a.Int("timeout") ?? 30000);
                var resp = await ((a.Bool("insecure") == true ? InsecureHttp : Http).SendAsync(req, cts.Token));
                result["status"] = (int)resp.StatusCode;
                result["body"] = await resp.Content.ReadAsStringAsync();
                var hs = new Dictionary<string, string>();
                foreach (var h in resp.Headers) hs[h.Key] = string.Join(",", h.Value);
                foreach (var h in resp.Content.Headers) hs[h.Key] = string.Join(",", h.Value);
                result["headers"] = hs;
            }
            catch (Exception e)
            {
                result["status"] = 0;
                result["error"] = e.Message;
            }
            reply(id, result);
        }

        public void Toast(string message)
        {
            var window = Application.Current?.MainWindow;
            if (window == null || !(window.Content is Panel root)) return;
            var label = new Border
            {
                Background = new SolidColorBrush(Color.FromArgb(230, 40, 40, 40)),
                CornerRadius = new CornerRadius(16),
                Padding = new Thickness(16, 8, 16, 8),
                Margin = new Thickness(0, 0, 0, 40),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Bottom,
                IsHitTestVisible = false,
                Child = new TextBlock { Text = message, Foreground = Brushes.White },
            };
            Panel.SetZIndex(label, 1000);
            root.Children.Add(label);
            var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2.5) };
            timer.Tick += (s, e) => { timer.Stop(); root.Children.Remove(label); };
            timer.Start();
        }

        void PickFile(string id)
        {
            var dlg = new Microsoft.Win32.OpenFileDialog();
            if (dlg.ShowDialog() == true)
                reply(id, new Dictionary<string, object> { ["path"] = dlg.FileName, ["name"] = Path.GetFileName(dlg.FileName) });
            else
                reply(id, null);
        }

        void PickDate(string id, string initial)
        {
            var calendar = new Calendar();
            if (DateTime.TryParse(initial, out var d)) { calendar.SelectedDate = d; calendar.DisplayDate = d; }
            var win = new Window { Title = "Select date", Content = calendar, SizeToContent = SizeToContent.WidthAndHeight, WindowStartupLocation = WindowStartupLocation.CenterOwner, Owner = Application.Current.MainWindow, ResizeMode = ResizeMode.NoResize };
            DateTime? picked = null;
            calendar.SelectedDatesChanged += (s, e) => { picked = calendar.SelectedDate; win.Close(); };
            win.ShowDialog();
            reply(id, picked.HasValue ? new Dictionary<string, object> { ["date"] = picked.Value.ToString("yyyy-MM-dd") } : null);
        }

        void PickTime(string id, string initial)
        {
            var box = new TextBox { Text = string.IsNullOrEmpty(initial) ? DateTime.Now.ToString("HH:mm") : initial, Margin = new Thickness(12), MinWidth = 120 };
            var ok = new Button { Content = "OK", Margin = new Thickness(12, 0, 12, 12), IsDefault = true };
            var panel = new StackPanel();
            panel.Children.Add(box);
            panel.Children.Add(ok);
            var win = new Window { Title = "Select time (HH:mm)", Content = panel, SizeToContent = SizeToContent.WidthAndHeight, WindowStartupLocation = WindowStartupLocation.CenterOwner, Owner = Application.Current.MainWindow, ResizeMode = ResizeMode.NoResize };
            string result = null;
            ok.Click += (s, e) => { result = box.Text; win.Close(); };
            win.ShowDialog();
            reply(id, result != null ? new Dictionary<string, object> { ["time"] = result } : null);
        }
    }
}
