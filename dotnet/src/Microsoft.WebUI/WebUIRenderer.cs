// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;

namespace Microsoft.WebUI;

/// <summary>
/// Static helpers for one-shot WebUI rendering (parse + render in a single call).
/// </summary>
public static class WebUIRenderer
{
    /// <summary>
    /// Parses and renders an HTML template with the given JSON state in a single call.
    /// </summary>
    /// <param name="html">The HTML template string to render.</param>
    /// <param name="stateJson">JSON-encoded state for the render.</param>
    /// <returns>The rendered HTML string.</returns>
    /// <exception cref="WebUIException">Thrown when rendering fails.</exception>
    public static string RenderHtml(string html, string stateJson)
    {
        ArgumentNullException.ThrowIfNull(html);
        ArgumentNullException.ThrowIfNull(stateJson);
        return RenderHtmlNative(html, stateJson);
    }

    /// <summary>
    /// Internal helper that performs the native WebUI render call and marshals the result.
    /// </summary>
    /// <param name="html">The HTML template string to render.</param>
    /// <param name="stateJson">JSON-encoded state for the render.</param>
    /// <returns>The rendered HTML string.</returns>
    /// <exception cref="WebUIException">Thrown when rendering fails.</exception>
    private static string RenderHtmlNative(string html, string stateJson)
    {
        IntPtr resultPtr = NativeBindings.webui_render(html, stateJson);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "RenderHtml failed.";
            throw new WebUIException(error);
        }

        return NativeBindings.ReadAndFreeString(resultPtr)!;
    }
}
