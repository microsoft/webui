// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Deprecated FAST parser plugin compatibility shim.
//!
//! FAST parser implementations live in `fast_v2` and `fast_v3`. This module
//! keeps the original `plugin::fast` public path available without containing
//! parser implementation logic.

#[deprecated(
    since = "0.0.11",
    note = "use FastV2ParserPlugin from plugin::fast_v2 or FastV3ParserPlugin from plugin::fast_v3"
)]
pub type FastParserPlugin = super::fast_v2::FastV2ParserPlugin;

#[deprecated(
    since = "0.0.11",
    note = "use generate_f_template from plugin::fast_v2 or plugin::fast_v3"
)]
pub use super::fast_v2::generate_f_template;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::ParserPlugin;

    #[test]
    #[allow(deprecated)]
    fn fast_parser_alias_uses_fast_v2_parser() {
        let mut plugin = FastParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("@click"),
            crate::plugin::AttributeAction::SkipAndCountBinding
        );
    }
}
