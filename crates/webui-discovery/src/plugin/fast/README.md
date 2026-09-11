# FAST Component Discovery

FAST reads each package's `customElements` manifest as its component inventory
and loads standard `*.template.html` files for those declarations.

## Package assets

A single-component package can export `./template.html` and optionally
`./styles.css`. Paths are relative to the package root, including symlinked
packages. Direct strings and `default`/`import`/`require` conditional exports
are supported. `./template-webui.html` is not selected.

The manifest supplies the inventory and names; these exports are only optional
asset-location hints.

Without a package-level template export, FAST uses CEM module-relative
template/style lookup and virtual-module fallbacks, then checks parent
directories within the package when needed. This supports multi-component
packages with templates beside their JavaScript modules.

An explicit missing or invalid asset is an error, not a reason to select a
different template.

## Default fallback

FAST also includes ordinary `<component-name>.html` files not declared in the
manifest, using default filename, CSS, and script-ownership rules. This fallback
also works when the manifest is absent or contains no component declarations.

Manifest declarations win name conflicts. Generated `.template.html` and
`.template-webui.html` assets are not accidentally registered as default names.
Malformed declared metadata still reports an error.

See the [discovery crate README](../../../README.md) for shared package lookup
and scope behavior.
