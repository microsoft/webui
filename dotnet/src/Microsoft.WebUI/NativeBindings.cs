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

    /// <summary>
    /// SafeHandle wrapper for a host-driven native streaming session.
    /// </summary>
    internal sealed class WebUIStreamingSessionSafeHandle : SafeHandle
    {
        internal WebUIStreamingSessionSafeHandle()
            : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        internal WebUIStreamingSessionSafeHandle(IntPtr handle)
            : this()
        {
            SetHandle(handle);
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            webui_streaming_session_destroy_raw(handle);
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

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_streaming_session_create")]
    private static extern IntPtr webui_streaming_session_create_raw(
        WebUIHandlerSafeHandle handlerPtr,
        WebUIProtocolSafeHandle protocolPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_streaming_session_destroy")]
    private static extern void webui_streaming_session_destroy_raw(IntPtr sessionPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_session_boundary(
        WebUIStreamingSessionSafeHandle sessionPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        out uint boundary);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint webui_streaming_session_boundary_count(
        WebUIStreamingSessionSafeHandle sessionPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_session_is_finished(
        WebUIStreamingSessionSafeHandle sessionPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_session_write_shell(
        WebUIStreamingSessionSafeHandle sessionPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_session_write_boundary(
        WebUIStreamingSessionSafeHandle sessionPtr,
        uint boundary,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        [MarshalAs(UnmanagedType.U1)] bool updatable,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_session_update(
        WebUIStreamingSessionSafeHandle sessionPtr,
        uint boundary,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_session_finish(
        WebUIStreamingSessionSafeHandle sessionPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        out nuint outLen);

    internal static WebUIStreamingSessionSafeHandle CreateStreamingSession(
        WebUIHandlerSafeHandle handler,
        WebUIProtocolSafeHandle protocol,
        string entryId,
        string requestPath)
    {
        IntPtr handle = webui_streaming_session_create_raw(
            handler,
            protocol,
            entryId,
            requestPath);
        return new WebUIStreamingSessionSafeHandle(handle);
    }

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
    /// Copies a native UTF-8 chunk of a known length into a managed array and
    /// frees the native memory. Returns <c>null</c> for
    /// <see cref="System.IntPtr.Zero"/>.
    /// </summary>
    /// <remarks>
    /// The length comes from the native call, so this never scans for a
    /// terminator on the response hot path.
    /// </remarks>
    internal static byte[]? ReadAndFreeBytes(IntPtr ptr, nuint length)
    {
        if (ptr == IntPtr.Zero)
        {
            return null;
        }

        try
        {
            if (length == 0)
            {
                return Array.Empty<byte>();
            }

            byte[] bytes = new byte[length];
            Marshal.Copy(ptr, bytes, 0, (int)length);
            return bytes;
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
