use crate::dom::NativeElement;
use dioxuscut_rasterizer::{Color, ImageFit};
use std::collections::HashMap;
use taffy::geometry::{Line, Point, Rect, Size};
use taffy::prelude::{
    auto, fr, length, line, percent, span, AlignContent, AlignItems, Dimension, Display,
    FlexDirection, FlexWrap, GridAutoFlow, GridPlacement, GridTemplateComponent, LengthPercentage,
    LengthPercentageAuto, Position, Style,
};
use taffy::style::Overflow;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CssError {
    #[error("invalid CSS rule: {0}")]
    InvalidRule(String),
    #[error("unsupported CSS selector: {0}")]
    UnsupportedSelector(String),
}

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    rules: Vec<CssRule>,
}

#[derive(Debug, Clone)]
struct CssRule {
    selector: SimpleSelector,
    declarations: Vec<(String, String)>,
    order: usize,
}

#[derive(Debug, Clone, Default)]
struct SimpleSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    universal: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedStyle {
    pub layout: Style,
    pub color: Color,
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub border_width: f32,
    pub border_radius: f32,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_sources: Vec<String>,
    pub line_height: f32,
    pub opacity: f32,
    pub overflow_hidden: bool,
    pub object_fit: ImageFit,
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        let layout = Style {
            display: Display::Block,
            ..Default::default()
        };
        Self {
            layout,
            color: Color::BLACK,
            background: None,
            border_color: None,
            border_width: 0.0,
            border_radius: 0.0,
            font_size: 16.0,
            font_weight: 400,
            font_sources: Vec::new(),
            line_height: 1.2,
            opacity: 1.0,
            overflow_hidden: false,
            object_fit: ImageFit::Cover,
        }
    }
}

impl ResolvedStyle {
    pub fn inherited(&self) -> Self {
        Self {
            color: self.color,
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_sources: self.font_sources.clone(),
            line_height: self.line_height,
            ..Self::default()
        }
    }
}

impl Stylesheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse rules using simple tag, `.class`, `#id`, and compound selectors.
    pub fn parse(css: &str) -> Result<Self, CssError> {
        let css = strip_comments(css);
        let mut rules = Vec::new();
        let mut remainder = css.as_str();
        let mut order = 0;
        while let Some(open) = remainder.find('{') {
            let selector_text = remainder[..open].trim();
            let after_open = &remainder[open + 1..];
            let close = after_open.find('}').ok_or_else(|| {
                CssError::InvalidRule(format!("missing closing brace after {selector_text}"))
            })?;
            let declarations = parse_declarations(&after_open[..close]);
            for selector in selector_text.split(',').map(str::trim) {
                if selector.is_empty() {
                    continue;
                }
                rules.push(CssRule {
                    selector: SimpleSelector::parse(selector)?,
                    declarations: declarations.clone(),
                    order,
                });
                order += 1;
            }
            remainder = &after_open[close + 1..];
        }
        if !remainder.trim().is_empty() {
            return Err(CssError::InvalidRule(remainder.trim().to_string()));
        }
        Ok(Self { rules })
    }

    pub(crate) fn resolve(
        &self,
        element: &NativeElement,
        parent: Option<&ResolvedStyle>,
    ) -> ResolvedStyle {
        let mut resolved = parent.map_or_else(ResolvedStyle::default, ResolvedStyle::inherited);
        apply_tag_defaults(&mut resolved, &element.tag);
        apply_html_attributes(&mut resolved, element);

        let mut winners: HashMap<String, (u32, usize, String)> = HashMap::new();
        for rule in &self.rules {
            if !rule.selector.matches(element) {
                continue;
            }
            let specificity = rule.selector.specificity();
            for (name, value) in &rule.declarations {
                let replace = winners.get(name).is_none_or(|(score, order, _)| {
                    specificity > *score || (specificity == *score && rule.order >= *order)
                });
                if replace {
                    winners.insert(name.clone(), (specificity, rule.order, value.clone()));
                }
            }
        }
        let inline_order = self.rules.len() + 1;
        if let Some(style) = element.attributes.get("style") {
            for (name, value) in parse_declarations(style) {
                winners.insert(name, (1_000, inline_order, value));
            }
        }
        for (name, value) in &element.styles {
            winners.insert(
                normalize_property(name),
                (1_000, inline_order, value.clone()),
            );
        }
        if let Some(font) = element.attributes.get("data-font-src") {
            winners.insert(
                "--dioxuscut-font-source".into(),
                (1_000, inline_order, font.clone()),
            );
        }

        let mut declarations = winners
            .into_iter()
            .map(|(name, (_, _, value))| (name, value))
            .collect::<Vec<_>>();
        declarations.sort_by(|left, right| left.0.cmp(&right.0));
        apply_declarations(&mut resolved, &declarations);
        resolved
    }
}

impl SimpleSelector {
    fn parse(selector: &str) -> Result<Self, CssError> {
        if selector == "*" {
            return Ok(Self {
                universal: true,
                ..Default::default()
            });
        }
        if selector.contains(char::is_whitespace)
            || selector.contains('>')
            || selector.contains('+')
            || selector.contains('~')
            || selector.contains('[')
            || selector.contains(':')
        {
            return Err(CssError::UnsupportedSelector(selector.into()));
        }

        let mut output = Self::default();
        let bytes = selector.as_bytes();
        let mut start = 0;
        let mut mode = 't';
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'.' || *byte == b'#' {
                if index > start {
                    output.push_part(mode, &selector[start..index])?;
                }
                mode = if *byte == b'.' { 'c' } else { 'i' };
                start = index + 1;
            }
        }
        if start < selector.len() {
            output.push_part(mode, &selector[start..])?;
        }
        if output.tag.is_none() && output.id.is_none() && output.classes.is_empty() {
            return Err(CssError::UnsupportedSelector(selector.into()));
        }
        Ok(output)
    }

    fn push_part(&mut self, mode: char, value: &str) -> Result<(), CssError> {
        if value.is_empty() {
            return Err(CssError::UnsupportedSelector(
                "empty selector component".into(),
            ));
        }
        match mode {
            't' => self.tag = Some(value.to_ascii_lowercase()),
            'c' => self.classes.push(value.to_string()),
            'i' => self.id = Some(value.to_string()),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn matches(&self, element: &NativeElement) -> bool {
        if self.universal {
            return true;
        }
        if self
            .tag
            .as_ref()
            .is_some_and(|tag| !element.tag.eq_ignore_ascii_case(tag))
        {
            return false;
        }
        if self.id.as_ref().is_some_and(|id| {
            element
                .attributes
                .get("id")
                .is_none_or(|actual| actual != id)
        }) {
            return false;
        }
        let classes = element
            .attributes
            .get("class")
            .map(|classes| classes.split_whitespace().collect::<Vec<_>>())
            .unwrap_or_default();
        self.classes
            .iter()
            .all(|required| classes.iter().any(|actual| *actual == required))
    }

    fn specificity(&self) -> u32 {
        u32::from(self.id.is_some()) * 100
            + self.classes.len() as u32 * 10
            + u32::from(self.tag.is_some())
    }
}

fn strip_comments(css: &str) -> String {
    let mut output = String::with_capacity(css.len());
    let mut remainder = css;
    while let Some(start) = remainder.find("/*") {
        output.push_str(&remainder[..start]);
        let after = &remainder[start + 2..];
        let Some(end) = after.find("*/") else {
            return output;
        };
        remainder = &after[end + 2..];
    }
    output.push_str(remainder);
    output
}

fn normalize_property(name: &str) -> String {
    name.trim().replace('_', "-").to_ascii_lowercase()
}

fn parse_declarations(input: &str) -> Vec<(String, String)> {
    input
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let name = normalize_property(name);
            let value = value.trim();
            (!name.is_empty() && !value.is_empty()).then(|| (name, value.to_string()))
        })
        .collect()
}

fn apply_tag_defaults(style: &mut ResolvedStyle, tag: &str) {
    style.layout.display = Display::Block;
    match tag.to_ascii_lowercase().as_str() {
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = 700;
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = 700;
        }
        "h3" => {
            style.font_size = 20.0;
            style.font_weight = 700;
        }
        "strong" | "b" => style.font_weight = 700,
        "img" | "video" => style.layout.item_is_replaced = true,
        _ => {}
    }
}

fn apply_html_attributes(style: &mut ResolvedStyle, element: &NativeElement) {
    if let Some(width) = element.attributes.get("width") {
        style.layout.size.width = parse_dimension(width);
    }
    if let Some(height) = element.attributes.get("height") {
        style.layout.size.height = parse_dimension(height);
    }
}

fn apply_declarations(style: &mut ResolvedStyle, declarations: &[(String, String)]) {
    for (name, value) in declarations {
        match name.as_str() {
            "display" => {
                style.layout.display = match value.as_str() {
                    "flex" | "inline-flex" => Display::Flex,
                    "grid" | "inline-grid" => Display::Grid,
                    "none" => Display::None,
                    _ => Display::Block,
                }
            }
            "position" => {
                style.layout.position = if value == "absolute" {
                    Position::Absolute
                } else {
                    Position::Relative
                }
            }
            "width" => style.layout.size.width = parse_dimension(value),
            "height" => style.layout.size.height = parse_dimension(value),
            "min-width" => style.layout.min_size.width = parse_dimension(value),
            "min-height" => style.layout.min_size.height = parse_dimension(value),
            "max-width" => style.layout.max_size.width = parse_dimension(value),
            "max-height" => style.layout.max_size.height = parse_dimension(value),
            "top" => style.layout.inset.top = parse_length_auto(value),
            "right" => style.layout.inset.right = parse_length_auto(value),
            "bottom" => style.layout.inset.bottom = parse_length_auto(value),
            "left" => style.layout.inset.left = parse_length_auto(value),
            "margin" => style.layout.margin = parse_rect_auto(value),
            "margin-top" => style.layout.margin.top = parse_length_auto(value),
            "margin-right" => style.layout.margin.right = parse_length_auto(value),
            "margin-bottom" => style.layout.margin.bottom = parse_length_auto(value),
            "margin-left" => style.layout.margin.left = parse_length_auto(value),
            "padding" => style.layout.padding = parse_rect(value),
            "padding-top" => style.layout.padding.top = parse_length(value),
            "padding-right" => style.layout.padding.right = parse_length(value),
            "padding-bottom" => style.layout.padding.bottom = parse_length(value),
            "padding-left" => style.layout.padding.left = parse_length(value),
            "gap" => {
                let values = css_values(value);
                let row = parse_length(values.first().copied().unwrap_or("0"));
                let column = parse_length(
                    values
                        .get(1)
                        .copied()
                        .unwrap_or_else(|| values.first().copied().unwrap_or("0")),
                );
                style.layout.gap = Size {
                    width: column,
                    height: row,
                };
            }
            "row-gap" => style.layout.gap.height = parse_length(value),
            "column-gap" => style.layout.gap.width = parse_length(value),
            "flex-direction" => {
                style.layout.flex_direction = match value.as_str() {
                    "column" => FlexDirection::Column,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => FlexDirection::Row,
                }
            }
            "flex-wrap" => {
                style.layout.flex_wrap = match value.as_str() {
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => FlexWrap::NoWrap,
                }
            }
            "flex-grow" => style.layout.flex_grow = parse_number(value, 0.0).max(0.0),
            "flex-shrink" => style.layout.flex_shrink = parse_number(value, 1.0).max(0.0),
            "flex-basis" => style.layout.flex_basis = parse_dimension(value),
            "align-items" => style.layout.align_items = parse_align_items(value),
            "align-self" => style.layout.align_self = parse_align_items(value),
            "align-content" => style.layout.align_content = parse_align_content(value),
            "justify-content" => style.layout.justify_content = parse_align_content(value),
            "grid-template-columns" => {
                style.layout.grid_template_columns = parse_grid_tracks(value)
            }
            "grid-template-rows" => style.layout.grid_template_rows = parse_grid_tracks(value),
            "grid-auto-flow" => {
                style.layout.grid_auto_flow = match value.as_str() {
                    "column" => GridAutoFlow::Column,
                    "row dense" => GridAutoFlow::RowDense,
                    "column dense" => GridAutoFlow::ColumnDense,
                    _ => GridAutoFlow::Row,
                }
            }
            "grid-row" => style.layout.grid_row = parse_grid_line(value),
            "grid-column" => style.layout.grid_column = parse_grid_line(value),
            "grid-row-start" => style.layout.grid_row.start = parse_grid_placement(value),
            "grid-row-end" => style.layout.grid_row.end = parse_grid_placement(value),
            "grid-column-start" => style.layout.grid_column.start = parse_grid_placement(value),
            "grid-column-end" => style.layout.grid_column.end = parse_grid_placement(value),
            "background" | "background-color" => style.background = Color::from_css(value),
            "color" => {
                if let Some(color) = Color::from_css(value) {
                    style.color = color;
                }
            }
            "border-color" => style.border_color = Color::from_css(value),
            "border" => {
                for part in css_values(value) {
                    if let Some(width) = parse_px(part) {
                        style.border_width = width.max(0.0);
                    } else if let Some(color) = Color::from_css(part) {
                        style.border_color = Some(color);
                    }
                }
                style.layout.border = Rect {
                    left: LengthPercentage::length(style.border_width),
                    right: LengthPercentage::length(style.border_width),
                    top: LengthPercentage::length(style.border_width),
                    bottom: LengthPercentage::length(style.border_width),
                };
            }
            "border-width" => {
                style.border_width = parse_px(value).unwrap_or(0.0).max(0.0);
                style.layout.border = Rect {
                    left: LengthPercentage::length(style.border_width),
                    right: LengthPercentage::length(style.border_width),
                    top: LengthPercentage::length(style.border_width),
                    bottom: LengthPercentage::length(style.border_width),
                };
            }
            "border-radius" => style.border_radius = parse_px(value).unwrap_or(0.0).max(0.0),
            "font-size" => style.font_size = parse_px(value).unwrap_or(style.font_size).max(1.0),
            "font-weight" => {
                style.font_weight = match value.as_str() {
                    "bold" => 700,
                    "normal" => 400,
                    _ => value.parse().unwrap_or(style.font_weight),
                }
            }
            "line-height" => {
                let parsed = value
                    .trim_end_matches('%')
                    .trim_end_matches("px")
                    .parse::<f32>()
                    .ok();
                if let Some(parsed) = parsed {
                    style.line_height = if value.ends_with('%') {
                        parsed / 100.0
                    } else if value.ends_with("px") {
                        parsed / style.font_size
                    } else {
                        parsed
                    }
                    .max(0.1);
                }
            }
            "opacity" => style.opacity = parse_number(value, 1.0).clamp(0.0, 1.0),
            "overflow" => {
                let overflow = match value.as_str() {
                    "hidden" => Overflow::Hidden,
                    "clip" => Overflow::Clip,
                    "scroll" | "auto" => Overflow::Scroll,
                    _ => Overflow::Visible,
                };
                style.layout.overflow = Point {
                    x: overflow,
                    y: overflow,
                };
                style.overflow_hidden = overflow != Overflow::Visible;
            }
            "object-fit" => {
                style.object_fit = match value.as_str() {
                    "contain" => ImageFit::Contain,
                    "fill" => ImageFit::Fill,
                    "none" => ImageFit::None,
                    "scale-down" => ImageFit::ScaleDown,
                    _ => ImageFit::Cover,
                }
            }
            "aspect-ratio" => {
                style.layout.aspect_ratio = value.split_once('/').and_then(|(width, height)| {
                    let width = width.trim().parse::<f32>().ok()?;
                    let height = height.trim().parse::<f32>().ok()?;
                    (height > 0.0).then_some(width / height)
                });
            }
            "--dioxuscut-font-source" => {
                style.font_sources = value
                    .split(',')
                    .map(|source| source.trim().trim_matches(['\'', '"']).to_string())
                    .filter(|source| !source.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
}

fn css_values(value: &str) -> Vec<&str> {
    value.split_whitespace().collect()
}

fn parse_number(value: &str, default: f32) -> f32 {
    value.parse().unwrap_or(default)
}

fn parse_px(value: &str) -> Option<f32> {
    value.trim().trim_end_matches("px").parse::<f32>().ok()
}

fn parse_dimension(value: &str) -> Dimension {
    let value = value.trim();
    if value == "auto" || value.is_empty() {
        Dimension::auto()
    } else if let Some(percent) = value.strip_suffix('%') {
        Dimension::percent(percent.parse::<f32>().unwrap_or(0.0) / 100.0)
    } else {
        Dimension::length(parse_px(value).unwrap_or(0.0))
    }
}

fn parse_length(value: &str) -> LengthPercentage {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        LengthPercentage::percent(percent.parse::<f32>().unwrap_or(0.0) / 100.0)
    } else {
        LengthPercentage::length(parse_px(value).unwrap_or(0.0))
    }
}

fn parse_length_auto(value: &str) -> LengthPercentageAuto {
    if value.trim() == "auto" {
        LengthPercentageAuto::auto()
    } else if let Some(percent) = value.trim().strip_suffix('%') {
        LengthPercentageAuto::percent(percent.parse::<f32>().unwrap_or(0.0) / 100.0)
    } else {
        LengthPercentageAuto::length(parse_px(value).unwrap_or(0.0))
    }
}

fn expand_four<'a>(values: &'a [&'a str]) -> [&'a str; 4] {
    match values {
        [all] => [all, all, all, all],
        [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
        [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
        [top, right, bottom, left, ..] => [top, right, bottom, left],
        [] => ["0", "0", "0", "0"],
    }
}

fn parse_rect(value: &str) -> Rect<LengthPercentage> {
    let values = css_values(value);
    let [top, right, bottom, left] = expand_four(&values);
    Rect {
        left: parse_length(left),
        right: parse_length(right),
        top: parse_length(top),
        bottom: parse_length(bottom),
    }
}

fn parse_rect_auto(value: &str) -> Rect<LengthPercentageAuto> {
    let values = css_values(value);
    let [top, right, bottom, left] = expand_four(&values);
    Rect {
        left: parse_length_auto(left),
        right: parse_length_auto(right),
        top: parse_length_auto(top),
        bottom: parse_length_auto(bottom),
    }
}

fn parse_align_items(value: &str) -> Option<AlignItems> {
    Some(match value {
        "start" => AlignItems::START,
        "end" => AlignItems::END,
        "flex-start" => AlignItems::FLEX_START,
        "flex-end" => AlignItems::FLEX_END,
        "center" => AlignItems::CENTER,
        "baseline" => AlignItems::BASELINE,
        "stretch" => AlignItems::STRETCH,
        _ => return None,
    })
}

fn parse_align_content(value: &str) -> Option<AlignContent> {
    Some(match value {
        "start" => AlignContent::START,
        "end" => AlignContent::END,
        "flex-start" => AlignContent::FLEX_START,
        "flex-end" => AlignContent::FLEX_END,
        "center" => AlignContent::CENTER,
        "stretch" => AlignContent::STRETCH,
        "space-between" => AlignContent::SPACE_BETWEEN,
        "space-around" => AlignContent::SPACE_AROUND,
        "space-evenly" => AlignContent::SPACE_EVENLY,
        _ => return None,
    })
}

fn parse_grid_tracks(value: &str) -> Vec<GridTemplateComponent<String>> {
    css_values(value)
        .into_iter()
        .filter_map(|value| {
            if value == "auto" {
                Some(auto())
            } else if let Some(fraction) = value.strip_suffix("fr") {
                Some(fr(fraction.parse::<f32>().unwrap_or(1.0).max(0.0)))
            } else if let Some(value) = value.strip_suffix('%') {
                Some(percent(value.parse::<f32>().ok()? / 100.0))
            } else {
                Some(length(parse_px(value)?))
            }
        })
        .collect()
}

fn parse_grid_line(value: &str) -> Line<GridPlacement<String>> {
    let mut parts = value.split('/').map(str::trim);
    Line {
        start: parts.next().map(parse_grid_placement).unwrap_or_default(),
        end: parts.next().map(parse_grid_placement).unwrap_or_default(),
    }
}

fn parse_grid_placement(value: &str) -> GridPlacement<String> {
    let value = value.trim();
    if value == "auto" || value.is_empty() {
        auto()
    } else if let Some(span_count) = value.strip_prefix("span ") {
        span(span_count.parse::<u16>().unwrap_or(1).max(1))
    } else {
        line(value.parse::<i16>().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade_uses_specificity_and_inline_styles() {
        let stylesheet = Stylesheet::parse(
            "div { color: #111111; } .card { color: #222222; padding: 8px; } #hero { color: #333333; }",
        )
        .unwrap();
        let mut element = NativeElement {
            tag: "div".into(),
            ..Default::default()
        };
        element.attributes.insert("class".into(), "card".into());
        element.attributes.insert("id".into(), "hero".into());
        element
            .attributes
            .insert("style".into(), "color: #444444; width: 50%".into());

        let style = stylesheet.resolve(&element, None);
        assert_eq!(style.color, Color::rgb(0x44, 0x44, 0x44));
        assert_eq!(style.layout.size.width, Dimension::percent(0.5));
        assert_eq!(style.layout.padding.left, LengthPercentage::length(8.0));
    }

    #[test]
    fn unsupported_complex_selector_is_reported() {
        assert!(matches!(
            Stylesheet::parse(".card > span { color: red; }"),
            Err(CssError::UnsupportedSelector(_))
        ));
    }

    #[test]
    fn parses_grid_tracks_and_placements() {
        let stylesheet = Stylesheet::parse(
            ".grid { display: grid; grid-template-columns: 100px 1fr 25%; } .item { grid-column: 2 / span 2; }",
        )
        .unwrap();
        let mut grid = NativeElement {
            tag: "div".into(),
            ..Default::default()
        };
        grid.attributes.insert("class".into(), "grid".into());
        let grid_style = stylesheet.resolve(&grid, None);
        assert_eq!(grid_style.layout.display, Display::Grid);
        assert_eq!(grid_style.layout.grid_template_columns.len(), 3);

        let mut item = NativeElement {
            tag: "div".into(),
            ..Default::default()
        };
        item.attributes.insert("class".into(), "item".into());
        let item_style = stylesheet.resolve(&item, None);
        assert!(matches!(
            item_style.layout.grid_column.start,
            GridPlacement::Line(_)
        ));
        assert_eq!(item_style.layout.grid_column.end, GridPlacement::Span(2));
    }
}
