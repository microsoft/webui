// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Deprecated FAST hydration plugin compatibility shim.
//!
//! FAST hydration implementations live in `fast_v2` and `fast_v3`. This module
//! keeps the original `plugin::fast` public path available without containing
//! hydration implementation logic.

pub use super::fast_v2::FastV2HydrationPlugin;

/// Deprecated compatibility alias for the legacy `fast` handler plugin.
///
/// Use [`FastV2HydrationPlugin`] for explicit FAST 2 compatibility. FAST 3
/// marker output lives in [`super::fast_v3::FastV3HydrationPlugin`].
#[deprecated(
    since = "0.0.11",
    note = "use plugin::fast_v2::FastV2HydrationPlugin for FAST 2 compatibility or plugin::fast_v3::FastV3HydrationPlugin for FAST 3"
)]
pub type FastHydrationPlugin = super::fast_v2::FastV2HydrationPlugin;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::HandlerPlugin;
    use crate::{ResponseWriter, Result};

    struct TestWriter {
        output: String,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                output: String::new(),
            }
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    #[allow(deprecated)]
    fn test_fast_alias_uses_v2_markers() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_component_scope();
        let mut writer = TestWriter::new();
        assert!(plugin.on_binding_start("userName", &mut writer).is_ok());
        assert_eq!(writer.output, "<!--fe-b$$start$$0$$userName$$fe-b-->");
    }
}
