using System;
using System.Runtime.InteropServices;
using System.Threading;

namespace Microsoft.WebUI;

/// <summary>
/// Manages a WebUI handler instance for protocol-based rendering.
/// Use this for repeated renders with pre-compiled protocol data.
/// <para>This type is thread-safe. The native handler creates per-render state
/// internally, so concurrent renders do not contend.</para>
/// </summary>
public sealed class WebUIHandler : IDisposable
{
    private IntPtr _handle;
    private volatile int _disposed;

    /// <summary>
    /// Creates a new WebUI handler instance.
    /// </summary>
    /// <param name="plugin">Optional plugin identifier (e.g., "fast" for FAST-HTML hydration).</param>
    /// <exception cref="WebUIException">Thrown when the native handler cannot be created.</exception>
    public WebUIHandler(string? plugin = null)
    {
        string? normalizedPlugin = string.IsNullOrWhiteSpace(plugin) ? null : plugin.Trim();

        _handle = normalizedPlugin is null
            ? NativeBindings.webui_handler_create()
            : NativeBindings.webui_handler_create_with_plugin(normalizedPlugin);

        if (_handle == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "Failed to create WebUI handler.";
            throw new WebUIException(error);
        }
    }

    /// <summary>
    /// Renders the given protocol data with the specified state, entry, and request path.
    /// </summary>
    /// <param name="protocol">Pre-compiled protocol binary data.</param>
    /// <param name="stateJson">JSON-encoded state for the render.</param>
    /// <param name="entryId">The entry identifier to render.</param>
    /// <param name="requestPath">The HTTP request path.</param>
    /// <returns>The rendered HTML string.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the handler has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when rendering fails.</exception>
    public string Render(byte[] protocol, string stateJson, string entryId, string requestPath)
    {
        ObjectDisposedException.ThrowIf(_disposed != 0, this);
        ArgumentNullException.ThrowIfNull(protocol);
        ArgumentNullException.ThrowIfNull(stateJson);
        ArgumentNullException.ThrowIfNull(entryId);
        ArgumentNullException.ThrowIfNull(requestPath);

        IntPtr resultPtr = NativeBindings.webui_handler_render(
            _handle,
            protocol,
            (nuint)protocol.Length,
            stateJson,
            entryId,
            requestPath);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "Render failed.";
            throw new WebUIException(error);
        }

        return NativeBindings.ReadAndFreeString(resultPtr)!;
    }

    /// <summary>
    /// Returns the route templates for the active route chain.
    /// </summary>
    /// <param name="protocol">Pre-compiled protocol binary data.</param>
    /// <param name="entryId">The persistent entry identifier.</param>
    /// <param name="requestPath">The current route path.</param>
    /// <param name="inventoryHex">Hex-encoded inventory string.</param>
    /// <returns>A JSON string containing the route templates.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the handler has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when the operation fails.</exception>
    public string GetRouteTemplates(byte[] protocol, string entryId, string requestPath, string inventoryHex)
    {
        ObjectDisposedException.ThrowIf(_disposed != 0, this);
        ArgumentNullException.ThrowIfNull(protocol);
        ArgumentNullException.ThrowIfNull(entryId);
        ArgumentNullException.ThrowIfNull(requestPath);
        ArgumentNullException.ThrowIfNull(inventoryHex);

        IntPtr resultPtr = NativeBindings.webui_get_route_templates(
            protocol,
            (nuint)protocol.Length,
            entryId,
            requestPath,
            inventoryHex);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "GetRouteTemplates failed.";
            throw new WebUIException(error);
        }

        return NativeBindings.ReadAndFreeString(resultPtr)!;
    }

    /// <summary>
    /// Releases the native handler resources.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.CompareExchange(ref _disposed, 1, 0) != 0) return;

        if (_handle != IntPtr.Zero)
        {
            NativeBindings.webui_handler_destroy(_handle);
            _handle = IntPtr.Zero;
        }
    }
}
