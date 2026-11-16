// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System.Globalization;
using System.Net;
using System.Text;
using Microsoft.WebUI;

try
{
    // The launcher preserves the application root as cwd, not server/.
    string root = Directory.GetCurrentDirectory();
    var assets = PublicAssets.Load(Path.Combine(root, "dist", "public"));
    using var protocol = new Protocol(await File.ReadAllBytesAsync(Path.Combine(root, "dist", "protocol.bin")));
    using var handler = new WebUIHandler("webui");
    var builder = WebApplication.CreateSlimBuilder(args);
    string host = Environment.GetEnvironmentVariable("HOST") ?? "127.0.0.1";
    string portText = Environment.GetEnvironmentVariable("PORT") ?? "3000";
    if ((!IPAddress.TryParse(host, out _) && host != "localhost") ||
        !int.TryParse(portText, NumberStyles.None, CultureInfo.InvariantCulture, out int port) ||
        port is < 1 or > 65535)
        throw new InvalidOperationException("Set HOST to an IP address (or localhost) and PORT to 1-65535.");
    builder.WebHost.UseUrls($"http://{(host.Contains(':') ? $"[{host}]" : host)}:{port}");
    builder.WebHost.ConfigureKestrel(options =>
    {
        options.Limits.MaxRequestBodySize = 0;
        options.Limits.MaxRequestHeadersTotalSize = 16 * 1024;
        options.Limits.RequestHeadersTimeout = TimeSpan.FromSeconds(10);
    });
    await using var app = builder.Build();
    app.Run(async context =>
    {
        bool head = HttpMethods.IsHead(context.Request.Method);
        context.Response.Headers.XContentTypeOptions = "nosniff";
        if (!head && !HttpMethods.IsGet(context.Request.Method))
        {
            context.Response.Headers.Allow = "GET, HEAD";
            await Reply(context, 405, "text/plain; charset=utf-8", "Method not allowed\n"u8.ToArray());
            return;
        }
        string path = context.Request.Path.Value ?? "/";
        if (path == "/" || path == "/about")
        {
            try
            {
                string state = path == "/"
                    ? """{"title":"Hello WebUI","count":0,"ready":false,"isHome":true}"""
                    : """{"title":"About WebUI","isHome":false}""";
                byte[] html = Encoding.UTF8.GetBytes(handler.Render(protocol, state, "index.html", path));
                await Reply(context, 200, "text/html; charset=utf-8", html);
            }
            catch (WebUIException error)
            {
                app.Logger.LogError(error, "WebUI render failed for {Path}", path);
                await Reply(context, 500, "text/plain; charset=utf-8", "Internal server error\n"u8.ToArray());
            }
        }
        else if (assets.TryGetValue(path, out var asset))
            await Reply(context, 200, asset.Mime, asset.Bytes);
        else
            await Reply(context, 404, "text/plain; charset=utf-8", "Not found\n"u8.ToArray());
    });
    // RunAsync drains requests before the native using declarations are disposed.
    await app.RunAsync();
    return 0;
}
catch (Exception error)
{
    Console.Error.WriteLine($"Unable to start WebUI: {error}\nRun the app build first; verify the matching Microsoft.WebUI runtime package is installed.");
    return 1;
}

static async Task Reply(HttpContext context, int status, string mime, byte[] bytes)
{
    context.Response.StatusCode = status;
    context.Response.ContentType = mime;
    context.Response.ContentLength = bytes.Length;
    if (!HttpMethods.IsHead(context.Request.Method))
        await context.Response.Body.WriteAsync(bytes, context.RequestAborted);
}

// Snapshot only public regular files. Requests never open paths, follow links,
// discover files, or observe half-published rebuilds. Restart after a build.
internal static class PublicAssets
{
    internal sealed record Asset(string Mime, byte[] Bytes);
    internal static readonly Dictionary<string, string> Mimes = new(StringComparer.OrdinalIgnoreCase)
    {
        [".js"] = "text/javascript; charset=utf-8", [".css"] = "text/css; charset=utf-8",
        [".json"] = "application/json", [".txt"] = "text/plain; charset=utf-8",
        [".svg"] = "image/svg+xml", [".png"] = "image/png", [".jpg"] = "image/jpeg",
        [".jpeg"] = "image/jpeg", [".gif"] = "image/gif", [".webp"] = "image/webp",
        [".avif"] = "image/avif", [".ico"] = "image/x-icon",
        [".woff"] = "font/woff", [".woff2"] = "font/woff2"
    };

    internal static Dictionary<string, Asset> Load(string root)
    {
        // Check ancestors too: dist/public itself must not redirect to private files.
        for (DirectoryInfo? ancestor = new(root); ancestor is not null; ancestor = ancestor.Parent)
            if ((ancestor.Attributes & FileAttributes.ReparsePoint) != 0)
                throw new IOException($"Public asset directory must not contain symlink ancestors: {ancestor.FullName}");
        var assets = new Dictionary<string, Asset>(StringComparer.Ordinal);
        var directories = new Stack<string>();
        directories.Push(root);
        long total = 0;
        while (directories.TryPop(out string? directory))
        {
            foreach (string path in Directory.EnumerateFileSystemEntries(directory))
            {
                string name = Path.GetFileName(path);
                FileAttributes attributes = File.GetAttributes(path);
                if (name.StartsWith('.') || (attributes & FileAttributes.ReparsePoint) != 0)
                    continue;
                if ((attributes & FileAttributes.Directory) != 0)
                {
                    if (directories.Count >= 4096) throw new IOException("Too many public asset directories.");
                    directories.Push(path);
                    continue;
                }
                if (!Mimes.TryGetValue(Path.GetExtension(path), out string? mime)) continue;
                using var file = File.OpenRead(path);
                if (file.Length > 16 * 1024 * 1024 || total + file.Length > 64 * 1024 * 1024 || assets.Count >= 4096)
                    throw new IOException("Public assets exceed starter limits (16 MiB/file, 64 MiB total, 4096 files).");
                byte[] bytes = new byte[checked((int)file.Length)];
                file.ReadExactly(bytes);
                if (file.ReadByte() != -1) throw new IOException($"Public asset changed during startup: {path}");
                total += bytes.Length;
                string url = "/assets/" + Path.GetRelativePath(root, path).Replace(Path.DirectorySeparatorChar, '/');
                assets.Add(url, new Asset(mime, bytes));
            }
        }
        return assets;
    }
}
