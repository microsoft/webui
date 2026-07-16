// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Integration tests for the webui-ffi shared library via P/Invoke.
//
// Usage (macOS):
//   cd crates/webui-ffi/tests/csharp
//   DYLD_LIBRARY_PATH=../../../../target/debug dotnet test
//
// Usage (Linux):
//   cd crates/webui-ffi/tests/csharp
//   LD_LIBRARY_PATH=../../../../target/debug dotnet test

using System;
using System.Runtime.InteropServices;
using Xunit;

namespace WebUIFFITest;

/// <summary>
/// P/Invoke declarations for the webui_ffi shared library.
/// </summary>
internal static class WebUIFFI
{
    private const string LibName = "webui_ffi";

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_handler_create();

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void webui_handler_destroy(IntPtr handlerPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_protocol_create(
        IntPtr protocolData,
        UIntPtr protocolLen);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void webui_protocol_destroy(IntPtr protocolPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_handler_render(
        IntPtr handlerPtr,
        IntPtr protocolPtr,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void webui_free(IntPtr stringPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_last_error();

    /// <summary>
    /// Return the last error message, or null if none.
    /// </summary>
    public static string? GetLastError()
    {
        IntPtr ptr = webui_last_error();
        if (ptr == IntPtr.Zero)
            return null;
        return Marshal.PtrToStringUTF8(ptr);
    }
}

// ---------------------------------------------------------------------------
// Tests: handler lifecycle
// ---------------------------------------------------------------------------

public class HandlerLifecycleTests
{
    [Fact]
    public void CreateAndDestroy()
    {
        IntPtr handler = WebUIFFI.webui_handler_create();
        Assert.NotEqual(IntPtr.Zero, handler);
        WebUIFFI.webui_handler_destroy(handler);
    }

    [Fact]
    public void DestroyNull()
    {
        WebUIFFI.webui_handler_destroy(IntPtr.Zero); // should not crash
    }

    [Fact]
    public void RenderNullArgs()
    {
        IntPtr handler = WebUIFFI.webui_handler_create();

        IntPtr ptr = WebUIFFI.webui_handler_render(
            handler, IntPtr.Zero, "{}", "index.html", "/");
        Assert.Equal(IntPtr.Zero, ptr);
        Assert.NotNull(WebUIFFI.GetLastError());

        WebUIFFI.webui_handler_destroy(handler);
    }
}

// ---------------------------------------------------------------------------
// Tests: free string
// ---------------------------------------------------------------------------

public class FreeStringTests
{
    [Fact]
    public void FreeNull()
    {
        WebUIFFI.webui_free(IntPtr.Zero); // should not crash
    }
}
