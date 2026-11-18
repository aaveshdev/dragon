using System;
using System.Collections.Generic;
using System.IO;
using System.Net.Http;
using System.Runtime.InteropServices;
using System.Windows.Input;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.Wpf;

namespace DragonRuntime
{
    internal sealed class WebTab
    {
        public string Id;
        public bool Private;
        public bool Desktop;
        public WebView2 Control;
        public string Url = "";
        public string Title = "";
        public int Progress;
        public bool Loading;
        public string Error;
        public string Favicon;
        public int FindCount;
        public string FindText = "";
        public bool Ready;
        public bool Closed;
        public readonly List<Action<CoreWebView2>> Queue = new List<Action<CoreWebView2>>();
    }

    public sealed class DragonWebPlugin : DragonPluginBase
    {
        public override string Namespace => "web";

        readonly PluginContext ctx;
        readonly Dictionary<string, WebTab> tabs = new Dictionary<string, WebTab>();
        readonly List<Dictionary<string, object>> events = new List<Dictionary<string, object>>();
        static readonly HttpClient Http = new HttpClient();
        int counter;
        bool hooked;

        [DllImport("user32.dll")]
        static extern short GetKeyState(int vk);

        public DragonWebPlugin(PluginContext context)
        {
            ctx = context;
        }

        string Root(params string[] parts)
        {
            string p = Path.Combine(ctx.DocsDir, Path.Combine(parts));
            Directory.CreateDirectory(Path.GetDirectoryName(p) ?? ctx.DocsDir);
            return p;
        }

        public override bool Handle(string method, JsonElement args, Action<object> reply)
        {
            HookWindow();
            string id = args.Str("tab") ?? "";
            tabs.TryGetValue(id, out var tab);
            switch (method)
            {
                case "web.config": reply(true); break;
                case "web.state": reply(State()); break;
                case "web.open":
                    {
                        var t = tab ?? Create(id, args.Bool("private") ?? false, args.Bool("desktop") ?? false);
                        string url = args.Str("url");
                        if (!string.IsNullOrEmpty(url)) Navigate(t, url);
                        reply(true);
                        break;
                    }
                case "web.load": if (tab != null) Navigate(tab, args.Str("url") ?? ""); reply(true); break;
                case "web.back": if (tab != null) Do(tab, c => { if (c.CanGoBack) c.GoBack(); }); reply(true); break;
                case "web.forward": if (tab != null) Do(tab, c => { if (c.CanGoForward) c.GoForward(); }); reply(true); break;
                case "web.reload": if (tab != null) Do(tab, c => c.Reload()); reply(true); break;
                case "web.stop": if (tab != null) Do(tab, c => c.Stop()); reply(true); break;
                case "web.close": if (tab != null) Close(tab); reply(true); break;
                case "web.desktop":
                    if (tab != null)
                    {
                        tab.Desktop = args.Bool("on") ?? false;
                        Do(tab, c => c.Reload());
                    }
                    reply(true);
                    break;
                case "web.find": if (tab != null) Find(tab, args.Str("text") ?? "", true, true); reply(true); break;
                case "web.findNext": if (tab != null) Find(tab, tab.FindText, args.Bool("forward") ?? true, false); reply(true); break;
                case "web.findClear":
                    if (tab != null) { tab.FindText = ""; tab.FindCount = 0; Do(tab, c => c.ExecuteScriptAsync("window.getSelection().removeAllRanges()")); }
                    reply(true);
                    break;
                case "web.snapshot": if (tab == null) reply(null); else Snapshot(tab, reply); break;
                case "web.favicon": reply(tab?.Favicon == null ? null : new Dictionary<string, object> { ["path"] = tab.Favicon }); break;
                case "web.clear": ClearData(reply); break;
                case "web.print": if (tab != null) Do(tab, c => c.ShowPrintUI(CoreWebView2PrintDialogKind.Browser)); reply(true); break;
                case "web.eval":
                    if (tab == null) reply(null);
                    else Eval(tab, args.Str("code") ?? "", reply);
                    break;
                case "web.zoom":
                    if (tab != null) { double f = Math.Max(0.25, Math.Min(5.0, args.Num("factor") ?? 1.0)); tab.Control.ZoomFactor = f; }
                    reply(true);
                    break;
                case "web.devtools": if (tab != null) Do(tab, c => c.OpenDevToolsWindow()); reply(true); break;
                case "web.download": Download(args.Str("url") ?? "", tab); reply(true); break;
                default: reply(null); break;
            }
            return true;
        }

        WebTab Create(string id, bool priv, bool desktop)
        {
            var tab = new WebTab { Id = id, Private = priv, Desktop = desktop };
            string folder = Root("webview2", priv ? "private_" + SafeName(id) : "default", "x");
            folder = Path.GetDirectoryName(folder);
            tab.Control = new WebView2
            {
                CreationProperties = new CoreWebView2CreationProperties { UserDataFolder = folder },
                DefaultBackgroundColor = System.Drawing.Color.White,
            };
            tabs[id] = tab;
            Init(tab);
            return tab;
        }

        static string SafeName(string s)
        {
            var sb = new System.Text.StringBuilder();
            foreach (char c in s) sb.Append(char.IsLetterOrDigit(c) ? c : '_');
            return sb.ToString();
        }

        async void Init(WebTab tab)
        {
            try
            {
                await tab.Control.EnsureCoreWebView2Async();
            }
            catch (Exception e)
            {
                tab.Error = "WebView2 is not available: " + e.Message;
                return;
            }
            if (tab.Closed) return;
            var core = tab.Control.CoreWebView2;
            core.Settings.AreDefaultContextMenusEnabled = true;
            core.NavigationStarting += (s, e) => { tab.Loading = true; tab.Progress = 10; tab.Error = null; tab.Url = e.Uri; };
            core.SourceChanged += (s, e) => { tab.Url = core.Source ?? tab.Url; };
            core.ContentLoading += (s, e) => { tab.Progress = 60; };
            core.NavigationCompleted += (s, e) =>
            {
                tab.Loading = false;
                tab.Progress = 100;
                tab.Url = core.Source ?? tab.Url;
                tab.Title = core.DocumentTitle ?? tab.Title;
                if (!e.IsSuccess && e.WebErrorStatus != CoreWebView2WebErrorStatus.OperationCanceled) tab.Error = e.WebErrorStatus.ToString();
            };
            core.DocumentTitleChanged += (s, e) => { tab.Title = core.DocumentTitle ?? ""; };
            core.NewWindowRequested += (s, e) =>
            {
                e.Handled = true;
                string nid = "t_w" + (++counter) + "_" + Environment.TickCount;
                var nt = Create(nid, tab.Private, tab.Desktop);
                Navigate(nt, e.Uri);
                events.Add(new Dictionary<string, object> { ["type"] = "newtab", ["tab"] = nid, ["private"] = tab.Private, ["from"] = tab.Id });
            };
            core.FaviconChanged += async (s, e) => { await SaveFavicon(tab, core); };
            core.DownloadStarting += (s, e) =>
            {
                string name = Path.GetFileName(e.ResultFilePath ?? "download");
                events.Add(new Dictionary<string, object> { ["type"] = "download", ["ok"] = true, ["name"] = name, ["url"] = e.DownloadOperation.Uri, ["tab"] = tab.Id });
            };
            core.ProcessFailed += (s, e) =>
            {
                events.Add(new Dictionary<string, object> { ["type"] = "crashed", ["tab"] = tab.Id });
                try { core.Reload(); } catch (Exception) { }
            };
            var controller = FindController(tab.Control);
            if (controller != null) controller.AcceleratorKeyPressed += (s, e) =>
            {
                if (e.KeyEventKind != CoreWebView2KeyEventKind.KeyDown && e.KeyEventKind != CoreWebView2KeyEventKind.SystemKeyDown) return;
                bool ctrl = (GetKeyState(0x11) & 0x8000) != 0, shift = (GetKeyState(0x10) & 0x8000) != 0, alt = (GetKeyState(0x12) & 0x8000) != 0;
                string name = ShortcutName((int)e.VirtualKey, ctrl, shift, alt);
                if (name == null) return;
                e.Handled = true;
                events.Add(new Dictionary<string, object> { ["type"] = "shortcut", ["name"] = name });
            };
            tab.Ready = true;
            foreach (var q in tab.Queue.ToArray()) q(core);
            tab.Queue.Clear();
        }

        static CoreWebView2Controller FindController(WebView2 view)
        {
            var flags = System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic;
            try
            {
                var t = typeof(WebView2);
                var prop = t.GetProperty("CoreWebView2Controller", flags);
                if (prop != null) { var c = prop.GetValue(view) as CoreWebView2Controller; if (c != null) return c; }
                foreach (var f in t.GetFields(flags))
                    if (f.FieldType == typeof(CoreWebView2Controller)) { var c = f.GetValue(view) as CoreWebView2Controller; if (c != null) return c; }
            }
            catch (Exception) { }
            return null;
        }

        async Task SaveFavicon(WebTab tab, CoreWebView2 core)
        {
            try
            {
                if (string.IsNullOrEmpty(core.FaviconUri)) return;
                string host = new Uri(core.Source).Host.ToLowerInvariant();
                string path = Root("favicons", host + ".png");
                using (var stream = await core.GetFaviconAsync(CoreWebView2FaviconImageFormat.Png))
                using (var f = File.Create(path))
                {
                    await stream.CopyToAsync(f);
                }
                tab.Favicon = path;
            }
            catch (Exception) { }
        }

        void Do(WebTab tab, Action<CoreWebView2> action)
        {
            if (tab.Ready && tab.Control.CoreWebView2 != null) action(tab.Control.CoreWebView2);
            else tab.Queue.Add(action);
        }

        void Navigate(WebTab tab, string raw)
        {
            string s = (raw ?? "").Trim();
            if (s.Length == 0) return;
            if (!s.Contains("://") && !s.StartsWith("about:") && !s.StartsWith("data:")) s = "https://" + s;
            tab.Url = s;
            tab.Error = null;
            Do(tab, c => c.Navigate(s));
        }

        void Close(WebTab tab)
        {
            tab.Closed = true;
            tabs.Remove(tab.Id);
            try
            {
                if (tab.Control.Parent is System.Windows.Controls.Panel p) p.Children.Remove(tab.Control);
                tab.Control.Dispose();
            }
            catch (Exception) { }
            if (tab.Private)
            {
                string folder = Path.Combine(ctx.DocsDir, "webview2", "private_" + SafeName(tab.Id));
                Task.Run(async () =>
                {
                    await Task.Delay(1500);
                    try { Directory.Delete(folder, true); } catch (Exception) { }
                });
            }
        }

        void Find(WebTab tab, string text, bool forward, bool fresh)
        {
            tab.FindText = text;
            if (text.Length == 0) { tab.FindCount = 0; return; }
            string lit = JsonSerializer.Serialize(text);
            string back = forward ? "false" : "true";
            Do(tab, async c =>
            {
                try
                {
                    string r = await c.ExecuteScriptAsync(
                        "(function(){var t=" + lit + ";var n=0;var h=(document.body?document.body.innerText:'').toLowerCase();var q=t.toLowerCase();var i=h.indexOf(q);while(i>=0){n++;i=h.indexOf(q,i+q.length);}" +
                        "window.find(t,false," + back + ",true);return n;})()");
                    if (int.TryParse(r, out int count)) tab.FindCount = count;
                }
                catch (Exception) { }
            });
        }

        async void Snapshot(WebTab tab, Action<object> reply)
        {
            if (!tab.Ready || tab.Control.CoreWebView2 == null) { reply(null); return; }
            try
            {
                string path = Root("thumbs", "thumb_" + SafeName(tab.Id) + ".jpg");
                using (var f = File.Create(path))
                {
                    await tab.Control.CoreWebView2.CapturePreviewAsync(CoreWebView2CapturePreviewImageFormat.Jpeg, f);
                }
                reply(new Dictionary<string, object> { ["path"] = path, ["t"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() });
            }
            catch (Exception) { reply(null); }
        }

        async void Eval(WebTab tab, string code, Action<object> reply)
        {
            if (!tab.Ready || tab.Control.CoreWebView2 == null) { reply(null); return; }
            try
            {
                string r = await tab.Control.CoreWebView2.ExecuteScriptAsync(code);
                reply(new Dictionary<string, object> { ["value"] = r ?? "null" });
            }
            catch (Exception) { reply(null); }
        }

        async void ClearData(Action<object> reply)
        {
            try
            {
                foreach (var t in tabs.Values)
                {
                    if (t.Ready && t.Control.CoreWebView2 != null && !t.Private)
                    {
                        await t.Control.CoreWebView2.Profile.ClearBrowsingDataAsync();
                        break;
                    }
                }
            }
            catch (Exception) { }
            reply(true);
        }

        async void Download(string url, WebTab tab)
        {
            if (!url.StartsWith("http", StringComparison.OrdinalIgnoreCase))
            {
                events.Add(new Dictionary<string, object> { ["type"] = "download", ["ok"] = false, ["name"] = "Unsupported link" });
                return;
            }
            string name = "download";
            try
            {
                name = Path.GetFileName(new Uri(url).AbsolutePath);
                if (string.IsNullOrEmpty(name)) name = "download";
                string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Downloads");
                Directory.CreateDirectory(dir);
                var bytes = await Http.GetByteArrayAsync(url);
                File.WriteAllBytes(Path.Combine(dir, name), bytes);
                events.Add(new Dictionary<string, object> { ["type"] = "download", ["ok"] = true, ["name"] = name, ["url"] = url, ["tab"] = tab?.Id ?? "" });
            }
            catch (Exception e)
            {
                events.Add(new Dictionary<string, object> { ["type"] = "download", ["ok"] = false, ["name"] = e.Message });
            }
        }

        void HookWindow()
        {
            if (hooked) return;
            var win = Application.Current?.MainWindow;
            if (win == null) return;
            hooked = true;
            win.PreviewKeyDown += (s, e) =>
            {
                var key = e.Key == Key.System ? e.SystemKey : e.Key;
                var m = Keyboard.Modifiers;
                string name = ShortcutName(KeyInterop.VirtualKeyFromKey(key), (m & ModifierKeys.Control) != 0, (m & ModifierKeys.Shift) != 0, (m & ModifierKeys.Alt) != 0);
                if (name == null) return;
                e.Handled = true;
                events.Add(new Dictionary<string, object> { ["type"] = "shortcut", ["name"] = name });
            };
        }

        static string ShortcutName(int vk, bool ctrl, bool shift, bool alt)
        {
            if (alt && !ctrl)
            {
                if (vk == 0x25) return "back";
                if (vk == 0x27) return "forward";
                if (vk == 0x44) return "focusaddress";
                if (vk == 0x24) return "home";
                return null;
            }
            if (ctrl && !alt)
            {
                if (vk == 0x09) return shift ? "prevtab" : "nexttab";
                if (vk == 0x21) return "prevtab";
                if (vk == 0x22) return "nexttab";
                if (vk >= 0x31 && vk <= 0x39) return "tab" + (vk - 0x30);
                if (vk == 0x30 || vk == 0x60) return "zoomreset";
                if (vk == 0xBB || vk == 0x6B) return "zoomin";
                if (vk == 0xBD || vk == 0x6D) return "zoomout";
                if (shift)
                {
                    if (vk == 0x54) return "reopentab";
                    if (vk == 0x4E) return "privatetab";
                    if (vk == 0x2E) return "cleardata";
                    if (vk == 0x49) return "devtools";
                    if (vk == 0x4F) return "bookmarks";
                    return null;
                }
                switch (vk)
                {
                    case 0x54: return "newtab";
                    case 0x57: return "closetab";
                    case 0x4C: return "focusaddress";
                    case 0x46: return "find";
                    case 0x52: return "reload";
                    case 0x44: return "bookmark";
                    case 0x48: return "history";
                    case 0x4A: return "downloads";
                    case 0x47: return "findnext";
                }
                return null;
            }
            if (!ctrl && !alt)
            {
                if (vk == 0x74) return "reload";
                if (vk == 0x75) return "focusaddress";
                if (vk == 0x7B) return "devtools";
            }
            return null;
        }

        Dictionary<string, object> State()
        {
            var ts = new Dictionary<string, object>();
            foreach (var t in tabs.Values)
            {
                var core = t.Ready ? t.Control.CoreWebView2 : null;
                bool canBack = false, canFwd = false;
                try { canBack = core != null && core.CanGoBack; canFwd = core != null && core.CanGoForward; } catch (Exception) { }
                ts[t.Id] = new Dictionary<string, object>
                {
                    ["url"] = t.Url,
                    ["title"] = t.Title,
                    ["progress"] = t.Progress,
                    ["loading"] = t.Loading,
                    ["canBack"] = canBack,
                    ["canForward"] = canFwd,
                    ["secure"] = t.Url.StartsWith("https://", StringComparison.OrdinalIgnoreCase) && t.Error == null,
                    ["counts"] = new Dictionary<string, object>(),
                    ["favicon"] = t.Favicon,
                    ["private"] = t.Private,
                    ["desktop"] = t.Desktop,
                    ["findCount"] = t.FindCount,
                    ["findIdx"] = 0,
                    ["error"] = t.Error,
                    ["zoom"] = t.Control.ZoomFactor,
                };
            }
            var ev = new List<object>(events);
            events.Clear();
            return new Dictionary<string, object>
            {
                ["tabs"] = ts,
                ["totals"] = new Dictionary<string, object> { ["upgrades"] = 0 },
                ["privateIsolated"] = true,
                ["events"] = ev,
            };
        }

        public override UIElement CreateView(string kind, string id, JsonElement parameters)
        {
            if (kind != "web") return null;
            string tabId = parameters.Str("tab") ?? id;
            if (!tabs.TryGetValue(tabId, out var tab)) tab = Create(tabId, false, false);
            return tab.Control;
        }
    }
}
