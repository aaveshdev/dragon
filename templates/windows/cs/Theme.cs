using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;
using System.Windows;
using System.Windows.Media;

namespace DragonRuntime
{
    internal static class Theme
    {
        public static bool Dark;
        public static Color Background = Color.FromRgb(0xFF, 0xFB, 0xFE);
        public static Color Surface = Color.FromRgb(0xFF, 0xFB, 0xFE);
        public static Color OnSurface = Color.FromRgb(0x1C, 0x1B, 0x1F);
        public static Color SurfaceVariant = Color.FromRgb(0xE7, 0xE0, 0xEC);
        public static Color Primary = Color.FromRgb(0x67, 0x50, 0xA4);
        public static Color OnPrimary = Colors.White;
        public static Color PrimaryContainer = Color.FromRgb(0xEA, 0xDD, 0xFF);
        public static Color Outline = Color.FromRgb(0x79, 0x74, 0x7E);
        public static Color Error = Color.FromRgb(0xB3, 0x26, 0x1E);

        public static readonly FontFamily Text = new FontFamily("Segoe UI Variable, Segoe UI");
        public static FontFamily Icons;

        public static Color? ParseColor(string s)
        {
            if (string.IsNullOrEmpty(s) || s[0] != '#') return null;
            string h = s.Substring(1);
            if (!uint.TryParse(h, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out uint v)) return null;
            if (h.Length == 8) return Color.FromArgb((byte)(v >> 24), (byte)(v >> 16), (byte)(v >> 8), (byte)v);
            if (h.Length == 6) return Color.FromRgb((byte)(v >> 16), (byte)(v >> 8), (byte)v);
            return null;
        }

        public static Brush BrushOf(string s)
        {
            var c = ParseColor(s);
            if (!c.HasValue) return null;
            var b = new SolidColorBrush(c.Value);
            b.Freeze();
            return b;
        }

        public static Brush Solid(Color c)
        {
            var b = new SolidColorBrush(c);
            b.Freeze();
            return b;
        }

        public static void Apply(JsonElement theme, bool systemDark)
        {
            Dark = theme.Bool("dark") ?? systemDark;
            var scheme = theme.Obj("scheme");
            Surface = Dark ? Color.FromRgb(0x14, 0x13, 0x18) : Color.FromRgb(0xFF, 0xFB, 0xFE);
            OnSurface = Dark ? Color.FromRgb(0xE6, 0xE1, 0xE5) : Color.FromRgb(0x1C, 0x1B, 0x1F);
            Primary = Dark ? Color.FromRgb(0xD0, 0xBC, 0xFF) : Color.FromRgb(0x67, 0x50, 0xA4);
            OnPrimary = Dark ? Color.FromRgb(0x38, 0x1E, 0x72) : Colors.White;
            PrimaryContainer = Dark ? Color.FromRgb(0x4F, 0x37, 0x8B) : Color.FromRgb(0xEA, 0xDD, 0xFF);
            SurfaceVariant = Dark ? Color.FromRgb(0x49, 0x45, 0x4F) : Color.FromRgb(0xE7, 0xE0, 0xEC);
            Outline = Dark ? Color.FromRgb(0x93, 0x8F, 0x99) : Color.FromRgb(0x79, 0x74, 0x7E);
            Error = Dark ? Color.FromRgb(0xF2, 0xB8, 0xB5) : Color.FromRgb(0xB3, 0x26, 0x1E);
            if (scheme.HasValue)
            {
                var s = scheme.Value;
                Surface = ParseColor(s.Str("surface")) ?? Surface;
                OnSurface = ParseColor(s.Str("onSurface")) ?? OnSurface;
                Primary = ParseColor(s.Str("primary")) ?? Primary;
                OnPrimary = ParseColor(s.Str("onPrimary")) ?? OnPrimary;
                PrimaryContainer = ParseColor(s.Str("primaryContainer")) ?? PrimaryContainer;
                SurfaceVariant = ParseColor(s.Str("surfaceVariant")) ?? SurfaceVariant;
                Outline = ParseColor(s.Str("outline")) ?? Outline;
                Error = ParseColor(s.Str("error")) ?? Error;
            }
            Background = ParseColor(theme.Str("background")) ?? (scheme.HasValue ? ParseColor(scheme.Value.Str("background")) : null) ?? Surface;
        }
    }
}
