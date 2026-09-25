// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt::Write;

use anyhow::Result;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use webview2_com::{CoTaskMemPWSTR, ExecuteScriptCompletedHandler};

/// Physical measurements of native controls, translated at the CSS boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Metrics {
    pub(super) left: i32,
    pub(super) right: i32,
    pub(super) height: i32,
    pub(super) dpi: u32,
    pub(super) minimum_height: u32,
}

impl Metrics {
    pub(super) fn script(self) -> String {
        let mut script = String::with_capacity(1100);
        let _ = write!(
            script,
            "(()=>{{const left={},right={},height={};",
            logical(self.left, self.dpi),
            logical(self.right, self.dpi),
            logical(self.height, self.dpi).max(self.minimum_height),
        );
        script.push_str(
            "const apply=()=>{const root=document.documentElement;if(!root)return false;\
             const rtl=getComputedStyle(root).direction==='rtl';\
             root.style.setProperty('--webui-titlebar-inset-start',(rtl?right:left)+'px');\
             root.style.setProperty('--webui-titlebar-inset-end',(rtl?left:right)+'px');\
             root.style.setProperty('--webui-titlebar-height',height+'px');return true;};\
             if(!apply()){const observer=new MutationObserver(()=>{if(apply())observer.disconnect();});\
             observer.observe(document,{childList:true,subtree:true});}\
             if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',apply,{once:true});\
             })();",
        );
        script
    }

    pub(super) fn publish(self, webview: &ICoreWebView2) -> Result<()> {
        let script = CoTaskMemPWSTR::from(self.script().as_str());
        let completion = ExecuteScriptCompletedHandler::create(Box::new(|result, _| {
            if let Err(error) = result {
                eprintln!("WebUI: failed to apply native titlebar safe areas: {error}");
            }
            Ok(())
        }));
        // SAFETY: The live WebView2 copies script text and retains its handler.
        unsafe { webview.ExecuteScript(*script.as_ref().as_pcwstr(), &completion)? };
        Ok(())
    }
}

fn logical(value: i32, dpi: u32) -> u32 {
    let pixels = u32::try_from(value).unwrap_or(0);
    let logical = (u64::from(pixels) * 96).div_ceil(u64::from(dpi.max(1)));
    u32::try_from(logical).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_safe_areas_round_outward_at_every_scale() {
        assert_eq!(logical(138, 96), 138);
        assert_eq!(logical(207, 144), 138);
        assert_eq!(logical(173, 120), 139);
        assert_eq!(logical(72, 144), 48);
        assert_eq!(logical(-1, 96), 0);
    }

    #[test]
    fn script_maps_physical_sides_to_document_direction_before_paint() {
        let script = Metrics {
            left: 0,
            right: 207,
            height: 72,
            dpi: 144,
            minimum_height: 0,
        }
        .script();
        assert!(script.contains("left=0,right=138,height=48"));
        assert!(script.contains("(rtl?right:left)"));
        assert!(script.contains("(rtl?left:right)"));
        assert!(script.contains("observer.disconnect()"));
        assert!(script.contains("'DOMContentLoaded',apply,{once:true}"));
    }

    #[test]
    fn taller_application_bands_do_not_shrink_to_the_native_caption_height() {
        let script = Metrics {
            height: 48,
            dpi: 96,
            minimum_height: 64,
            ..Default::default()
        }
        .script();
        assert!(script.contains("height=64"));
    }
}
