using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;

namespace DragonRuntime
{
    internal sealed class DragonHost
    {
        public JsonElement Frame { get; private set; }
        public string FatalError { get; private set; }
        public event Action FrameChanged;

        public readonly Dictionary<string, bool> Drawers = new Dictionary<string, bool>();
        public readonly Dictionary<int, ScrollViewer> Scrollers = new Dictionary<int, ScrollViewer>();
        public readonly Dictionary<int, double> ScrollExtents = new Dictionary<int, double>();

        readonly Dispatcher ui;
        readonly DispatcherTimer timer;
        DragonPlatform platform;
        string mediaJson = "";
        string lastFrameText = "";
        bool started;
        JsonDocument doc;

        public List<IDragonPlugin> Plugins => platform.Plugins;

        public DragonHost(Dispatcher dispatcher)
        {
            ui = dispatcher;
            timer = new DispatcherTimer(DispatcherPriority.Background, dispatcher);
            timer.Tick += (s, e) => Pump();
        }

        public void Start(string media)
        {
            if (started) return;
            started = true;
            mediaJson = media;
            string docs = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), AppName(), "dragon_docs");
            Directory.CreateDirectory(docs);
            platform = new DragonPlatform(docs, ui, OnPlatformResult);
            string err;
            try { err = DragonNative.Start(docs, media); }
            catch (Exception e) { err = e.Message; }
            if (!string.IsNullOrEmpty(err)) { FatalError = err; FrameChanged?.Invoke(); return; }
            Refresh();
            DrainPlatform();
            timer.Interval = TimeSpan.FromMilliseconds(16);
            timer.Start();
        }

        static string AppName() => System.Reflection.Assembly.GetEntryAssembly()?.GetName().Name ?? "DragonApp";

        void Pump()
        {
            if (FatalError != null) return;
            long next = DragonNative.Tick(DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
            if (DragonNative.IsDirty()) Refresh();
            DrainPlatform();
            double ms = next >= 0 && next < 100 ? Math.Max(next, 8) : 100;
            timer.Interval = TimeSpan.FromMilliseconds(ms);
        }

        static readonly JsonSerializerOptions JsonOpts = new JsonSerializerOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

        void OnPlatformResult(string id, object value)
        {
            ui.BeginInvoke(new Action(() =>
            {
                string json;
                if (value == null) json = "";
                else if (value is string s) json = JsonSerializer.Serialize(s, JsonOpts);
                else json = JsonSerializer.Serialize(value, JsonOpts);
                DragonNative.PlatformResult(id, json);
                Refresh();
                DrainPlatform();
            }));
        }

        public void UpdateMedia(string media)
        {
            if (!started || FatalError != null || media == mediaJson) return;
            mediaJson = media;
            DragonNative.SetMedia(media);
            Refresh();
        }

        public void Lifecycle(string state)
        {
            if (!started || FatalError != null) return;
            DragonNative.Lifecycle(state);
            if (state == "resumed") Refresh();
        }

        public void Dispatch(string evt)
        {
            if (string.IsNullOrEmpty(evt) || FatalError != null) return;
            DragonNative.Event(evt);
            Refresh();
            DrainPlatform();
        }

        public bool Back()
        {
            var open = new List<string>();
            foreach (var kv in Drawers) if (kv.Value) open.Add(kv.Key);
            if (open.Count > 0)
            {
                foreach (var k in open) Drawers[k] = false;
                FrameChanged?.Invoke();
                return true;
            }
            bool handled = DragonNative.Back();
            Refresh();
            return handled;
        }

        public bool CanHandleBack
        {
            get
            {
                if (Frame.ValueKind != JsonValueKind.Object) return false;
                foreach (var v in Drawers.Values) if (v) return true;
                return (Frame.Bool("canPop") ?? false) || (Frame.Bool("hasBack") ?? false) || Frame.Arr("overlays").Count > 0;
            }
        }

        public void SetDrawer(string side, bool open)
        {
            Drawers[side] = open;
            FrameChanged?.Invoke();
        }

        void Refresh()
        {
            string text = DragonNative.Render();
            bool same = text == lastFrameText;
            if (!same)
            {
                try
                {
                    var next = JsonDocument.Parse(text);
                    doc = next;
                    Frame = doc.RootElement;
                    lastFrameText = text;
                }
                catch (Exception e)
                {
                    FatalError = "Bad frame from the Dragon runtime: " + e.Message;
                    FrameChanged?.Invoke();
                    return;
                }
            }
            foreach (var c in Frame.Arr("commands")) HandleCommand(c);
            if (!same) FrameChanged?.Invoke();
        }

        void HandleCommand(JsonElement c)
        {
            switch (c.Str("cmd"))
            {
                case "drawer":
                    Drawers[c.Str("side") ?? "start"] = c.Str("op") == "open";
                    break;
                case "scroll":
                    int id = c.Int("id") ?? -1;
                    ui.BeginInvoke(new Action(() => ApplyScroll(id, c.Clone())), DispatcherPriority.Loaded);
                    break;
            }
        }

        void ApplyScroll(int id, JsonElement c)
        {
            if (!Scrollers.TryGetValue(id, out var sv)) return;
            double extent = ScrollExtents.TryGetValue(id, out var e) ? e : 0;
            double target;
            if (c.Has("index") && extent > 0) target = (c.Int("index") ?? 0) * extent;
            else if (c.Bool("end") == true) target = sv.ScrollableHeight;
            else target = c.Num("offset") ?? 0;
            sv.ScrollToVerticalOffset(target);
        }

        void DrainPlatform()
        {
            string txt = DragonNative.PlatformCalls();
            if (txt.Length <= 2) return;
            try
            {
                using var calls = JsonDocument.Parse(txt);
                foreach (var call in calls.RootElement.EnumerateArray()) platform.Handle(call.Clone());
            }
            catch (JsonException) { }
        }
    }
}
