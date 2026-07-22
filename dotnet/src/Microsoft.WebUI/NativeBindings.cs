// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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

    /// <summary>
    /// SafeHandle wrapper for a native <c>webui_handler</c> pointer.
    /// </summary>
    internal sealed class WebUIHandlerSafeHandle : SafeHandle
    {
        internal WebUIHandlerSafeHandle()
            : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        internal WebUIHandlerSafeHandle(IntPtr handle)
            : this()
        {
            SetHandle(handle);
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            webui_handler_destroy_raw(handle);
            return true;
        }
    }

    /// <summary>
    /// SafeHandle wrapper for a loaded native WebUI protocol.
    /// </summary>
    internal sealed class WebUIProtocolSafeHandle : SafeHandle
    {
        internal WebUIProtocolSafeHandle()
            : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        internal WebUIProtocolSafeHandle(IntPtr handle)
            : this()
        {
            SetHandle(handle);
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            webui_protocol_destroy_raw(handle);
            return true;
        }
    }

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

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_handler_create")]
    private static extern IntPtr webui_handler_create_raw();

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_handler_create_with_plugin")]
    private static extern IntPtr webui_handler_create_with_plugin_raw(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? pluginId);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_handler_destroy")]
    private static extern void webui_handler_destroy_raw(IntPtr handlerPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_protocol_create")]
    private static extern IntPtr webui_protocol_create_raw(
        byte[] protocolData,
        nuint protocolLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_protocol_destroy")]
    private static extern void webui_protocol_destroy_raw(IntPtr protocolPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_handler_render(
        WebUIHandlerSafeHandle handlerPtr,
        WebUIProtocolSafeHandle protocolPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_protocol_render_partial(
        WebUIProtocolSafeHandle protocolPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string inventoryHex);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_protocol_render_component_templates(
        WebUIProtocolSafeHandle protocolPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string componentTagsJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string inventoryHex);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_protocol_tokens(
        WebUIProtocolSafeHandle protocolPtr);

    internal static WebUIHandlerSafeHandle CreateHandler(string? pluginId)
    {
        IntPtr handle = pluginId is null
            ? webui_handler_create_raw()
            : webui_handler_create_with_plugin_raw(pluginId);
        return new WebUIHandlerSafeHandle(handle);
    }

    internal static WebUIProtocolSafeHandle CreateProtocol(byte[] protocolData)
    {
        IntPtr handle = webui_protocol_create_raw(protocolData, (nuint)protocolData.Length);
        return new WebUIProtocolSafeHandle(handle);
    }

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
