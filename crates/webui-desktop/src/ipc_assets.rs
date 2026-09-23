// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::DesktopProtocolResponse;

pub(crate) const NATIVE_BOOTSTRAP_SCRIPT: &str = include_str!("generated/ipc/native-bootstrap.js");
pub(crate) const BROWSER_RUNTIME: &[u8] = include_bytes!("generated/ipc/desktop-runtime.js");

pub(crate) fn response(path: &str) -> Option<DesktopProtocolResponse> {
    let body = match path {
        "/_webui/ipc/bootstrap.js" => NATIVE_BOOTSTRAP_SCRIPT.as_bytes(),
        "/_webui/ipc/runtime.js" => BROWSER_RUNTIME,
        _ => return None,
    };
    Some(DesktopProtocolResponse::new(
        200,
        "text/javascript; charset=utf-8",
        body.to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_assets_use_the_embedded_sdk_bytes() {
        let bootstrap = response("/_webui/ipc/bootstrap.js");
        assert!(bootstrap.is_some_and(|response| {
            response.status == 200 && response.body == NATIVE_BOOTSTRAP_SCRIPT.as_bytes()
        }));
        let runtime = response("/_webui/ipc/runtime.js");
        assert!(runtime.is_some_and(|response| response.body == BROWSER_RUNTIME));
        assert!(response("/application.js").is_none());
    }
}
