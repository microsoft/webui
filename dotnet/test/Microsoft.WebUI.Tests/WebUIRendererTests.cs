using Xunit;

namespace Microsoft.WebUI.Tests;

public class WebUIRendererTests
{
    [Fact]
    public void RenderHtml_SimpleTemplate_ReturnsRenderedOutput()
    {
        // This test requires the native library to be built.
        // Run: cargo build --release -p webui-ffi
        // Then set WEBUI_LIB_PATH to the target/release directory.
        var html = "<div>Hello, {{name}}!</div>";
        var json = "{\"name\": \"World\"}";
        var result = WebUIRenderer.RenderHtml(html, json);
        Assert.Contains("Hello, World!", result);
    }

    [Fact]
    public void RenderHtml_EmptyState_ReturnsTemplate()
    {
        var html = "<p>Static content</p>";
        var result = WebUIRenderer.RenderHtml(html, "{}");
        Assert.Contains("Static content", result);
    }
}
