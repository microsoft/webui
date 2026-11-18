# microsoft-webui-projection

Native build-time state projection analysis for [WebUI](https://github.com/microsoft/webui).

This crate analyzes original JavaScript and TypeScript sources together with the
resolved module graph from a completed bundler invocation. It determines the
component properties required for hydration and client navigation.

- `analyze` returns the proven component state surfaces.
- `analyze_with_report` also reports which source modules were parsed.
- Unsupported or ambiguous declarations produce structured diagnostics rather
  than incomplete property lists or a full-state fallback.

The caller supplies source text, exact resolved imports, and package ownership
through `ModuleGraph`. This crate does not resolve files or packages, bundle
browser assets, construct projection manifests, or render requests. It belongs
in build tooling, not a production rendering dependency graph.

For application authoring and supported build workflows, see the
[WebUI documentation](https://microsoft.github.io/webui).

Licensed under the MIT license.
