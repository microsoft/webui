# __APP_NAME__

A two-page WebUI app using the **__SERVER__** server.

## Run

Install Node.js 22 or newer and your selected server's toolchain, then run
these commands from this directory:

```sh
__PACKAGE_MANAGER__ install
__PACKAGE_MANAGER__ run check
__PACKAGE_MANAGER__ run dev
```

Open <http://127.0.0.1:3000>. Development watches your source files and runs the
selected server. Changes rebuild the matching browser assets and protocol
before restarting it. A failed rebuild stops the server until you fix the
source. Stop with Ctrl+C.

| Server | Additional prerequisites |
| --- | --- |
| Rust | Rust/Cargo __RUST_VERSION__ or newer |
| Node | No additional toolchain |
| Bun | Bun with Node-API support |

Keep optional npm platform dependencies enabled. If your package manager blocks
esbuild's install script, review and approve that package specifically; do not
globally enable dependency scripts.
Use your selected package manager's `run` commands. Yarn starters include a
project-local `.yarnrc.yml` selecting the `node-modules` linker for the native
TypeScript compiler; no global Yarn settings are changed.

## Edit

- `src/home-page/home-page.html` - Home content.
- `src/about-page/about-page.html` - a page with no executable JavaScript.
- `src/app-shell/` - shared layout and styles.
- `src/app-counter/` - the interactive island.
- `server/` - request state and native rendering.

Navigation uses ordinary document links. Without JavaScript, both pages work
and the counter stays disabled. On Home, hydration enables its native button.

## Build and start

```sh
__PACKAGE_MANAGER__ run check
__PACKAGE_MANAGER__ run build
__PACKAGE_MANAGER__ run start
```

`build` compiles the browser entry, templates, CSS, and selected host.
`start` uses those artifacts without compiling templates. Keep `dist/protocol.bin`
private; only `dist/public/` is served at `/assets/`. Run from the app root.
Rust needs its built server binary. Node/Bun need the installed WebUI native addon.
Rust builds explicitly use the app-owned `server/target/` directory, even when
`CARGO_TARGET_DIR` is set, so build, dev, and start always share the same binary.

The default listener is loopback on port 3000. Set `PORT` and `HOST` using your
deployment environment to change it. Binding a public interface is an explicit
deployment choice; terminate TLS at a trusted ingress.

## AI reference

Optionally install the existing project-local authoring reference:

```sh
npx skills add microsoft/webui --skill webui-reference
```

Its version-matched content becomes available after dependencies are installed.
See the [WebUI documentation](https://microsoft.github.io/webui/) for more.
