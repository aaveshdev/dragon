using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows.Media;
using System.Windows.Threading;

namespace DragonRuntime
{
    public sealed class DragonAudioPlugin : DragonPluginBase
    {
        public override string Namespace => "audio";

        readonly PluginContext ctx;
        readonly MediaPlayer player = new MediaPlayer();
        readonly List<Dictionary<string, object>> items = new List<Dictionary<string, object>>();
        readonly Dictionary<string, string> albumTrack = new Dictionary<string, string>();
        readonly DispatcherTimer sleepTimer;
        readonly Random rng = new Random();
        int index = -1;
        bool playing;
        bool ended;
        bool shuffle;
        string repeat = "off";
        double speed = 1.0;
        long sleepEnd;
        long durationMs;

        static readonly string[] Exts = { ".mp3", ".m4a", ".aac", ".wav", ".wma", ".flac", ".ogg", ".opus" };

        public DragonAudioPlugin(PluginContext context)
        {
            ctx = context;
            player.MediaOpened += (s, e) =>
            {
                if (player.NaturalDuration.HasTimeSpan) durationMs = (long)player.NaturalDuration.TimeSpan.TotalMilliseconds;
            };
            player.MediaEnded += (s, e) => OnEnded();
            player.MediaFailed += (s, e) => OnEnded();
            sleepTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
            sleepTimer.Tick += (s, e) =>
            {
                if (sleepEnd > 0 && Now() >= sleepEnd)
                {
                    sleepEnd = 0;
                    sleepTimer.Stop();
                    Pause();
                }
            };
        }

        static long Now() => DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();

        public override bool Handles(string method) => method.StartsWith("audio.", StringComparison.Ordinal) || method == "media.query" || method == "media.art";

        public override bool Handle(string method, JsonElement a, Action<object> reply)
        {
            switch (method)
            {
                case "media.query": Query(a.Int("minDurationMs") ?? 30000, reply); return true;
                case "media.art": Art(a.Str("albumId") ?? "", reply); return true;
                case "audio.setQueue":
                    items.Clear();
                    foreach (var it in a.Arr("items")) items.Add(ToItem(it));
                    Load(a.Int("index") ?? 0, a.Bool("play") ?? true);
                    reply(true);
                    break;
                case "audio.play": Resume(); reply(true); break;
                case "audio.pause": Pause(); reply(true); break;
                case "audio.toggle": if (playing) Pause(); else Resume(); reply(true); break;
                case "audio.next": Advance(); reply(true); break;
                case "audio.previous":
                    if (player.Position.TotalSeconds > 3) player.Position = TimeSpan.Zero;
                    else Load(Math.Max(0, index - 1), true);
                    reply(true);
                    break;
                case "audio.seek": player.Position = TimeSpan.FromMilliseconds(a.Int("ms") ?? 0); reply(true); break;
                case "audio.skipTo": Load(a.Int("index") ?? 0, true); reply(true); break;
                case "audio.shuffle": shuffle = a.Bool("on") ?? false; reply(true); break;
                case "audio.repeat": repeat = a.Str("mode") ?? "off"; reply(true); break;
                case "audio.speed":
                    speed = a.Num("value") ?? 1.0;
                    player.SpeedRatio = speed;
                    reply(true);
                    break;
                case "audio.stop":
                    player.Stop();
                    player.Close();
                    items.Clear();
                    index = -1;
                    playing = false;
                    ended = false;
                    durationMs = 0;
                    reply(true);
                    break;
                case "audio.sleep":
                    {
                        double minutes = a.Num("minutes") ?? 0;
                        if (minutes <= 0) { sleepEnd = 0; sleepTimer.Stop(); }
                        else { sleepEnd = Now() + (long)(minutes * 60000); sleepTimer.Start(); }
                        reply(true);
                        break;
                    }
                case "audio.queue.add":
                    {
                        var it = a.Obj("item");
                        if (it.HasValue)
                        {
                            var item = ToItem(it.Value);
                            if ((a.Bool("next") ?? false) && index >= 0 && index < items.Count) items.Insert(index + 1, item);
                            else items.Add(item);
                        }
                        reply(true);
                        break;
                    }
                case "audio.queue.remove":
                    {
                        int i = a.Int("index") ?? -1;
                        if (i >= 0 && i < items.Count && i != index)
                        {
                            items.RemoveAt(i);
                            if (i < index) index--;
                        }
                        reply(true);
                        break;
                    }
                case "audio.queue.move":
                    {
                        int from = a.Int("from") ?? -1, to = a.Int("to") ?? -1;
                        if (from >= 0 && from < items.Count && to >= 0 && to < items.Count && from != to)
                        {
                            var it = items[from];
                            items.RemoveAt(from);
                            items.Insert(to, it);
                            if (index == from) index = to;
                            else if (from < index && to >= index) index--;
                            else if (from > index && to <= index) index++;
                        }
                        reply(true);
                        break;
                    }
                case "audio.eq.get": reply(null); break;
                case "audio.eq.set": reply(null); break;
                case "audio.state": reply(State()); break;
                default: reply(null); break;
            }
            return true;
        }

        static Dictionary<string, object> ToItem(JsonElement it)
        {
            return new Dictionary<string, object>
            {
                ["id"] = it.Str("id") ?? "",
                ["uri"] = it.Str("uri") ?? "",
                ["title"] = it.Str("title") ?? "",
                ["artist"] = it.Str("artist") ?? "",
                ["album"] = it.Str("album") ?? "",
                ["albumId"] = it.Str("albumId") ?? "",
                ["durationMs"] = (long)(it.Num("durationMs") ?? 0),
            };
        }

        void Load(int i, bool play)
        {
            if (items.Count == 0) return;
            if (i >= items.Count)
            {
                if (repeat == "all") i = 0;
                else { player.Pause(); playing = false; ended = true; return; }
            }
            i = Math.Max(0, i);
            index = i;
            ended = false;
            string uri = (string)items[i]["uri"];
            Uri u;
            if (!Uri.TryCreate(uri, UriKind.Absolute, out u)) u = new Uri(Path.GetFullPath(uri));
            durationMs = (long)items[i]["durationMs"];
            player.Open(u);
            player.SpeedRatio = speed;
            if (play) { player.Play(); playing = true; }
            else { player.Pause(); playing = false; }
        }

        void Resume()
        {
            if (items.Count == 0) return;
            if (ended) { Load(0, true); return; }
            player.Play();
            player.SpeedRatio = speed;
            playing = true;
        }

        void Pause()
        {
            player.Pause();
            playing = false;
        }

        void Advance()
        {
            if (items.Count == 0) return;
            if (shuffle && items.Count > 1)
            {
                int n;
                do { n = rng.Next(items.Count); } while (n == index);
                Load(n, true);
            }
            else Load(index + 1, true);
        }

        void OnEnded()
        {
            if (repeat == "one") { Load(index, true); return; }
            Advance();
        }

        Dictionary<string, object> State()
        {
            double pos = 0;
            try { pos = player.Position.TotalMilliseconds; } catch (Exception) { }
            long left = sleepEnd > 0 ? Math.Max(0, sleepEnd - Now()) : 0;
            string mediaId = index >= 0 && index < items.Count ? (string)items[index]["id"] : "";
            return new Dictionary<string, object>
            {
                ["active"] = items.Count > 0,
                ["playing"] = playing,
                ["buffering"] = false,
                ["ended"] = ended,
                ["positionMs"] = (long)pos,
                ["durationMs"] = durationMs,
                ["index"] = items.Count == 0 ? -1 : index,
                ["count"] = items.Count,
                ["mediaId"] = mediaId,
                ["nextIndex"] = !shuffle && index + 1 < items.Count ? index + 1 : -1,
                ["shuffle"] = shuffle,
                ["repeat"] = repeat,
                ["speed"] = speed,
                ["sleepLeftMs"] = left,
            };
        }

        static string StableId(string s)
        {
            unchecked
            {
                ulong h = 1469598103934665603UL;
                foreach (char c in s) { h ^= c; h *= 1099511628211UL; }
                return (h & 0x7FFFFFFFFFFFFFFFUL).ToString();
            }
        }

        void Query(long minMs, Action<object> reply)
        {
            var folders = new List<string> { Environment.GetFolderPath(Environment.SpecialFolder.MyMusic) };
            Task.Run(() =>
            {
                var tracks = new List<object>();
                string error = null;
                try
                {
                    foreach (string folder in folders)
                    {
                        if (string.IsNullOrEmpty(folder) || !Directory.Exists(folder)) continue;
                        foreach (string path in EnumerateAudio(folder))
                        {
                            try { AddTrack(tracks, path, minMs); } catch (Exception) { }
                        }
                    }
                }
                catch (Exception e) { error = e.Message; }
                var res = new Dictionary<string, object> { ["tracks"] = tracks.OrderBy(t => (string)((Dictionary<string, object>)t)["title"], StringComparer.OrdinalIgnoreCase).ToList() };
                if (error != null) res["error"] = error;
                reply(res);
            });
        }

        static IEnumerable<string> EnumerateAudio(string root)
        {
            var stack = new Stack<string>();
            stack.Push(root);
            while (stack.Count > 0)
            {
                string dir = stack.Pop();
                string[] files = new string[0], subs = new string[0];
                try { files = Directory.GetFiles(dir); subs = Directory.GetDirectories(dir); } catch (Exception) { }
                foreach (string f in files)
                    if (Exts.Contains(Path.GetExtension(f).ToLowerInvariant())) yield return f;
                foreach (string d in subs) stack.Push(d);
            }
        }

        void AddTrack(List<object> tracks, string path, long minMs)
        {
            string title = Path.GetFileNameWithoutExtension(path), artist = "Unknown artist", album = "Unknown album";
            long dur = 0;
            int trackNo = 0, year = 0;
            try
            {
                using (var f = TagLib.File.Create(path))
                {
                    if (!string.IsNullOrWhiteSpace(f.Tag.Title)) title = f.Tag.Title;
                    string a = f.Tag.FirstAlbumArtist ?? f.Tag.FirstPerformer;
                    if (!string.IsNullOrWhiteSpace(a)) artist = a;
                    if (!string.IsNullOrWhiteSpace(f.Tag.Album)) album = f.Tag.Album;
                    dur = (long)f.Properties.Duration.TotalMilliseconds;
                    trackNo = (int)f.Tag.Track;
                    year = (int)f.Tag.Year;
                }
            }
            catch (Exception) { }
            if (dur < minMs) return;
            string albumId = StableId(album + "|" + artist);
            lock (albumTrack) { if (!albumTrack.ContainsKey(albumId)) albumTrack[albumId] = path; }
            tracks.Add(new Dictionary<string, object>
            {
                ["id"] = StableId(path),
                ["uri"] = path,
                ["title"] = title,
                ["artist"] = artist,
                ["album"] = album,
                ["albumId"] = albumId,
                ["artistId"] = StableId(artist),
                ["durationMs"] = dur,
                ["track"] = trackNo,
                ["year"] = year,
                ["added"] = new DateTimeOffset(File.GetLastWriteTimeUtc(path)).ToUnixTimeSeconds(),
                ["size"] = new FileInfo(path).Length,
            });
        }

        void Art(string albumId, Action<object> reply)
        {
            string src;
            lock (albumTrack) { albumTrack.TryGetValue(albumId, out src); }
            if (src == null) { reply(null); return; }
            Task.Run(() =>
            {
                try
                {
                    string dir = Path.Combine(ctx.DocsDir, "art");
                    Directory.CreateDirectory(dir);
                    string dest = Path.Combine(dir, albumId + ".jpg");
                    if (!File.Exists(dest))
                    {
                        using (var f = TagLib.File.Create(src))
                        {
                            if (f.Tag.Pictures.Length == 0) { reply(null); return; }
                            File.WriteAllBytes(dest, f.Tag.Pictures[0].Data.Data);
                        }
                    }
                    reply(new Dictionary<string, object> { ["path"] = dest });
                }
                catch (Exception) { reply(null); }
            });
        }
    }
}
