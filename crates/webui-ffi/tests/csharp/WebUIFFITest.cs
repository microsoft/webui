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
using System.IO;
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
    public static extern IntPtr webui_handler_render(
        IntPtr handlerPtr,
        IntPtr protocolData,
        UIntPtr protocolLen,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string requestPath);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_render(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? html,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? dataJson);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void webui_free(IntPtr stringPtr);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr webui_last_error();

    /// <summary>
    /// Call webui_render, marshal the result to a managed string,
    /// and free the native memory.
    /// </summary>
    public static string ParseAndRender(string html, string dataJson)
    {
        IntPtr ptr = webui_render(html, dataJson);
        if (ptr == IntPtr.Zero)
        {
            string? err = GetLastError();
            throw new InvalidOperationException(
                $"webui_render failed: {err ?? "<no error>"}");
        }

        string result = Marshal.PtrToStringUTF8(ptr) ?? "";
        webui_free(ptr);
        return result;
    }

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
// Tests: happy paths
// ---------------------------------------------------------------------------

public class ParseAndRenderTests
{
    [Fact]
    public void SimplePassthrough()
    {
        string result = WebUIFFI.ParseAndRender("<p>Hello</p>", "{}");
        Assert.Equal("<p>Hello</p>", result);
    }

    [Fact]
    public void SignalSubstitution()
    {
        string result = WebUIFFI.ParseAndRender("Hello, {{name}}!", "{\"name\":\"WebUI\"}");
        Assert.Equal("Hello, WebUI!", result);
    }

    [Fact]
    public void ForLoop()
    {
        string html = "<ul><for each=\"item in items\"><li>{{item}}</li></for></ul>";
        string result = WebUIFFI.ParseAndRender(html, "{\"items\":[\"a\",\"b\",\"c\"]}");
        Assert.Equal("<ul><li>a</li><li>b</li><li>c</li></ul>", result);
    }

    [Fact]
    public void IfConditionTrue()
    {
        string html = "<if condition=\"show\"><p>Visible</p></if>";
        string result = WebUIFFI.ParseAndRender(html, "{\"show\":true}");
        Assert.Equal("<p>Visible</p>", result);
    }

    [Fact]
    public void IfConditionFalse()
    {
        string html = "<if condition=\"show\"><p>Hidden</p></if>";
        string result = WebUIFFI.ParseAndRender(html, "{\"show\":false}");
        Assert.Equal("", result);
    }

    [Fact]
    public void HtmlEscaping()
    {
        string html = "<div>{{content}}</div>";
        string json = "{\"content\":\"<script>alert('xss')</script>\"}";
        string result = WebUIFFI.ParseAndRender(html, json);
        Assert.DoesNotContain("<script>", result);
        Assert.Contains("&lt;script&gt;", result);
    }

    [Fact]
    public void RawSignalUnescaped()
    {
        string result = WebUIFFI.ParseAndRender(
            "<div>{{{content}}}</div>",
            "{\"content\":\"<b>bold</b>\"}");
        Assert.Equal("<div><b>bold</b></div>", result);
    }

    [Fact]
    public void EmptyData()
    {
        string result = WebUIFFI.ParseAndRender("<p>static</p>", "{}");
        Assert.Equal("<p>static</p>", result);
    }
}

// ---------------------------------------------------------------------------
// Tests: error cases
// ---------------------------------------------------------------------------

public class ErrorHandlingTests
{
    [Fact]
    public void NullHtml()
    {
        IntPtr ptr = WebUIFFI.webui_render(null, "{}");
        Assert.Equal(IntPtr.Zero, ptr);

        string? err = WebUIFFI.GetLastError();
        Assert.NotNull(err);
        Assert.Contains("null", err);
    }

    [Fact]
    public void NullJson()
    {
        IntPtr ptr = WebUIFFI.webui_render("<p>hi</p>", null);
        Assert.Equal(IntPtr.Zero, ptr);
        Assert.NotNull(WebUIFFI.GetLastError());
    }

    [Fact]
    public void InvalidJson()
    {
        IntPtr ptr = WebUIFFI.webui_render("<p>hi</p>", "NOT JSON");
        Assert.Equal(IntPtr.Zero, ptr);

        string? err = WebUIFFI.GetLastError();
        Assert.NotNull(err);
        Assert.Contains("JSON", err);
    }

    [Fact]
    public void SuccessfulCallClearsError()
    {
        // Trigger error
        WebUIFFI.webui_render("<p>hi</p>", "NOT JSON");
        Assert.NotNull(WebUIFFI.GetLastError());

        // Successful call clears it
        WebUIFFI.ParseAndRender("<p>ok</p>", "{}");
        Assert.Null(WebUIFFI.GetLastError());
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

        IntPtr ptr = WebUIFFI.webui_handler_render(handler, IntPtr.Zero, UIntPtr.Zero, "{}", "index.html", "/");
        Assert.Equal(IntPtr.Zero, ptr);
        Assert.NotNull(WebUIFFI.GetLastError());

        WebUIFFI.webui_handler_destroy(handler);
    }
}

// ---------------------------------------------------------------------------
// Tests: fixture file
// ---------------------------------------------------------------------------

public class FixtureTests
{
    [Fact]
    public void FixtureFileRendersCorrectly()
    {
        // Navigate up from the test project directory to the fixtures dir
        string testDir = AppContext.BaseDirectory;
        // Walk up to find the crates/webui-ffi/tests directory
        string? current = testDir;
        string fixturesDir = "";
        // Try a known relative path from test output dir
        // The test runs from bin/Debug/net8.0/ relative to the .csproj dir
        string csprojDir = Path.GetFullPath(
            Path.Combine(testDir, "..", "..", ".."));
        fixturesDir = Path.Combine(csprojDir, "..", "fixtures");

        if (!Directory.Exists(fixturesDir))
        {
            // Fallback: search upward
            current = testDir;
            for (int i = 0; i < 10 && current != null; i++)
            {
                string candidate = Path.Combine(current, "crates", "webui-ffi", "tests", "fixtures");
                if (Directory.Exists(candidate))
                {
                    fixturesDir = candidate;
                    break;
                }
                current = Path.GetDirectoryName(current);
            }
        }

        string html = File.ReadAllText(Path.Combine(fixturesDir, "simple.html"));
        string state = File.ReadAllText(Path.Combine(fixturesDir, "state.json"));
        string expected = File.ReadAllText(Path.Combine(fixturesDir, "expected_output.html"));

        string result = WebUIFFI.ParseAndRender(html, state);
        Assert.Equal(expected, result);
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
