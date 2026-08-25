// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST parser plugins and shared implementation.

mod convert;
pub(crate) mod diagnostic;
mod shared;
pub mod v2;
pub mod v3;

#[deprecated(
    since = "0.0.11",
    note = "use FastV2ParserPlugin from plugin::fast_v2 or FastV3ParserPlugin from plugin::fast_v3"
)]
pub type FastParserPlugin = v2::FastV2ParserPlugin;

#[deprecated(
    since = "0.0.11",
    note = "use generate_f_template from plugin::fast_v2 or plugin::fast_v3"
)]
pub use v2::generate_f_template;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::ParserPlugin;

    #[test]
    #[allow(deprecated)]
    fn fast_parser_alias_uses_fast_v2_parser() {
        let mut plugin = FastParserPlugin::new();
        assert_eq!(
            plugin.process_attribute(crate::plugin::AttributeContext { name: "@click" }),
            crate::plugin::AttributeAction::SkipAndCountBinding
        );
    }
}
