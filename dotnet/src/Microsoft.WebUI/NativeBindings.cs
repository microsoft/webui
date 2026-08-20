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

    /// <summary>
    /// SafeHandle wrapper for one owned native streaming step.
    /// </summary>
    internal sealed class WebUIStreamingStepSafeHandle : SafeHandle
    {
        internal WebUIStreamingStepSafeHandle()
            : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        internal WebUIStreamingStepSafeHandle(IntPtr handle)
            : this()
        {
            SetHandle(handle);
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            webui_streaming_step_destroy_raw(handle);
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

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_streaming_session_start")]
    private static extern IntPtr webui_streaming_session_start_raw(
        WebUIStreamingSessionSafeHandle sessionPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_streaming_session_resume")]
    private static extern IntPtr webui_streaming_session_resume_raw(
        WebUIStreamingSessionSafeHandle sessionPtr,
        uint instanceId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson,
        uint mode);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_session_update(
        WebUIStreamingSessionSafeHandle sessionPtr,
        uint instanceId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string patchJson,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "webui_streaming_step_destroy")]
    private static extern void webui_streaming_step_destroy_raw(IntPtr stepPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_step_bytes(
        WebUIStreamingStepSafeHandle stepPtr,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_done(
        WebUIStreamingStepSafeHandle stepPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_has_boundary(
        WebUIStreamingStepSafeHandle stepPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_boundary_instance_id(
        WebUIStreamingStepSafeHandle stepPtr,
        out uint instanceId);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_boundary_declaration_id(
        WebUIStreamingStepSafeHandle stepPtr,
        out uint declarationId);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_step_boundary_owner(
        WebUIStreamingStepSafeHandle stepPtr,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_step_boundary_name(
        WebUIStreamingStepSafeHandle stepPtr,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_boundary_key_type(
        WebUIStreamingStepSafeHandle stepPtr,
        out uint keyType);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr webui_streaming_step_boundary_key_string(
        WebUIStreamingStepSafeHandle stepPtr,
        out nuint outLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static extern bool webui_streaming_step_boundary_key_number(
        WebUIStreamingStepSafeHandle stepPtr,
        out double value);

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

    internal static WebUIStreamingStepSafeHandle StartStreamingSession(
        WebUIStreamingSessionSafeHandle session,
        string stateJson)
    {
        IntPtr handle = webui_streaming_session_start_raw(session, stateJson);
        return new WebUIStreamingStepSafeHandle(handle);
    }

    internal static WebUIStreamingStepSafeHandle ResumeStreamingSession(
        WebUIStreamingSessionSafeHandle session,
        uint instanceId,
        string stateJson,
        uint mode)
    {
        IntPtr handle = webui_streaming_session_resume_raw(
            session,
            instanceId,
            stateJson,
            mode);
        return new WebUIStreamingStepSafeHandle(handle);
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
            return CopyBytes(ptr, length);
        }
        finally
        {
            webui_free(ptr);
        }
    }

    /// <summary>
    /// Copies bytes borrowed from a live native owner without freeing the pointer.
    /// </summary>
    internal static byte[]? ReadBorrowedBytes(IntPtr ptr, nuint length)
    {
        return ptr == IntPtr.Zero ? null : CopyBytes(ptr, length);
    }

    /// <summary>
    /// Copies a length-delimited UTF-8 string borrowed from a live native owner.
    /// </summary>
    internal static string? ReadBorrowedString(IntPtr ptr, nuint length)
    {
        if (ptr == IntPtr.Zero)
        {
            return null;
        }

        if (length > int.MaxValue)
        {
            throw new WebUIException("Native UTF-8 value exceeds the managed string limit.");
        }

        return length == 0
            ? string.Empty
            : Marshal.PtrToStringUTF8(ptr, checked((int)length));
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

    private static byte[] CopyBytes(IntPtr ptr, nuint length)
    {
        if (length == 0)
        {
            return Array.Empty<byte>();
        }

        if (length > int.MaxValue)
        {
            throw new WebUIException("Native byte payload exceeds the managed array limit.");
        }

        int count = checked((int)length);
        byte[] bytes = new byte[count];
        Marshal.Copy(ptr, bytes, 0, count);
        return bytes;
    }
}
