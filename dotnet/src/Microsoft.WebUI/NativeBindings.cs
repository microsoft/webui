using System;
using System.Reflection;
using System.Runtime.InteropServices;

namespace Microsoft.WebUI;

/// <summary>
/// Internal P/Invoke bindings to the native <c>webui_ffi</c> library.
/// </summary>
internal static class NativeBindings
{
    private const string LibName = "webui_ffi";

    static NativeBindings()
    {
        NativeLibrary.SetDllImportResolver(
            typeof(NativeBindings).Assembly,
            ResolveNativeLibrary);
    }

    private static IntPtr ResolveNativeLibrary(
        string libraryName,
        Assembly assembly,
        DllImportSearchPath? searchPath)
    {
        if (libraryName != LibName)
        {
            return IntPtr.Zero;
        }

        // Allow overriding the native library path via environment variable.
        string? customPath = Environment.GetEnvironmentVariable("WEBUI_LIB_PATH");
        if (!string.IsNullOrEmpty(customPath) &&
            NativeLibrary.TryLoad(customPath, out IntPtr handle))
        {
            return handle;
        }

        // Fall back to default resolution.
        if (NativeLibrary.TryLoad(LibName, assembly, searchPath, out handle))
        {
            return handle;
        }

        return IntPtr.Zero;
    }

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_handler_create();

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_handler_create_with_plugin(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? pluginId);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void webui_handler_destroy(IntPtr handlerPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_handler_render(
        IntPtr handlerPtr,
        byte[] protocolData,
        nuint protocolLen,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_render(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string html,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_get_route_templates(
        byte[] protocolData,
        nuint protocolLen,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string inventoryHex);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void webui_free(IntPtr stringPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_last_error();

    /// <summary>
    /// Reads a UTF-8 string from a native pointer and frees the native memory.
    /// Returns <c>null</c> if the pointer is <see cref="System.IntPtr.Zero"/>.
    /// </summary>
    internal static string? ReadAndFreeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero)
        {
            return null;
        }

        try
        {
            return Marshal.PtrToStringUTF8(ptr);
        }
        finally
        {
            webui_free(ptr);
        }
    }

    /// <summary>
    /// Reads the last error message from the native library.
    /// Returns <c>null</c> if there is no error.
    /// </summary>
    internal static string? GetLastError()
    {
        IntPtr errorPtr = webui_last_error();
        if (errorPtr == IntPtr.Zero)
        {
            return null;
        }

        return Marshal.PtrToStringUTF8(errorPtr);
    }
}
