# microsoft-webui-projection

Native build-time state projection analysis for [WebUI](https://github.com/microsoft/webui).

This crate analyzes original JavaScript and TypeScript to determine the
component properties required for hydration and client navigation.

- `analyze_sources` analyzes compiler-selected scripts and their supported
  relative imports, returning a proven state surface or a diagnostic per tag.
- `analyze` proves component state surfaces from a completed bundle graph.
- `analyze_with_report` also reports which source modules were parsed.
- Unsupported or ambiguous declarations produce structured diagnostics rather
  than incomplete property lists or a full-state fallback.

For managed bundles, the caller supplies source text, exact resolved imports,
and package ownership through `ModuleGraph`. This crate does not bundle browser
assets, construct projection manifests, or render requests.

For application authoring and supported build workflows, see the
[WebUI documentation](https://microsoft.github.io/webui).

Licensed under the MIT license.
