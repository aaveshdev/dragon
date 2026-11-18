using System;
using System.Runtime.InteropServices;

namespace DragonRuntime
{
    internal static class DragonNative
    {
        const string Lib = "dragon_lib";

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern IntPtr dgn_app_init([MarshalAs(UnmanagedType.LPUTF8Str)] string docsDir, [MarshalAs(UnmanagedType.LPUTF8Str)] string media);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern IntPtr dgn_app_render();
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_event([MarshalAs(UnmanagedType.LPUTF8Str)] string e);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_is_dirty();
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern long dgn_app_tick(long nowMs);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern IntPtr dgn_app_platform_calls();
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_platform_result([MarshalAs(UnmanagedType.LPUTF8Str)] string id, [MarshalAs(UnmanagedType.LPUTF8Str)] string json);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_back();
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_set_media([MarshalAs(UnmanagedType.LPUTF8Str)] string media);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int dgn_app_lifecycle([MarshalAs(UnmanagedType.LPUTF8Str)] string state);
        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern IntPtr dgn_app_last_error();

        static string S(IntPtr p) => p == IntPtr.Zero ? "" : Marshal.PtrToStringUTF8(p) ?? "";

        public static string Start(string docsDir, string media) => S(dgn_app_init(docsDir, media));
        public static string Render() => S(dgn_app_render());
        public static bool Event(string e) => dgn_app_event(e) != 0;
        public static bool IsDirty() => dgn_app_is_dirty() != 0;
        public static long Tick(long nowMs) => dgn_app_tick(nowMs);
        public static string PlatformCalls() => S(dgn_app_platform_calls());
        public static bool PlatformResult(string id, string json) => dgn_app_platform_result(id, json) != 0;
        public static bool Back() => dgn_app_back() != 0;
        public static bool SetMedia(string media) => dgn_app_set_media(media) != 0;
        public static bool Lifecycle(string state) => dgn_app_lifecycle(state) != 0;
        public static string LastError() => S(dgn_app_last_error());
    }
}
