// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Separate Node.js addons compiled from one crate with mutually exclusive modes.
//!
//! The default `handler` mode renders precompiled protocols. The `build` mode
//! compiles templates and bundles browser assets. Build each into its own target
//! directory with `--no-default-features --features handler` or
//! `--no-default-features --features build`, respectively. Only owned protocol
//! buffers cross between the addons; native handles remain local to each addon.

#[cfg(all(feature = "handler", feature = "build"))]
compile_error!(
    "webui-node modes are mutually exclusive: enable exactly one of `handler` or `build`; \
     use --no-default-features --features handler or --no-default-features --features build"
);

#[cfg(not(any(feature = "handler", feature = "build")))]
compile_error!(
    "webui-node requires exactly one mode: enable `handler` or `build` with \
     --no-default-features --features handler or --no-default-features --features build"
);

#[cfg(all(feature = "handler", not(feature = "build")))]
mod handler;
#[cfg(all(feature = "handler", not(feature = "build")))]
pub use handler::*;

#[cfg(all(feature = "build", not(feature = "handler")))]
mod build;
#[cfg(all(feature = "build", not(feature = "handler")))]
pub use build::*;
