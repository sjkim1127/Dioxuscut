//! Tailwind CSS utility parser for native video composition.
//!
//! Maps standard Tailwind utility classes (`flex`, `grid-cols-*`, `gap-*`,
//! `p-*`, `m-*`, `bg-*`, `text-*`, `rounded-*`, etc.) into CSS declarations
//! and native [`ResolvedStyle`] values without requiring Node.js or PostCSS.

use crate::css::ResolvedStyle;

/// Parse a single Tailwind utility class token into CSS declaration pairs `(property, value)`.
/// Returns an empty list if the token is not a recognized Tailwind utility.
pub fn parse_utility_class(token: &str) -> Vec<(String, String)> {
    let mut declarations = Vec::new();

    // 1. Display & Flexbox basics
    match token {
        "flex" => declarations.push(("display".into(), "flex".into())),
        "inline-flex" => declarations.push(("display".into(), "inline-flex".into())),
        "grid" => declarations.push(("display".into(), "grid".into())),
        "block" => declarations.push(("display".into(), "block".into())),
        "inline-block" => declarations.push(("display".into(), "block".into())),
        "hidden" => declarations.push(("display".into(), "none".into())),
        // Flex direction
        "flex-row" => declarations.push(("flex-direction".into(), "row".into())),
        "flex-col" | "flex-column" => declarations.push(("flex-direction".into(), "column".into())),
        "flex-row-reverse" => declarations.push(("flex-direction".into(), "row-reverse".into())),
        "flex-col-reverse" => declarations.push(("flex-direction".into(), "column-reverse".into())),
        // Flex wrap
        "flex-wrap" => declarations.push(("flex-wrap".into(), "wrap".into())),
        "flex-nowrap" => declarations.push(("flex-wrap".into(), "nowrap".into())),
        "flex-wrap-reverse" => declarations.push(("flex-wrap".into(), "wrap-reverse".into())),
        // Flex grow / shrink / basis
        "flex-1" => {
            declarations.push(("flex-grow".into(), "1".into()));
            declarations.push(("flex-shrink".into(), "1".into()));
            declarations.push(("flex-basis".into(), "0%".into()));
        }
        "flex-auto" => {
            declarations.push(("flex-grow".into(), "1".into()));
            declarations.push(("flex-shrink".into(), "1".into()));
            declarations.push(("flex-basis".into(), "auto".into()));
        }
        "flex-initial" => {
            declarations.push(("flex-grow".into(), "0".into()));
            declarations.push(("flex-shrink".into(), "1".into()));
            declarations.push(("flex-basis".into(), "auto".into()));
        }
        "flex-none" => {
            declarations.push(("flex-grow".into(), "0".into()));
            declarations.push(("flex-shrink".into(), "0".into()));
            declarations.push(("flex-basis".into(), "auto".into()));
        }
        "grow" | "flex-grow" => declarations.push(("flex-grow".into(), "1".into())),
        "grow-0" | "flex-grow-0" => declarations.push(("flex-grow".into(), "0".into())),
        "shrink" | "flex-shrink" => declarations.push(("flex-shrink".into(), "1".into())),
        "shrink-0" | "flex-shrink-0" => declarations.push(("flex-shrink".into(), "0".into())),
        // Alignment
        "items-start" => declarations.push(("align-items".into(), "flex-start".into())),
        "items-center" => declarations.push(("align-items".into(), "center".into())),
        "items-end" => declarations.push(("align-items".into(), "flex-end".into())),
        "items-baseline" => declarations.push(("align-items".into(), "baseline".into())),
        "items-stretch" => declarations.push(("align-items".into(), "stretch".into())),
        // Justify
        "justify-start" => declarations.push(("justify-content".into(), "flex-start".into())),
        "justify-center" => declarations.push(("justify-content".into(), "center".into())),
        "justify-end" => declarations.push(("justify-content".into(), "flex-end".into())),
        "justify-between" => declarations.push(("justify-content".into(), "space-between".into())),
        "justify-around" => declarations.push(("justify-content".into(), "space-around".into())),
        "justify-evenly" => declarations.push(("justify-content".into(), "space-evenly".into())),
        // Self
        "self-auto" => declarations.push(("align-self".into(), "auto".into())),
        "self-start" => declarations.push(("align-self".into(), "flex-start".into())),
        "self-center" => declarations.push(("align-self".into(), "center".into())),
        "self-end" => declarations.push(("align-self".into(), "flex-end".into())),
        "self-stretch" => declarations.push(("align-self".into(), "stretch".into())),
        // Position
        "absolute" => declarations.push(("position".into(), "absolute".into())),
        "relative" => declarations.push(("position".into(), "relative".into())),
        "fixed" => declarations.push(("position".into(), "absolute".into())),
        "inset-0" => {
            declarations.push(("top".into(), "0px".into()));
            declarations.push(("right".into(), "0px".into()));
            declarations.push(("bottom".into(), "0px".into()));
            declarations.push(("left".into(), "0px".into()));
        }
        // Overflow
        "overflow-hidden" => declarations.push(("overflow".into(), "hidden".into())),
        "overflow-visible" => declarations.push(("overflow".into(), "visible".into())),
        "overflow-auto" => declarations.push(("overflow".into(), "scroll".into())),
        // Object fit
        "object-cover" => declarations.push(("object-fit".into(), "cover".into())),
        "object-contain" => declarations.push(("object-fit".into(), "contain".into())),
        "object-fill" => declarations.push(("object-fit".into(), "fill".into())),
        _ => {}
    }

    if !declarations.is_empty() {
        return declarations;
    }

    // 2. Grid template columns & rows
    if let Some(rest) = token.strip_prefix("grid-cols-") {
        if let Ok(n) = rest.parse::<usize>() {
            if n > 0 {
                let tracks = vec!["1fr"; n].join(" ");
                declarations.push(("grid-template-columns".into(), tracks));
                return declarations;
            }
        }
    }
    if let Some(rest) = token.strip_prefix("grid-rows-") {
        if let Ok(n) = rest.parse::<usize>() {
            if n > 0 {
                let tracks = vec!["1fr"; n].join(" ");
                declarations.push(("grid-template-rows".into(), tracks));
                return declarations;
            }
        }
    }
    if let Some(rest) = token.strip_prefix("col-span-") {
        if rest == "full" {
            declarations.push(("grid-column".into(), "1 / -1".into()));
            return declarations;
        } else if let Ok(n) = rest.parse::<usize>() {
            declarations.push(("grid-column".into(), format!("span {n}")));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("row-span-") {
        if rest == "full" {
            declarations.push(("grid-row".into(), "1 / -1".into()));
            return declarations;
        } else if let Ok(n) = rest.parse::<usize>() {
            declarations.push(("grid-row".into(), format!("span {n}")));
            return declarations;
        }
    }

    // 3. Gap
    if let Some(rest) = token.strip_prefix("gap-x-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("column-gap".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("gap-y-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("row-gap".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("gap-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("gap".into(), v));
            return declarations;
        }
    }

    // 4. Padding & Margin
    if let Some(rest) = token.strip_prefix("p-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("px-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-left".into(), v.clone()));
            declarations.push(("padding-right".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("py-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-top".into(), v.clone()));
            declarations.push(("padding-bottom".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("pt-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-top".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("pr-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-right".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("pb-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-bottom".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("pl-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("padding-left".into(), v));
            return declarations;
        }
    }

    // Margin
    if token == "m-auto" {
        declarations.push(("margin".into(), "auto".into()));
        return declarations;
    }
    if token == "mx-auto" {
        declarations.push(("margin-left".into(), "auto".into()));
        declarations.push(("margin-right".into(), "auto".into()));
        return declarations;
    }
    if token == "my-auto" {
        declarations.push(("margin-top".into(), "auto".into()));
        declarations.push(("margin-bottom".into(), "auto".into()));
        return declarations;
    }
    if let Some(rest) = token.strip_prefix("m-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("mx-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-left".into(), v.clone()));
            declarations.push(("margin-right".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("my-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-top".into(), v.clone()));
            declarations.push(("margin-bottom".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("mt-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-top".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("mr-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-right".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("mb-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-bottom".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("ml-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("margin-left".into(), v));
            return declarations;
        }
    }

    // 5. Width & Height
    if let Some(rest) = token.strip_prefix("w-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("width".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("h-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("height".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("min-w-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("min-width".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("min-h-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("min-height".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("max-w-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("max-width".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("max-h-") {
        if let Some(v) = parse_size_or_fraction(rest) {
            declarations.push(("max-height".into(), v));
            return declarations;
        }
    }

    // 6. Inset (top, right, bottom, left)
    if let Some(rest) = token.strip_prefix("top-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("top".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("right-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("right".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("bottom-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("bottom".into(), v));
            return declarations;
        }
    }
    if let Some(rest) = token.strip_prefix("left-") {
        if let Some(v) = parse_spacing_or_exact(rest) {
            declarations.push(("left".into(), v));
            return declarations;
        }
    }

    // 7. Rounded (border-radius)
    if token == "rounded" {
        declarations.push(("border-radius".into(), "4px".into()));
        return declarations;
    }
    if let Some(rest) = token.strip_prefix("rounded-") {
        let radius = match rest {
            "none" => "0px",
            "sm" => "2px",
            "md" => "6px",
            "lg" => "8px",
            "xl" => "12px",
            "2xl" => "16px",
            "3xl" => "24px",
            "full" => "9999px",
            bracket if bracket.starts_with('[') && bracket.ends_with(']') => {
                &bracket[1..bracket.len() - 1]
            }
            _ => "",
        };
        if !radius.is_empty() {
            declarations.push(("border-radius".into(), radius.into()));
            return declarations;
        }
    }

    // 8. Border width & Border color
    if token == "border" {
        declarations.push(("border-width".into(), "1px".into()));
        return declarations;
    }
    if let Some(rest) = token.strip_prefix("border-") {
        if let Ok(w) = rest.parse::<usize>() {
            declarations.push(("border-width".into(), format!("{w}px")));
            return declarations;
        }
        if rest.starts_with('[') && rest.ends_with(']') {
            let inner = &rest[1..rest.len() - 1];
            if inner.ends_with("px") {
                declarations.push(("border-width".into(), inner.into()));
            } else {
                declarations.push(("border-color".into(), inner.into()));
            }
            return declarations;
        }
        if let Some(color) = resolve_tailwind_color(rest) {
            declarations.push(("border-color".into(), color));
            return declarations;
        }
    }

    // 9. Background color
    if let Some(rest) = token.strip_prefix("bg-") {
        if rest.starts_with('[') && rest.ends_with(']') {
            let color = &rest[1..rest.len() - 1];
            declarations.push(("background".into(), color.into()));
            return declarations;
        }
        if let Some(color) = resolve_tailwind_color(rest) {
            declarations.push(("background".into(), color));
            return declarations;
        }
    }

    // 10. Text color & Font size & Font weight
    if let Some(rest) = token.strip_prefix("text-") {
        // Bracketed arbitrary values: text-[#...] or text-[20px]
        if rest.starts_with('[') && rest.ends_with(']') {
            let inner = &rest[1..rest.len() - 1];
            if inner.starts_with('#')
                || inner.starts_with("rgb")
                || inner.starts_with("hsl")
                || inner == "transparent"
            {
                declarations.push(("color".into(), inner.into()));
            } else {
                declarations.push(("font-size".into(), inner.into()));
            }
            return declarations;
        }

        // Font sizes: text-xs, text-sm, text-base, text-lg, text-xl..text-9xl
        let size = match rest {
            "xs" => Some("12px"),
            "sm" => Some("14px"),
            "base" => Some("16px"),
            "lg" => Some("18px"),
            "xl" => Some("20px"),
            "2xl" => Some("24px"),
            "3xl" => Some("30px"),
            "4xl" => Some("36px"),
            "5xl" => Some("48px"),
            "6xl" => Some("60px"),
            "7xl" => Some("72px"),
            "8xl" => Some("96px"),
            "9xl" => Some("128px"),
            _ => None,
        };
        if let Some(s) = size {
            declarations.push(("font-size".into(), s.into()));
            return declarations;
        }

        // Color: text-white, text-red-500, etc.
        if let Some(color) = resolve_tailwind_color(rest) {
            declarations.push(("color".into(), color));
            return declarations;
        }
    }

    // 11. Font weight
    if let Some(rest) = token.strip_prefix("font-") {
        let weight = match rest {
            "thin" => Some("100"),
            "extralight" => Some("200"),
            "light" => Some("300"),
            "normal" => Some("400"),
            "medium" => Some("500"),
            "semibold" => Some("600"),
            "bold" => Some("700"),
            "extrabold" => Some("800"),
            "black" => Some("900"),
            _ => None,
        };
        if let Some(w) = weight {
            declarations.push(("font-weight".into(), w.into()));
            return declarations;
        }
    }

    // 12. Line height (leading)
    if let Some(rest) = token.strip_prefix("leading-") {
        let leading = match rest {
            "none" => Some("1.0"),
            "tight" => Some("1.25"),
            "snug" => Some("1.375"),
            "normal" => Some("1.5"),
            "relaxed" => Some("1.625"),
            "loose" => Some("2.0"),
            bracket if bracket.starts_with('[') && bracket.ends_with(']') => {
                Some(&bracket[1..bracket.len() - 1])
            }
            _ => None,
        };
        if let Some(l) = leading {
            declarations.push(("line-height".into(), l.into()));
            return declarations;
        }
    }

    // 13. Opacity
    if let Some(rest) = token.strip_prefix("opacity-") {
        if rest.starts_with('[') && rest.ends_with(']') {
            let val = &rest[1..rest.len() - 1];
            declarations.push(("opacity".into(), val.into()));
            return declarations;
        }
        if let Ok(pct) = rest.parse::<f32>() {
            let clamped = (pct / 100.0).clamp(0.0, 1.0);
            declarations.push(("opacity".into(), clamped.to_string()));
            return declarations;
        }
    }

    declarations
}

/// Applies a single Tailwind utility class directly to a [`ResolvedStyle`].
/// Returns `true` if the class was recognized and applied.
pub fn apply_utility_class(style: &mut ResolvedStyle, token: &str) -> bool {
    let decls = parse_utility_class(token);
    if decls.is_empty() {
        return false;
    }
    for (name, value) in &decls {
        crate::css::apply_declarations(style, &[(name.clone(), value.clone())]);
    }
    true
}

/// Helper to parse spacing numbers or arbitrary bracket values.
fn parse_spacing_or_exact(s: &str) -> Option<String> {
    if s.starts_with('[') && s.ends_with(']') {
        return Some(s[1..s.len() - 1].to_string());
    }
    if let Ok(val) = s.parse::<f32>() {
        // Tailwind 1 unit = 4px (0.25rem)
        let px = val * 4.0;
        return Some(format!("{px}px"));
    }
    None
}

/// Helper to parse size utilities: fractions ("1/2", "full", "screen", etc.) or spacing scale.
fn parse_size_or_fraction(s: &str) -> Option<String> {
    match s {
        "full" => return Some("100%".into()),
        "screen" => return Some("100vw".into()),
        "auto" => return Some("auto".into()),
        "fit" | "max" => return Some("auto".into()),
        "1/2" => return Some("50%".into()),
        "1/3" => return Some("33.333333%".into()),
        "2/3" => return Some("66.666667%".into()),
        "1/4" => return Some("25%".into()),
        "2/4" => return Some("50%".into()),
        "3/4" => return Some("75%".into()),
        "1/5" => return Some("20%".into()),
        "2/5" => return Some("40%".into()),
        "3/5" => return Some("60%".into()),
        "4/5" => return Some("80%".into()),
        "1/6" => return Some("16.666667%".into()),
        "5/6" => return Some("83.333333%".into()),
        "1/12" => return Some("8.333333%".into()),
        _ => {}
    }
    parse_spacing_or_exact(s)
}

/// Resolves a Tailwind color name (e.g. `white`, `black`, `slate-900`, `blue-500`, `red-600`) to a hex string.
pub fn resolve_tailwind_color(color_name: &str) -> Option<String> {
    match color_name {
        "white" => Some("#ffffff".into()),
        "black" => Some("#000000".into()),
        "transparent" => Some("transparent".into()),
        // Slate
        "slate-50" => Some("#f8fafc".into()),
        "slate-100" => Some("#f1f5f9".into()),
        "slate-200" => Some("#e2e8f0".into()),
        "slate-300" => Some("#cbd5e1".into()),
        "slate-400" => Some("#94a3b8".into()),
        "slate-500" => Some("#64748b".into()),
        "slate-600" => Some("#475569".into()),
        "slate-700" => Some("#334155".into()),
        "slate-800" => Some("#1e293b".into()),
        "slate-900" => Some("#0f172a".into()),
        "slate-950" => Some("#020617".into()),
        // Gray
        "gray-50" => Some("#f9fafb".into()),
        "gray-100" => Some("#f3f4f6".into()),
        "gray-200" => Some("#e5e7eb".into()),
        "gray-300" => Some("#d1d5db".into()),
        "gray-400" => Some("#9ca3af".into()),
        "gray-500" => Some("#6b7280".into()),
        "gray-600" => Some("#4b5563".into()),
        "gray-700" => Some("#374151".into()),
        "gray-800" => Some("#1f2937".into()),
        "gray-900" => Some("#111827".into()),
        "gray-950" => Some("#030712".into()),
        // Zinc
        "zinc-50" => Some("#fafafa".into()),
        "zinc-100" => Some("#f4f4f5".into()),
        "zinc-200" => Some("#e4e4e7".into()),
        "zinc-300" => Some("#d4d4d8".into()),
        "zinc-400" => Some("#a1a1aa".into()),
        "zinc-500" => Some("#71717a".into()),
        "zinc-600" => Some("#52525b".into()),
        "zinc-700" => Some("#3f3f46".into()),
        "zinc-800" => Some("#27272a".into()),
        "zinc-900" => Some("#18181b".into()),
        "zinc-950" => Some("#09090b".into()),
        // Red
        "red-50" => Some("#fef2f2".into()),
        "red-100" => Some("#fee2e2".into()),
        "red-200" => Some("#fecaca".into()),
        "red-300" => Some("#fca5a5".into()),
        "red-400" => Some("#f87171".into()),
        "red-500" => Some("#ef4444".into()),
        "red-600" => Some("#dc2626".into()),
        "red-700" => Some("#b91c1c".into()),
        "red-800" => Some("#991b1b".into()),
        "red-900" => Some("#7f1d1d".into()),
        "red-950" => Some("#450a0a".into()),
        // Orange
        "orange-500" => Some("#f97316".into()),
        "orange-600" => Some("#ea580c".into()),
        // Amber
        "amber-400" => Some("#fbbf24".into()),
        "amber-500" => Some("#f59e0b".into()),
        // Yellow
        "yellow-300" => Some("#fde047".into()),
        "yellow-400" => Some("#facc15".into()),
        "yellow-500" => Some("#eab308".into()),
        // Green
        "green-400" => Some("#4ade80".into()),
        "green-500" => Some("#22c55e".into()),
        "green-600" => Some("#16a34a".into()),
        // Emerald
        "emerald-400" => Some("#34d399".into()),
        "emerald-500" => Some("#10b981".into()),
        "emerald-600" => Some("#059669".into()),
        // Cyan
        "cyan-400" => Some("#22d3ee".into()),
        "cyan-500" => Some("#06b6d4".into()),
        // Sky
        "sky-400" => Some("#38bdf8".into()),
        "sky-500" => Some("#0ea5e9".into()),
        // Blue
        "blue-50" => Some("#eff6ff".into()),
        "blue-100" => Some("#dbeafe".into()),
        "blue-200" => Some("#bfdbfe".into()),
        "blue-300" => Some("#93c5fd".into()),
        "blue-400" => Some("#60a5fa".into()),
        "blue-500" => Some("#3b82f6".into()),
        "blue-600" => Some("#2563eb".into()),
        "blue-700" => Some("#1d4ed8".into()),
        "blue-800" => Some("#1e40af".into()),
        "blue-900" => Some("#1e3a8a".into()),
        "blue-950" => Some("#172554".into()),
        // Indigo
        "indigo-400" => Some("#818cf8".into()),
        "indigo-500" => Some("#6366f1".into()),
        "indigo-600" => Some("#4f46e5".into()),
        // Purple
        "purple-500" => Some("#a855f7".into()),
        "purple-600" => Some("#9333ea".into()),
        // Fuchsia
        "fuchsia-500" => Some("#d946ef".into()),
        // Pink
        "pink-500" => Some("#ec4899".into()),
        "pink-600" => Some("#db2777".into()),
        // Rose
        "rose-500" => Some("#f43f5e".into()),
        "rose-600" => Some("#e11d48".into()),
        _ => None,
    }
}
