// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use clap::{Args, ValueEnum};
use webui_desktop::{CaptionButtonSize, Rgba, TitlebarStyle, WindowEffect, WindowOptions};

#[derive(Args, Default)]
pub(super) struct WindowArgs {
    /// Window title
    #[arg(long)]
    title: Option<String>,
    /// Initial window width
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    width: Option<u32>,
    /// Initial window height
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    height: Option<u32>,
    /// Minimum window width
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    min_width: Option<u32>,
    /// Minimum window height
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    min_height: Option<u32>,
    /// Maximum window width
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    max_width: Option<u32>,
    /// Maximum window height
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    max_height: Option<u32>,
    /// Permit resizing (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    resizable: Option<bool>,
    /// Start maximized (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    maximized: Option<bool>,
    /// Start fullscreen (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    fullscreen: Option<bool>,
    /// Keep the window above ordinary windows (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    always_on_top: Option<bool>,
    /// Center the initial window (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    center: Option<bool>,
    /// Pre-paint window background color (#rrggbb or #rrggbbaa)
    #[arg(long)]
    background: Option<Rgba>,
    /// Native titlebar style
    #[arg(long, value_enum)]
    titlebar_style: Option<TitlebarArg>,
    /// Overlay titlebar height in CSS pixels
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    titlebar_height: Option<u32>,
    /// Windows caption button size
    #[arg(long, value_enum)]
    caption_button_size: Option<CaptionButtonArg>,
    /// Native window visual effect
    #[arg(long, value_enum)]
    effect: Option<EffectArg>,
    /// Persist validated window geometry (true or false)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    remember_state: Option<bool>,
    /// Enable web inspector/devtools for the desktop webview
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    devtools: Option<bool>,
}

#[derive(Clone, Copy, ValueEnum)]
enum TitlebarArg {
    Native,
    HiddenInset,
    Overlay,
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum CaptionButtonArg {
    Standard,
    Tall,
}

#[derive(Clone, Copy, ValueEnum)]
enum EffectArg {
    None,
    Vibrancy,
    Acrylic,
    Mica,
    Tabbed,
}

impl WindowArgs {
    pub(super) fn is_set(&self) -> bool {
        self.title.is_some()
            || self.width.is_some()
            || self.height.is_some()
            || self.min_width.is_some()
            || self.min_height.is_some()
            || self.max_width.is_some()
            || self.max_height.is_some()
            || self.resizable.is_some()
            || self.maximized.is_some()
            || self.fullscreen.is_some()
            || self.always_on_top.is_some()
            || self.center.is_some()
            || self.background.is_some()
            || self.titlebar_style.is_some()
            || self.titlebar_height.is_some()
            || self.caption_button_size.is_some()
            || self.effect.is_some()
            || self.remember_state.is_some()
            || self.devtools.is_some()
    }

    pub(super) fn resolve(&self, mut window: WindowOptions) -> Result<WindowOptions> {
        if let Some(title) = &self.title {
            window.title.clone_from(title);
        }
        for (value, target) in [
            (self.width, &mut window.width),
            (self.height, &mut window.height),
        ] {
            if let Some(value) = value {
                *target = value;
            }
        }
        for (value, target) in [
            (self.min_width, &mut window.min_width),
            (self.min_height, &mut window.min_height),
            (self.max_width, &mut window.max_width),
            (self.max_height, &mut window.max_height),
        ] {
            if let Some(value) = value {
                *target = Some(value);
            }
        }
        for (value, target) in [
            (self.resizable, &mut window.resizable),
            (self.maximized, &mut window.maximized),
            (self.fullscreen, &mut window.fullscreen),
            (self.always_on_top, &mut window.always_on_top),
            (self.center, &mut window.center),
            (self.remember_state, &mut window.remember_state),
            (self.devtools, &mut window.devtools),
        ] {
            if let Some(value) = value {
                *target = value;
            }
        }
        if let Some(background) = self.background {
            window.background = Some(background);
        }
        if let Some(size) = self.caption_button_size {
            window.caption_button_size = match size {
                CaptionButtonArg::Standard => CaptionButtonSize::Standard,
                CaptionButtonArg::Tall => CaptionButtonSize::Tall,
            };
        }
        if let Some(effect) = self.effect {
            window.effect = match effect {
                EffectArg::None => WindowEffect::None,
                EffectArg::Vibrancy => WindowEffect::Vibrancy,
                EffectArg::Acrylic => WindowEffect::Acrylic,
                EffectArg::Mica => WindowEffect::Mica,
                EffectArg::Tabbed => WindowEffect::Tabbed,
            };
        }
        if let Some(style) = self.titlebar_style {
            window.titlebar = match style {
                TitlebarArg::Native => TitlebarStyle::Native,
                TitlebarArg::HiddenInset => TitlebarStyle::HiddenInset,
                TitlebarArg::Overlay => TitlebarStyle::Overlay {
                    height: self.titlebar_height.unwrap_or(48),
                },
                TitlebarArg::None => TitlebarStyle::None,
            };
        }
        if let Some(height) = self.titlebar_height {
            match &mut window.titlebar {
                TitlebarStyle::Overlay { height: current } => *current = height,
                _ => return Err(anyhow::anyhow!(
                    "--titlebar-height requires --titlebar-style overlay or an existing overlay titlebar; help: pass --titlebar-style overlay"
                )),
            }
        }
        Ok(window)
    }
}
