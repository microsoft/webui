// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt::{Display, Formatter, Write};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Cross-platform desktop window configuration stored in a bundle manifest.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowOptions {
    /// Window title.
    #[serde(default = "default_title")]
    pub title: String,
    /// Initial window width in physical-independent pixels.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Initial window height in physical-independent pixels.
    #[serde(default = "default_height")]
    pub height: u32,
    /// Minimum width, when supported by the native backend.
    #[serde(default)]
    pub min_width: Option<u32>,
    /// Minimum height, when supported by the native backend.
    #[serde(default)]
    pub min_height: Option<u32>,
    /// Maximum width, when supported by the native backend.
    #[serde(default)]
    pub max_width: Option<u32>,
    /// Maximum height, when supported by the native backend.
    #[serde(default)]
    pub max_height: Option<u32>,
    /// Whether the user can resize the window.
    #[serde(default = "default_resizable")]
    pub resizable: bool,
    /// Whether to start maximized.
    #[serde(default)]
    pub maximized: bool,
    /// Whether to start fullscreen.
    #[serde(default)]
    pub fullscreen: bool,
    /// Whether to keep the window above ordinary windows.
    #[serde(default)]
    pub always_on_top: bool,
    /// Whether the native backend should center the initial window.
    #[serde(default = "default_center")]
    pub center: bool,
    /// Solid background painted before web content makes its first paint.
    #[serde(default)]
    pub background: Option<Rgba>,
    /// Native titlebar presentation.
    #[serde(default)]
    pub titlebar: TitlebarStyle,
    /// Requested platform visual effect.
    #[serde(default)]
    pub effect: WindowEffect,
    /// Whether to persist validated window geometry between launches.
    #[serde(default)]
    pub remember_state: bool,
    /// Whether to enable web inspector/devtools for development builds.
    #[serde(default)]
    pub devtools: bool,
}

const fn default_width() -> u32 {
    1200
}
const fn default_height() -> u32 {
    800
}
const fn default_resizable() -> bool {
    true
}
const fn default_center() -> bool {
    true
}
fn default_title() -> String {
    "WebUI".to_string()
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: default_title(),
            width: default_width(),
            height: default_height(),
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            resizable: true,
            maximized: false,
            fullscreen: false,
            always_on_top: false,
            center: true,
            background: None,
            titlebar: TitlebarStyle::default(),
            effect: WindowEffect::default(),
            remember_state: false,
            devtools: false,
        }
    }
}

/// An RGBA color used for the native window's pre-paint background.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgba {
    /// Red component.
    pub r: u8,
    /// Green component.
    pub g: u8,
    /// Blue component.
    pub b: u8,
    /// Alpha component.
    pub a: u8,
}

impl Display for Rgba {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        if self.a == u8::MAX {
            write!(formatter, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(
                formatter,
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

/// Error returned when a manifest color is malformed.
#[derive(Debug, Error)]
pub enum RgbaParseError {
    /// The color did not have a supported number of hex digits.
    #[error("invalid RGBA color '{value}': expected #rrggbb or #rrggbbaa; help: use lowercase or uppercase hexadecimal digits")]
    InvalidLength {
        /// Rejected color text.
        value: String,
    },
    /// The color contains a non-hexadecimal character.
    #[error("invalid RGBA color '{value}': component '{component}' is not hexadecimal; help: use digits 0-9 and letters a-f")]
    InvalidHex {
        /// Rejected color text.
        value: String,
        /// Rejected component.
        component: char,
    },
}

impl FromStr for Rgba {
    type Err = RgbaParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let bytes = value.as_bytes();
        if !(bytes.len() == 7 || bytes.len() == 9) || bytes.first() != Some(&b'#') {
            return Err(RgbaParseError::InvalidLength {
                value: value.to_string(),
            });
        }
        let byte = |offset: usize| -> Result<u8, RgbaParseError> {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                b'A'..=b'F' => Ok(byte - b'A' + 10),
                other => Err(RgbaParseError::InvalidHex {
                    value: value.to_string(),
                    component: char::from(other),
                }),
            };
            Ok((nibble(bytes[offset])? << 4) | nibble(bytes[offset + 1])?)
        };
        Ok(Self {
            r: byte(1)?,
            g: byte(3)?,
            b: byte(5)?,
            a: if bytes.len() == 9 { byte(7)? } else { u8::MAX },
        })
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Native titlebar presentation requested by a window.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "style")]
pub enum TitlebarStyle {
    /// Platform-native titlebar, matching prior WebUI behavior.
    #[default]
    Native,
    /// Native controls float over full-bleed application content.
    HiddenInset,
    /// System caption buttons with an application-drawn bar.
    Overlay {
        /// Height of the overlay titlebar in CSS pixels.
        height: u32,
    },
    /// Fully frameless window. The application draws all chrome.
    None,
}

/// Platform visual effect requested by a window.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WindowEffect {
    /// No platform effect.
    #[default]
    None,
    /// macOS vibrancy or the closest native equivalent.
    Vibrancy,
    /// Windows acrylic or closest equivalent.
    Acrylic,
    /// Windows mica or closest equivalent.
    Mica,
    /// Tabbed titlebar treatment where supported.
    Tabbed,
}

/// Native platform used to calculate titlebar safe areas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopPlatform {
    /// macOS.
    Macos,
    /// Microsoft Windows.
    Windows,
    /// Linux desktop environments.
    Linux,
}
impl DesktopPlatform {
    /// Return the compilation target platform.
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Linux
        }
    }
}

/// Safe-area insets reserved for native window controls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowInsets {
    /// Leading safe inset in CSS pixels.
    pub start: u32,
    /// Trailing safe inset in CSS pixels.
    pub end: u32,
    /// Top safe inset in CSS pixels.
    pub top: u32,
}
impl WindowInsets {
    /// Calculate native control insets for a style and platform.
    #[must_use]
    pub const fn for_style(style: &TitlebarStyle, platform: DesktopPlatform) -> Self {
        let height = match style {
            TitlebarStyle::Overlay { height } => *height,
            TitlebarStyle::HiddenInset => 28,
            TitlebarStyle::Native | TitlebarStyle::None => 0,
        };
        match style {
            TitlebarStyle::HiddenInset | TitlebarStyle::Overlay { .. } => match platform {
                DesktopPlatform::Macos => Self {
                    start: 78,
                    end: 0,
                    top: height,
                },
                DesktopPlatform::Windows => Self {
                    start: 0,
                    end: 138,
                    top: height,
                },
                DesktopPlatform::Linux => Self {
                    start: 0,
                    end: 0,
                    top: height,
                },
            },
            TitlebarStyle::Native | TitlebarStyle::None => Self {
                start: 0,
                end: 0,
                top: 0,
            },
        }
    }
}

/// Build the constant `<style>` block carrying window CSS custom properties.
///
/// The block depends only on window configuration and the target platform, so
/// hosts compute it once at startup and reuse it for every rendered document.
/// Returns an empty string when the window needs no injected CSS.
#[must_use]
pub fn window_css_block(window: &WindowOptions, platform: DesktopPlatform) -> String {
    if matches!(window.titlebar, TitlebarStyle::Native) && window.background.is_none() {
        return String::new();
    }
    let insets = WindowInsets::for_style(&window.titlebar, platform);
    let mut style = String::with_capacity(196);
    style.push_str("<style>:root{--webui-titlebar-inset-start:");
    let _ = write!(
        style,
        "{}px;--webui-titlebar-inset-end:{}px;--webui-titlebar-height:{}px",
        insets.start, insets.end, insets.top
    );
    if let Some(background) = window.background {
        style.push_str(";--webui-window-background:");
        let _ = write!(style, "{background}");
    }
    style.push('}');
    if window.background.is_some() {
        style.push_str("html{background:var(--webui-window-background)}");
    }
    style.push_str("</style>");
    style
}

/// Splice a precomputed window CSS block into rendered HTML in one linear scan.
///
/// The block is placed inside `<head>` when present. Documents without a head
/// fall back to the start of `<body>`, and finally to the end of the document,
/// because injecting ahead of the doctype would force quirks mode.
#[must_use]
pub fn apply_window_css(html: String, block: &str) -> String {
    if block.is_empty() {
        return html;
    }
    let position = html
        .find("</head>")
        .or_else(|| html.find("<body"))
        .unwrap_or(html.len());
    let mut output = String::with_capacity(html.len() + block.len());
    output.push_str(&html[..position]);
    output.push_str(block);
    output.push_str(&html[position..]);
    output
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    #[test]
    fn parses_rgba() {
        assert_eq!(
            "#1234ab80".parse::<Rgba>().unwrap(),
            Rgba {
                r: 0x12,
                g: 0x34,
                b: 0xab,
                a: 0x80
            }
        );
        assert_eq!("#1234ab".parse::<Rgba>().unwrap().a, 255);
    }
    #[test]
    fn rejects_bad_rgba_actionably() {
        assert!("#12"
            .parse::<Rgba>()
            .unwrap_err()
            .to_string()
            .contains("#rrggbb"));
        assert!("#zzzzzz"
            .parse::<Rgba>()
            .unwrap_err()
            .to_string()
            .contains("not hexadecimal"));
    }
    #[test]
    fn calculates_platform_insets() {
        let style = TitlebarStyle::HiddenInset;
        assert_eq!(
            WindowInsets::for_style(&style, DesktopPlatform::Macos).start,
            78
        );
        assert_eq!(
            WindowInsets::for_style(&style, DesktopPlatform::Windows).end,
            138
        );
        assert_eq!(
            WindowInsets::for_style(&style, DesktopPlatform::Linux).top,
            28
        );
    }
    #[test]
    fn injects_window_css() {
        let options = WindowOptions {
            titlebar: TitlebarStyle::HiddenInset,
            background: Some("#112233".parse().unwrap()),
            ..WindowOptions::default()
        };
        let block = window_css_block(&options, DesktopPlatform::Macos);
        let html = apply_window_css(
            "<html><head></head><body></body></html>".to_string(),
            &block,
        );
        assert!(html.contains("--webui-titlebar-inset-start:78px"));
        assert!(html.contains("html{background"));
        assert!(html.find("<style>") < html.find("</head>"));
    }

    #[test]
    fn native_titlebar_without_background_injects_nothing() {
        let block = window_css_block(&WindowOptions::default(), DesktopPlatform::Macos);
        assert!(block.is_empty());
        let html = apply_window_css("<html></html>".to_string(), &block);
        assert_eq!(html, "<html></html>");
    }

    #[test]
    fn headless_documents_never_inject_ahead_of_the_doctype() {
        let options = WindowOptions {
            titlebar: TitlebarStyle::HiddenInset,
            ..WindowOptions::default()
        };
        let block = window_css_block(&options, DesktopPlatform::Macos);

        let with_body =
            apply_window_css("<!doctype html><body><p>hi</p></body>".to_string(), &block);
        assert!(with_body.starts_with("<!doctype html>"));
        assert!(with_body.find("<style>") < with_body.find("<body"));

        let fragment = apply_window_css("<!doctype html><p>hi</p>".to_string(), &block);
        assert!(fragment.starts_with("<!doctype html>"));
        assert!(fragment.ends_with("</style>"));
    }
    #[test]
    fn legacy_window_manifest_defaults_new_fields() {
        let options: WindowOptions = serde_json::from_str(
            r#"{"title":"Legacy","width":640,"height":480,"maximized":false,"devtools":true}"#,
        )
        .unwrap();
        assert!(options.resizable);
        assert!(options.center);
        assert_eq!(options.titlebar, TitlebarStyle::Native);
        assert_eq!(options.effect, WindowEffect::None);
    }
}
