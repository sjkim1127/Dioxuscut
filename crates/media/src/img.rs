//! `<Img>` component — image element synchronized with the composition.
//!
//! Equivalent to Remotion's `<Img>`.

use dioxus::prelude::*;

/// How to fit the image within its container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Scale to fill, preserving aspect ratio (may crop). Default.
    #[default]
    Cover,
    /// Scale to fit, preserving aspect ratio (may letterbox).
    Contain,
    /// Stretch to fill exactly.
    Fill,
    /// Natural size.
    None,
    /// Scale down if larger than container.
    ScaleDown,
}

impl ImageFit {
    pub fn as_css(&self) -> &'static str {
        match self {
            ImageFit::Cover => "cover",
            ImageFit::Contain => "contain",
            ImageFit::Fill => "fill",
            ImageFit::None => "none",
            ImageFit::ScaleDown => "scale-down",
        }
    }
}

/// Props for `<Img>`.
#[derive(Props, Clone, PartialEq)]
pub struct ImgProps {
    /// Image source URL or static file path.
    pub src: String,
    /// Alt text for accessibility.
    #[props(default)]
    pub alt: Option<String>,
    /// object-fit CSS value.
    #[props(default)]
    pub fit: ImageFit,
    /// Extra inline styles.
    #[props(default)]
    pub style: Option<String>,
    /// CSS class.
    #[props(default)]
    pub class: Option<String>,
}

/// An image element that participates in the composition.
///
/// Equivalent to Remotion's `<Img>`.
#[component]
pub fn Img(props: ImgProps) -> Element {
    let base_style = format!(
        "object-fit: {}; width: 100%; height: 100%;",
        props.fit.as_css()
    );
    let style = match &props.style {
        Some(extra) => format!("{base_style} {extra}"),
        None => base_style,
    };

    rsx! {
        img {
            src: "{props.src}",
            alt: props.alt.unwrap_or_default(),
            style: "{style}",
            class: props.class.unwrap_or_default(),
        }
    }
}
