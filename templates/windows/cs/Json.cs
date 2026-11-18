using System.Collections.Generic;
using System.Text.Json;

namespace DragonRuntime
{
    internal static class J
    {
        public static bool Has(this JsonElement e, string k) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(k, out var v) && v.ValueKind != JsonValueKind.Null;

        public static JsonElement Get(this JsonElement e, string k) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(k, out var v) ? v : default;

        public static string Str(this JsonElement e, string k)
        {
            if (!e.Has(k)) return null;
            var v = e.GetProperty(k);
            return v.ValueKind == JsonValueKind.String ? v.GetString() : v.ToString();
        }

        public static double? Num(this JsonElement e, string k)
        {
            if (!e.Has(k)) return null;
            var v = e.GetProperty(k);
            if (v.ValueKind == JsonValueKind.Number) return v.GetDouble();
            return null;
        }

        public static int? Int(this JsonElement e, string k)
        {
            var d = e.Num(k);
            return d.HasValue ? (int?)(int)d.Value : null;
        }

        public static bool? Bool(this JsonElement e, string k)
        {
            if (!e.Has(k)) return null;
            var v = e.GetProperty(k);
            if (v.ValueKind == JsonValueKind.True) return true;
            if (v.ValueKind == JsonValueKind.False) return false;
            return null;
        }

        public static JsonElement? Obj(this JsonElement e, string k)
        {
            if (!e.Has(k)) return null;
            var v = e.GetProperty(k);
            return v.ValueKind == JsonValueKind.Object ? v : (JsonElement?)null;
        }

        public static List<JsonElement> Arr(this JsonElement e, string k)
        {
            var list = new List<JsonElement>();
            if (!e.Has(k)) return list;
            var v = e.GetProperty(k);
            if (v.ValueKind != JsonValueKind.Array) return list;
            foreach (var x in v.EnumerateArray()) list.Add(x);
            return list;
        }

        public static bool IsObj(this JsonElement e) => e.ValueKind == JsonValueKind.Object;
    }
}
