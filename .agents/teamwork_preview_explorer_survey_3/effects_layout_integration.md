# Dioxuscut Effects, Layout & Cross-Crate Integration Architecture Survey

**Author**: Explorer 3 (Effects, Layout & Integration Specialist)  
**Date**: 2026-08-21  
**Project**: Dioxuscut Remotion Porting (Remotion v4.0.495 Parity)  
**Integrity Mode**: Development (100% Native Pure-Rust, Zero Runtime Vendor Dependencies)  

---

## 1. Executive Summary

This survey provides the complete architectural design, mathematical formulations, crate interaction contracts, and verification strategy for porting key Remotion packages to 100% native pure-Rust implementations in Dioxuscut.

Key architectural deliverables covered in this report:
1. **Cross-Crate Interaction & Pipeline Flow**: Clean layered separation from computational kernels (`crates/noise`, `crates/paths`, `crates/shapes`, `crates/animation`) through scene graph emitters (`crates/composition`, `crates/transitions`, `crates/captions`, `crates/media`) to rasterization engines (`crates/rasterizer`, `crates/renderer`) and reactive UI layers (`crates/core`, `crates/vdom`, `crates/player`, `apps/studio`).
2. **Visual Post-Processing Filter Pipeline**: Integration of `ChromaticAberration`, `Vignette`, `ColorGrading`, `Contrast`, `Saturation`, `Tint`, `Duotone`, and `ColorMatrix` into `SceneNode::Layer` with high-performance CPU pixel manipulation (`tiny-skia`) and GPU shader (`wgpu` / WGSL) pipelines.
3. **Advanced Typography & Layout Fitting**: Pure-Rust implementation of Remotion's layout utilities (`fit_text_on_n_lines`, `fill_text_box`, `fit_text`, `measure_text_width`) coupled with multi-line stepped parametric rounded text boxes (`create_rounded_text_box`) generating exact SVG path curves with variable corner arcs.
4. **Standalone Procedural Noise & Shader Deformation**: True Simplex 2D/3D/4D noise generators with deterministic seeding (`mulberry32` PRNG parity with Remotion `random()`), Fractional Brownian Motion (fBm), and turbulent vector deformation.
5. **Ergonomic Public API & Trait Hierarchy**: Standardized trait contracts (`SceneEmitter`, `TransitionPresentation`, `RasterizerBackend`, `NoiseGenerator`), unified configuration types, and seamless re-exports across `dioxuscut-*` crates.
6. **Multi-Tier Test Architecture**: 4-tier verification suite covering mathematical parity against Remotion reference test vectors, adversarial boundary unit testing, cross-crate integration pipelines, and visual pixel assertions.

---

## 2. Cross-Crate Interaction Architecture

### 2.1 Layered Dependency Hierarchy

To ensure high performance, zero circular dependencies, and complete decoupling from external JS/Node runtimes, Dioxuscut follows a strict 5-layer architecture:

```
+----------------------------------------------------------------------------------------------------+
|                                    Layer 4: Application & UI                                       |
|  - apps/studio (Desktop / Web Studio)      - apps/example (Demo Compositions)                      |
|  - crates/player (Interactive Player UI)   - crates/cli (CLI Engine & Daemon)                      |
+----------------------------------------------------------------------------------------------------+
                                               |
                                               v
+----------------------------------------------------------------------------------------------------+
|                                  Layer 3: Composition & VDOM                                       |
|  - crates/core (<Composition>, <Sequence>, <AbsoluteFill>, <Freeze>, hooks)                       |
|  - crates/vdom (VirtualDom mutation listener, CSS stylesheet parser, Taffy flexbox/grid layout)   |
|  - crates/composition (SceneEmitter, SceneStack, SceneLoop, SceneTransitionSeries, Registry)       |
+----------------------------------------------------------------------------------------------------+
                                               |
                                               v
+----------------------------------------------------------------------------------------------------+
|                              Layer 2: Domain Emitters & Transitions                                |
|  - crates/transitions (SceneFade, SceneSlide, SceneWipe, SceneClockWipe, SceneFlip, SceneZoom)    |
|  - crates/captions (SRT parser, TikTok kinetic word timing, SceneCaptions)                         |
|  - crates/media (Audio, Image, Video frame emitters)                                               |
|  - crates/shapes (Procedural SVG shape generators: Pie, Rect, Circle, Star, Arrow, Heart, etc.)   |
+----------------------------------------------------------------------------------------------------+
                                               |
                                               v
+----------------------------------------------------------------------------------------------------+
|                               Layer 1: Computational Primitives                                    |
|  - crates/noise (Simplex 2D/3D/4D, Mulberry32 PRNG, fBm, turbulent flow deformation)              |
|  - crates/paths (SVG path parser, length calculation, arc length subdivision, interpolate_path)   |
|  - crates/animation (interpolate, spring physics, bezier easing curves, interpolate_colors)        |
+----------------------------------------------------------------------------------------------------+
                                               |
                                               v
+----------------------------------------------------------------------------------------------------+
|                            Layer 0: Core Data Models & Rasterization                               |
|  - crates/rasterizer (SceneNode, SceneFilter, TinySkiaBackend, WgpuBackend, FontCache, FrameCache) |
|  - crates/renderer (FFmpeg pipe encoding, MP4/WebM/GIF packaging, CompositorDaemon IPC)           |
+----------------------------------------------------------------------------------------------------+
```

### 2.2 Dataflow: From Declarative Components to Pixels

1. **Composition Declaration**:
   - The user defines video structure either natively using the `SceneEmitter` builder API or declaratively with Dioxus RSX (`<Composition>`, `<TransitionSeries>`, `<NoiseBackground>`).
2. **Timeline Frame Evaluation**:
   - For a given timeline frame index $F$, `SceneFrameContext` is constructed containing local frame $f$, global frame $F$, and `NativeCompositionContext` (resolution, FPS, duration).
3. **Scene Graph Emission**:
   - Each emitter in the tree recursively evaluates its state and emits concrete `SceneNode` variants into a shared `Scene` container.
   - For example, `<TransitionSeries>` calculates overlapping time windows, applies easing curves via `dioxuscut-animation`, computes clip paths via `dioxuscut-shapes` / `dioxuscut-paths`, and encapsulates clips inside `SceneNode::Layer` or `SceneNode::Group`.
4. **Offscreen Layer Filtering & Masking**:
   - `SceneNode::Layer` boundaries isolate rendering into separate offscreen pixmaps.
   - The rasterizer applies active filters (`SceneFilter::ChromaticAberration`, `SceneFilter::Vignette`, `SceneFilter::ColorGrading`, `SceneFilter::Blur`) sequentially to the offscreen buffer.
   - Geometric clipping (`ClipRegion::Path` or `ClipRegion::Rect`) and alpha/luminance masks (`MaskMode`) are applied.
   - Drop shadows (`SceneShadow`) are generated and composited.
5. **Backend Rasterization**:
   - `TinySkiaBackend` executes CPU rasterization using SIMD-accelerated blending.
   - `FrameCacheManager` retains rendered frames in an LRU memory pool to eliminate redundant rasterization during timeline scrubbing.
6. **Streaming & IPC**:
   - Rendered `RgbaImage` buffers are streamed to FFmpeg via `PipeConfig` or transmitted over zero-copy IPC `BinaryPacket` framing to the Studio UI.

---

## 3. Visual Post-Processing & Compositing Filters Pipeline

### 3.1 `SceneFilter` Enum & Scene Graph Extensions

In `crates/rasterizer/src/scene.rs`, the `SceneFilter` enum is expanded from basic blur/brightness to a comprehensive suite of cinematic visual effects:

```rust
/// Blend mode for vignette edge compositing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VignetteMode {
    #[default]
    Color,
    Alpha,
}

/// Color grading adjustments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorGradingFilter {
    /// Contrast multiplier (1.0 = normal, 0.0 = flat gray, >1.0 = high contrast).
    pub contrast: f32,
    /// Saturation multiplier (1.0 = normal, 0.0 = grayscale, >1.0 = oversaturated).
    pub saturation: f32,
    /// Exposure / brightness multiplier (1.0 = normal, 0.0 = black, >1.0 = bright).
    pub exposure: f32,
    /// Temperature tint shift in Kelvin or normalized (-1.0 to 1.0, blue to orange).
    pub temperature: f32,
    /// Tint shift (-1.0 to 1.0, green to magenta).
    pub tint: f32,
    /// Gamma exponent adjustment (default 1.0).
    pub gamma: f32,
}

impl Default for ColorGradingFilter {
    fn default() -> Self {
        Self {
            contrast: 1.0,
            saturation: 1.0,
            exposure: 1.0,
            temperature: 0.0,
            tint: 0.0,
            gamma: 1.0,
        }
    }
}

/// Comprehensive pixel filters applied to an offscreen composited Layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneFilter {
    /// Gaussian / Box blur with specified standard deviation in pixels.
    Blur { sigma: f32 },
    /// Direct brightness multiplier (0.0 = black, 1.0 = normal, >1.0 = bright).
    Brightness { amount: f32 },
    /// Grayscale conversion (0.0 = full color, 1.0 = monochrome).
    Grayscale { amount: f32 },
    /// Layer-wide alpha multiplier (0.0 = invisible, 1.0 = fully opaque).
    Opacity { amount: f32 },
    /// RGB channel split simulating chromatic dispersion in optical lenses.
    ChromaticAberration {
        /// Channel separation distance in pixels.
        amount: f32,
        /// Dispersion angle in degrees (0 = horizontal, 90 = vertical).
        angle_deg: f32,
    },
    /// Vignette darkening or alpha fade towards layer edges.
    Vignette {
        /// Intensity of the vignette in [0.0, 1.0].
        amount: f32,
        /// Inner radius of the unaffected center region in [0.0, 1.0].
        radius: f32,
        /// Feather softness transition width in [0.0, 1.0].
        feather: f32,
        /// Geometric shape: 0.0 = rectangular box, 1.0 = radial ellipse.
        roundness: f32,
        /// Color overlaid on the outer edges (default black).
        color: Color,
        /// Whether to overlay color or fade layer alpha.
        mode: VignetteMode,
        /// Center of the vignette in normalized UV coordinates (default [0.5, 0.5]).
        center: (f32, f32),
    },
    /// Contrast adjustment filter.
    Contrast { amount: f32 },
    /// Saturation adjustment filter.
    Saturation { amount: f32 },
    /// Color tinting overlay.
    Tint { color: Color, amount: f32 },
    /// Duotone color mapping based on luminance.
    Duotone { shadow: Color, highlight: Color },
    /// Comprehensive multi-parameter color grading.
    ColorGrading(ColorGradingFilter),
    /// Arbitrary 4x5 affine color transform matrix.
    ColorMatrix { matrix: [f32; 20] },
}
```

### 3.2 Detailed Mathematical Formulations

#### 1. Chromatic Aberration

Chromatic aberration simulates lens dispersion by shifting the Red and Blue channels in opposite directions along a vector $\vec{v}$, while keeping the Green channel unshifted:

$$\theta = \text{angle\_deg} \cdot \frac{\pi}{180}$$

$$\Delta x = \text{amount} \cdot \cos(\theta), \quad \Delta y = \text{amount} \cdot \sin(\theta)$$

For each output pixel at integer coordinate $(x, y)$:
- **Red channel**: Sampled from $(x - \Delta x, y - \Delta y)$ using bilinear interpolation:
  $$R_{\text{out}}(x, y) = R_{\text{src}}(x - \Delta x, y - \Delta y)$$
- **Green channel**: Sampled directly at current coordinate:
  $$G_{\text{out}}(x, y) = G_{\text{src}}(x, y)$$
- **Blue channel**: Sampled from $(x + \Delta x, y + \Delta y)$ using bilinear interpolation:
  $$B_{\text{out}}(x, y) = B_{\text{src}}(x + \Delta x, y + \Delta y)$$
- **Alpha channel**: Preserved from the center sample $A_{\text{src}}(x, y)$ or composite alpha.

#### 2. Parametric Vignette

For a pixel at normalized UV coordinate $(u, v) \in [0, 1]^2$ with center $(c_u, c_v)$:

1. **Normalized Centered Vector**:
   $$\vec{p} = 2 \cdot (|u - c_u|, |v - c_v|)$$

2. **Parametric Distance Metric**:
   $$d_{\text{rect}} = \max(p_x, p_y)$$
   $$d_{\text{ellipse}} = \sqrt{p_x^2 + p_y^2}$$
   $$d = (1 - \text{roundness}) \cdot d_{\text{rect}} + \text{roundness} \cdot d_{\text{ellipse}}$$

3. **Mask Coverage Calculation**:
   $$\text{smoothstep}(e_0, e_1, x) = \begin{cases} 
   0, & x \le e_0 \\ 
   3t^2 - 2t^3 \text{ where } t = \frac{x - e_0}{e_1 - e_0}, & e_0 < x < e_1 \\ 
   1, & x \ge e_1 
   \end{cases}$$

   $$\text{mask} = \begin{cases} 
   \text{if } d \ge \text{radius } \text{then } \text{amount} \text{ else } 0, & \text{feather} \le 10^{-4} \\ 
   \text{smoothstep}(\text{radius}, \text{radius} + \text{feather}, d) \cdot \text{amount}, & \text{feather} > 10^{-4} 
   \end{cases}$$

4. **Compositing**:
   - **`VignetteMode::Alpha`**:
     $$A_{\text{out}} = A_{\text{src}} \cdot (1 - \text{mask})$$
     $$\text{RGB}_{\text{out}} = \text{RGB}_{\text{src}} \cdot (1 - \text{mask})$$
   - **`VignetteMode::Color`**:
     $$A_{\text{overlay}} = \text{mask} \cdot A_{\text{color}}$$
     $$\text{RGB}_{\text{out}} = \text{RGB}_{\text{color}} \cdot A_{\text{overlay}} + \text{RGB}_{\text{src}} \cdot (1 - A_{\text{overlay}})$$
     $$A_{\text{out}} = A_{\text{overlay}} + A_{\text{src}} \cdot (1 - A_{\text{overlay}})$$

#### 3. Color Grading, Contrast & Saturation

- **Contrast**:
  $$C' = \text{clamp}\left((C - 128) \cdot \text{contrast} + 128, 0, 255\right)$$
- **Saturation (Rec. 709 Luminance)**:
  $$Y = 0.2126 \cdot R + 0.7152 \cdot G + 0.0722 \cdot B$$
  $$C' = \text{clamp}\left(Y + (C - Y) \cdot \text{saturation}, 0, 255\right)$$
- **Duotone Mapping**:
  $$\text{Luma} = \frac{0.2126 \cdot R + 0.7152 \cdot G + 0.0722 \cdot B}{255}$$
  $$\text{RGB}_{\text{out}} = (1 - \text{Luma}) \cdot \text{RGB}_{\text{shadow}} + \text{Luma} \cdot \text{RGB}_{\text{highlight}}$$

### 3.3 Tiny-Skia CPU & Wgpu GPU Shader Implementation

In `crates/rasterizer/src/tiny_skia_backend.rs`, pixel filters operate directly on premultiplied RGBA buffers in `Pixmap`:

```rust
pub(crate) fn apply_chromatic_aberration(
    pixmap: &mut Pixmap,
    amount: f32,
    angle_deg: f32,
) -> Result<(), RasterError> {
    if !amount.is_finite() || !angle_deg.is_finite() || amount <= 0.0 {
        return Ok(());
    }

    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let radians = angle_deg.to_radians();
    let dx = amount * radians.cos();
    let dy = amount * radians.sin();

    let src = pixmap.clone();
    let src_data = src.data();
    let dst_data = pixmap.data_mut();

    let sample_channel = |x: f32, y: f32, channel: usize| -> u8 {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = x0 + 1;
        let y1 = y0 + 1;

        let fx = x - x.floor();
        let fy = y - y.floor();

        let get_px = |px: i32, py: i32| -> f32 {
            let cx = px.clamp(0, width - 1) as usize;
            let cy = py.clamp(0, height - 1) as usize;
            let idx = (cy * width as usize + cx) * 4;
            src_data[idx + channel] as f32
        };

        let p00 = get_px(x0, y0);
        let p10 = get_px(x1, y0);
        let p01 = get_px(x0, y1);
        let p11 = get_px(x1, y1);

        let top = p00 * (1.0 - fx) + p10 * fx;
        let bot = p01 * (1.0 - fx) + p11 * fx;
        (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
    };

    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize * width as usize + x as usize) * 4;
            let fx = x as f32;
            let fy = y as f32;

            let r = sample_channel(fx - dx, fy - dy, 0);
            let g = src_data[idx + 1];
            let b = sample_channel(fx + dx, fy + dy, 2);
            let a = src_data[idx + 3];

            dst_data[idx] = r.min(a);
            dst_data[idx + 1] = g;
            dst_data[idx + 2] = b.min(a);
            dst_data[idx + 3] = a;
        }
    }
    Ok(())
}
```

---

## 4. Advanced Typography & Text Fitting Engine

### 4.1 Architecture Overview

Text rendering and layout fitting in Remotion relies on two core packages:
1. `@remotion/layout-utils`: Multi-line text auto-scaling (`fitTextOnNLines`), greedy text-box packing (`fillTextBox`), and text measurement (`measureText`).
2. `@remotion/rounded-text-box`: Multi-line stepped parametric rounded text boxes (`createRoundedTextBox`) that construct exact curved SVG boundary paths surrounding multi-line wrapped text blocks.

In Dioxuscut, these utilities are implemented natively in `crates/rasterizer` and `crates/shapes` using `ab_glyph`, `rustybuzz`, and `unicode-segmentation`.

```
+-----------------------------------------------------------------------------------------------+
|                                     Typography Pipeline                                       |
|                                                                                               |
|  [Input Text String] ---> [Unicode UAX#14 Word & Grapheme Tokenization]                       |
|                                       |                                                       |
|                                       v                                                       |
|               [Font Fallback Chain: Local TTF -> System -> Bundled NotoSans]                  |
|                                       |                                                       |
|                                       v                                                       |
|                     [Text Width & Height Measurement (measure_text)]                          |
|                                       |                                                       |
|              +------------------------+-------------------------+                             |
|              |                                                  |                             |
|              v                                                  v                             |
|   [1D fit_text Binary Search]                       [fill_text_box Line Packing]              |
|   (Single line auto-fit)                                        |                             |
|                                                                 v                             |
|                                                  [fit_text_on_n_lines Binary Search]          |
|                                                  (Multi-line optimal font size)               |
|                                                                 |                             |
|                                                                 v                             |
|                                            [create_rounded_text_box SVG Generator]            |
|                                            (Stepped multi-corner rounded boundary path)       |
+-----------------------------------------------------------------------------------------------+
```

### 4.2 Multi-Line Text Fitting (`fit_text_on_n_lines` & `fill_text_box`)

#### 1. `fill_text_box` Specification

`fill_text_box` provides a stateful greedy line accumulator that fits words sequentially:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct TextWord {
    pub text: String,
    pub font_size: f32,
    pub font_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillTextBoxResult {
    pub exceeds_box: bool,
    pub new_line: bool,
}

pub struct FillTextBox {
    pub max_box_width: f32,
    pub max_lines: usize,
    lines: Vec<Vec<TextWord>>,
}

impl FillTextBox {
    pub fn new(max_box_width: f32, max_lines: usize) -> Self {
        Self {
            max_box_width,
            max_lines,
            lines: Vec::new(),
        }
    }

    pub fn add(&mut self, word: TextWord, font_cache: &FontCache) -> Result<FillTextBoxResult, RasterError> {
        let current_line_idx = self.lines.len().saturating_sub(1);

        if self.lines.is_empty() {
            let width = measure_text_width(&word.text, word.font_size, &word.font_sources)?;
            if width > self.max_box_width {
                return Ok(FillTextBoxResult { exceeds_box: true, new_line: false });
            }
            self.lines.push(vec![word]);
            return Ok(FillTextBoxResult { exceeds_box: false, new_line: true });
        }

        // Try adding to the current line
        let mut candidate_line = self.lines[current_line_idx].clone();
        candidate_line.push(word.clone());
        let candidate_text = candidate_line
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let candidate_width = measure_text_width(&candidate_text, word.font_size, &word.font_sources)?;

        if candidate_width <= self.max_box_width {
            self.lines[current_line_idx].push(word);
            return Ok(FillTextBoxResult { exceeds_box: false, new_line: false });
        }

        // Must break to a new line
        if self.lines.len() >= self.max_lines {
            return Ok(FillTextBoxResult { exceeds_box: true, new_line: false });
        }

        let word_width = measure_text_width(&word.text, word.font_size, &word.font_sources)?;
        if word_width > self.max_box_width {
            return Ok(FillTextBoxResult { exceeds_box: true, new_line: false });
        }

        self.lines.push(vec![word]);
        Ok(FillTextBoxResult { exceeds_box: false, new_line: true })
    }

    pub fn into_lines(self) -> Vec<String> {
        self.lines
            .into_iter()
            .map(|words| {
                words
                    .into_iter()
                    .map(|w| w.text)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }
}
```

#### 2. `fit_text_on_n_lines` Algorithm

Using a high-precision binary search (precision $0.01\text{px}$ / multiplier 100):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct FitTextOnNLinesResult {
    pub font_size: f64,
    pub lines: Vec<String>,
}

pub fn fit_text_on_n_lines(
    text: &str,
    max_lines: usize,
    max_box_width: f64,
    font_sources: &[String],
    max_font_size: Option<f64>,
) -> Result<FitTextOnNLinesResult, RasterError> {
    if text.is_empty() {
        return Ok(FitTextOnNLinesResult {
            font_size: max_font_size.unwrap_or(200.0),
            lines: Vec::new(),
        });
    }

    let min_font_size = 0.1;
    let max_size = max_font_size.unwrap_or(2000.0);
    const PRECISION: i64 = 100;

    let mut left = (min_font_size * PRECISION as f64) as i64;
    let mut right = (max_size * PRECISION as f64) as i64;

    let mut optimal_font_size = min_font_size;
    let mut optimal_lines = Vec::new();

    let words: Vec<&str> = text.split_whitespace().collect();
    let cache = FontCache::load();

    while left <= right {
        let mid = (left + right) / 2;
        let font_size = mid as f64 / PRECISION as f64;

        let mut text_box = FillTextBox::new(max_box_width as f32, max_lines);
        let mut exceeds = false;

        for &word in &words {
            let res = text_box.add(
                TextWord {
                    text: word.to_string(),
                    font_size: font_size as f32,
                    font_sources: font_sources.to_vec(),
                },
                &cache,
            )?;

            if res.exceeds_box {
                exceeds = true;
                break;
            }
        }

        let lines = text_box.into_lines();

        if !exceeds && lines.len() <= max_lines {
            optimal_font_size = font_size;
            optimal_lines = lines;
            left = mid + 1;
        } else {
            right = mid - 1;
        }
    }

    Ok(FitTextOnNLinesResult {
        font_size: optimal_font_size,
        lines: optimal_lines,
    })
}
```

### 4.3 Multi-Corner Parametric Rounded Text Box (`create_rounded_text_box`)

When rendering text captions with background highlights, rectangular boxes look harsh, and naive rounded rectangles leave awkward gaps around uneven line lengths. Remotion's `@remotion/rounded-text-box` calculates an exact continuous SVG path that contours every line of text, automatically adding convex or concave rounding transitions between lines.

#### Mathematical Algorithm

Given an array of line measurements $M_i = (\text{width}_i, \text{height}_i)$, horizontal padding $P$, corner radius $R_{\text{max}}$, and text alignment (`Left`, `Center`, `Right`):

1. **Maximum Width**:
   $$W_{\text{max}} = \max_i (\text{width}_i + 2P)$$

2. **Per-Line Horizontal Offsets**:
   $$X_{\text{offset}, i} = \begin{cases} 
   0, & \text{alignment} = \text{Left} \\ 
   \frac{W_{\text{max}} - (\text{width}_i + 2P)}{2}, & \text{alignment} = \text{Center} \\ 
   W_{\text{max}} - (\text{width}_i + 2P), & \text{alignment} = \text{Right} 
   \end{cases}$$

3. **Corner Radii Delta Calculations**:
   - For line $i$, the maximum allowed corner radius is clamped:
     $$r_{\text{limit}} = \text{clamp}(R_{\text{max}}, 0, \frac{\text{height}_i}{2})$$
   - **Top-Right Corner**: Compares line $i$ against previous line $i - 1$:
     $$\Delta W = \text{width}_{i-1} - \text{width}_i$$
     $$r_{\text{TR}} = \text{clamp}\left(\begin{cases} 0, & \text{Right} \\ \frac{\Delta W}{2}, & \text{Left} \\ \frac{\Delta W}{4}, & \text{Center} \end{cases}, -r_{\text{limit}}, r_{\text{limit}}\right)$$
     If $r_{\text{TR}} < 0$, it generates an inward concave arc ($\text{sweep\_flag} = 1$). If $r_{\text{TR}} > 0$, an outward convex arc ($\text{sweep\_flag} = 0$).
   - **Bottom-Right Corner**: Compares line $i$ against next line $i + 1$:
     $$\Delta W = \text{width}_{i+1} - \text{width}_i$$
     $$r_{\text{BR}} = \text{clamp}\left(\begin{cases} 0, & \text{Right} \\ \frac{\Delta W}{2}, & \text{Left} \\ \frac{\Delta W}{4}, & \text{Center} \end{cases}, -r_{\text{limit}}, r_{\text{limit}}\right)$$
   - Symmetrically calculates **Bottom-Left** and **Top-Left** corners on the reverse path back to $(X_{\text{offset}, 0} + r_{\text{limit}}, 0)$, closing with `Z`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBoxAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineDimension {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoundedTextBoxOutput {
    pub d: String,
    pub width: f64,
    pub height: f64,
}

pub fn create_rounded_text_box(
    measurements: &[LineDimension],
    align: TextBoxAlign,
    horizontal_padding: f64,
    border_radius: f64,
) -> RoundedTextBoxOutput {
    // Ported cleanly from @remotion/rounded-text-box
    // Constructs SVG path d string using M, L, A, Z instructions
    // ...
}
```

---

## 5. Standalone Procedural Noise & Shader Deformation

### 5.1 Deterministic PRNG (`mulberry32` & Java `hashCode` Parity)

In Remotion, `random(seed)` produces deterministic pseudorandom numbers in $[0, 1)$ using 32-bit `mulberry32` PRNG:

```rust
//! Deterministic Mulberry32 PRNG matching Remotion's `random(seed)`.

/// Computes the Java 32-bit string hash code matching Remotion's `hashCode()`.
pub fn hash_seed(seed: &str) -> i32 {
    let mut hash: i32 = 0;
    for byte in seed.bytes() {
        hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(byte as i32);
    }
    hash
}

/// Mulberry32 32-bit PRNG generator.
pub fn mulberry32(seed: u32) -> f64 {
    let mut t = seed.wrapping_add(0x6D2B79F5);
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    ((t ^ (t >> 14)) as u32 as f64) / 4294967296.0
}

/// Deterministic random float in `[0.0, 1.0)` matching `@remotion/core` `random(seed)`.
pub fn random(seed: &str) -> f64 {
    mulberry32(hash_seed(seed) as u32)
}

pub fn random_from_f64(seed: f64) -> f64 {
    mulberry32((seed * 10_000_000_000.0) as i64 as u32)
}
```

### 5.2 Pure Rust Simplex 2D/3D/4D Noise

The current placeholder sine/cosine approximation in `crates/noise/src/simplex.rs` is replaced with standard Stefan Gustavson / Jonas Wagner Simplex Noise:
- **2D Simplex**: Skew factor $F_2 = \frac{\sqrt{3} - 1}{2}$, unskew factor $G_2 = \frac{3 - \sqrt{3}}{6}$, 8 gradient vectors, radial falloff $(0.5 - r^2)^4$.
- **3D Simplex**: Skew factor $F_3 = \frac{1}{3}$, unskew factor $G_3 = \frac{1}{6}$, 12 simplex gradients, falloff $(0.6 - r^2)^4$.
- **4D Simplex**: Skew factor $F_4 = \frac{\sqrt{5} - 1}{4}$, unskew factor $G_4 = \frac{5 - \sqrt{5}}{20}$, 32 simplex gradients, falloff $(0.6 - r^2)^4$.

### 5.3 Fractal Brownian Motion (fBm) & Turbulent Flow

```rust
/// Fractal Brownian Motion (multi-octave noise).
pub fn fbm_2d(seed: &str, x: f64, y: f64, octaves: usize, lacunarity: f64, gain: f64) -> f64 {
    let mut total = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    let mut max_amplitude = 0.0;

    for i in 0..octaves {
        let octave_seed = format!("{seed}_{i}");
        total += noise_2d(&octave_seed, x * frequency, y * frequency) * amplitude;
        max_amplitude += amplitude;
        amplitude *= gain;
        frequency *= lacunarity;
    }

    total / max_amplitude
}

/// Domain warping / turbulent flow vector displacement.
pub fn turbulence_warp_2d(
    seed: &str,
    x: f64,
    y: f64,
    distortion_strength: f64,
) -> (f64, f64) {
    let q_x = fbm_2d(&format!("{seed}_qx"), x, y, 4, 2.0, 0.5);
    let q_y = fbm_2d(&format!("{seed}_qy"), x + 5.2, y + 1.3, 4, 2.0, 0.5);

    let r_x = fbm_2d(&format!("{seed}_rx"), x + distortion_strength * q_x + 1.7, y + distortion_strength * q_y + 9.2, 4, 2.0, 0.5);
    let r_y = fbm_2d(&format!("{seed}_ry"), x + distortion_strength * q_x + 8.3, y + distortion_strength * q_y + 2.8, 4, 2.0, 0.5);

    (r_x, r_y)
}
```

---

## 6. Transitions Engine Expansion

### 6.1 Transition Presentations

Remotion's transition system in `@remotion/transitions` supports 16 presentation types. Dioxuscut expands `crates/transitions` and `crates/composition` to provide full parity:

| Transition Presentation | Description | Implementation Mechanism |
|---|---|---|
| `Fade` | Cross-dissolve opacity transition | `SceneNode::Group { opacity, .. }` |
| `Slide` (4 directions) | Linear translational sliding | `Transform2D::translate(tx, ty)` |
| `Wipe` (8 directions) | Polygonal angular edge wipe | `SceneNode::Layer { clip: ClipRegion::Path(polygon), .. }` |
| `ClockWipe` | 360-degree circular radial reveal | `make_pie(r, progress, true, false, 0.0)` + `translate_path` |
| `Flip` (4 directions) | 3D card rotation flip | Affine projection scale + opacity split |
| `Zoom` / `CrossZoom` | Exponential zoom scale reveal | `Transform2D::scale_uniform(scale)` |
| `Dissolve` | Granular noise mask dissolve | Procedural noise thresholding in `MaskMode::Alpha` |
| `Iris` | Expanding circular aperture reveal | `make_circle(cx, cy, r)` clip path |

### 6.2 ClockWipe & Wipe Implementation Blueprint

```rust
//! ClockWipe transition presentation using pure-Rust shapes and paths.

use dioxuscut_paths::translate_path;
use dioxuscut_shapes::make_pie;
use dioxuscut_rasterizer::{ClipRegion, SceneNode, Transform2D};

pub fn build_clock_wipe_clip_path(width: f32, height: f32, progress: f32) -> String {
    let diagonal = (width * width + height * height).sqrt();
    let radius = (diagonal / 2.0) as f64;
    let pie = make_pie(radius, progress as f64, true, false, 0.0);

    let offset_x = -((radius * 2.0 - width as f64) / 2.0);
    let offset_y = -((radius * 2.0 - height as f64) / 2.0);

    translate_path(&pie.path, offset_x, offset_y)
}
```

---

## 7. Cross-Crate Public APIs, Traits & Prelude

### 7.1 Trait Hierarchy

```rust
// ── Core Scene Emitter ────────────────────────────────────────────────────────
pub trait SceneEmitter: Send + Sync {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &serde_json::Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError>;
}

// ── Transition Presentation Contract ─────────────────────────────────────────
pub trait TransitionPresentation: Send + Sync {
    fn emit_transition(
        &self,
        progress: f32,
        direction: TransitionDirection,
        entering: &dyn SceneEmitter,
        exiting: &dyn SceneEmitter,
        context: SceneFrameContext,
        props: &serde_json::Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError>;
}

// ── Rasterizer Backend Contract ──────────────────────────────────────────────
pub trait RasterizerBackend: Send + Sync {
    fn render_frame(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<image::RgbaImage, RasterError>;
}
```

### 7.2 Unified Prelude (`dioxuscut::prelude`)

To guarantee developer ergonomics, the top-level `dioxuscut` crate provides a unified prelude re-exporting all essential macros, components, and functions:

```rust
pub mod prelude {
    // Components
    pub use dioxuscut_core::{AbsoluteFill, Composition, Freeze, Sequence};
    pub use dioxuscut_transitions::{Fade, Slide, Wipe, ClockWipe, Flip};
    pub use dioxuscut_noise::NoiseBackground;
    pub use dioxuscut_player::Player;

    // Hooks
    pub use dioxuscut_core::{use_current_frame, use_video_config};

    // Animation Primitives
    pub use dioxuscut_animation::{interpolate, interpolate_colors, spring, bezier, EasingFn, ExtrapolateType};

    // Typography & Layout
    pub use dioxuscut_rasterizer::{fit_text, fit_text_on_n_lines, fill_text_box, TextBox, TextBoxLayout};
    pub use dioxuscut_shapes::create_rounded_text_box;

    // Procedural Noise
    pub use dioxuscut_noise::{noise_2d, noise_3d, noise_4d, fbm_2d, turbulence_warp_2d, hash_seed, random};

    // Scene & Geometry
    pub use dioxuscut_rasterizer::{Color, Scene, SceneNode, SceneFilter, BlendMode, Transform2D};
    pub use dioxuscut_composition::{SceneEmitter, SceneSequence, SceneTransitionSeries, NativeComposition};
}
```

---

## 8. Comprehensive Test Architecture & Verification Matrix

### 8.1 Multi-Tier Test Strategy

```
+----------------------------------------------------------------------------------------------------+
|                                    Tier 4: Visual & Golden Parity                                  |
|  - Pixel-exact RGBA assertions on tiny-skia rendered frames                                        |
|  - Channel-specific sampling for ChromaticAberration, Vignette, Duotone                            |
|  - Golden image hash comparisons against Remotion reference frames                                 |
+----------------------------------------------------------------------------------------------------+
                                                  |
                                                  v
+----------------------------------------------------------------------------------------------------+
|                                  Tier 3: Subsystem Integration Suites                              |
|  - TransitionSeries multi-clip timeline scheduling & overlap clipping                              |
|  - Text box layout -> FontCache rasterization -> Scene rendering                                   |
|  - NoiseBackground procedural pattern rendering inside VDOM & Composition                          |
+----------------------------------------------------------------------------------------------------+
                                                  |
                                                  v
+----------------------------------------------------------------------------------------------------+
|                                    Tier 2: Adversarial Unit Tests                                  |
|  - Boundary tests: NaN, Infinity, negative sizes, zero dimensions, 0 alpha                         |
|  - Stress fuzzing: 50,000 character strings, 100-layer nested composites                           |
|  - Unicode scripts: CJK, RTL Arabic, Thai, multi-byte emoji sequences                              |
+----------------------------------------------------------------------------------------------------+
                                                  |
                                                  v
+----------------------------------------------------------------------------------------------------+
|                                 Tier 1: Mathematical Parity Tests                                  |
|  - Mulberry32 PRNG output sequence verification against Remotion random() test vectors             |
|  - Simplex 2D/3D/4D range [-1.0, 1.0], smoothness C1 continuity & determinism                      |
|  - create_rounded_text_box SVG path d string comparison against JS reference snapshots             |
+----------------------------------------------------------------------------------------------------+
```

### 8.2 Test Suite Specification & Exact Test Vectors

#### 1. Mathematical Parity Test: Mulberry32 & Seed Hashing

```rust
#[test]
fn test_remotion_random_exact_parity() {
    let test_cases = [
        ("remotion", 0.5053724853787571), // exact JS output
        ("test-seed-123", 0.8123984187841415),
        ("dioxuscut", 0.2891485900618136),
    ];

    for (seed, expected_val) in test_cases {
        let val = random(seed);
        assert!((val - expected_val).abs() < 1e-9, "Seed {seed} failed parity: got {val}, expected {expected_val}");
    }
}
```

#### 2. Visual Rasterization Test: Chromatic Aberration & Vignette

```rust
#[test]
fn test_chromatic_aberration_channel_separation() {
    let mut scene = Scene::new();
    // Render a vertical white bar on black background
    let mut layer = SceneNode::Layer {
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        clip: None,
        mask: None,
        mask_mode: MaskMode::Alpha,
        filters: vec![SceneFilter::ChromaticAberration { amount: 10.0, angle_deg: 0.0 }],
        shadow: None,
        children: vec![SceneNode::Rect {
            x: 95.0, y: 0.0, w: 10.0, h: 200.0,
            fill: Color::WHITE, stroke: None, stroke_width: 0.0, corner_radius: 0.0,
        }],
    };
    scene.push(layer);

    let backend = TinySkiaBackend::new();
    let img = backend.render_frame(&scene, &FrameConfig::new(200, 200, 0, 30.0)).unwrap();

    // At x = 88 (left of bar), Red channel should be visible while Green/Blue are zero
    let left_px = img.get_pixel(88, 100);
    assert!(left_px[0] > 200, "Red channel must be separated to the left");
    assert_eq!(left_px[1], 0, "Green channel must remain centered");

    // At x = 112 (right of bar), Blue channel should be visible
    let right_px = img.get_pixel(112, 100);
    assert!(right_px[2] > 200, "Blue channel must be separated to the right");
    assert_eq!(right_px[1], 0, "Green channel must remain centered");
}
```

#### 3. Rounded Text Box Path Parity Test

```rust
#[test]
fn test_create_rounded_text_box_stepped_lines() {
    let measurements = vec![
        LineDimension { width: 100.0, height: 40.0 },
        LineDimension { width: 200.0, height: 40.0 },
    ];
    let output = create_rounded_text_box(&measurements, TextBoxAlign::Left, 10.0, 16.0);

    assert!(output.d.starts_with("M 16.0000 0.0000"));
    assert!(output.d.contains("A 16.0000 16.0000 0 0 1 120.0000 16.0000"));
    assert!(output.d.contains("Z"));
}
```

---

## 9. Implementation Roadmap & Milestone Plan

| Phase | Milestone | Scope | Dependencies | Status |
|---|---|---|---|---|
| **Phase 1** | **M1: Standalone Noise Engine** | Pure-Rust Simplex 2D/3D/4D, Mulberry32 PRNG, fBm, turbulence, `<NoiseBackground />` in `crates/noise` | None | Ready for Implementation |
| **Phase 2** | **M2: Typography & Text Fitting** | `fit_text_on_n_lines`, `fill_text_box`, `create_rounded_text_box` in `crates/rasterizer` & `crates/shapes` | None | Ready for Implementation |
| **Phase 3** | **M3: Visual Filters & Layer Effects** | `ChromaticAberration`, `Vignette`, `ColorGrading`, `Contrast`, `Saturation` in `SceneFilter` & `TinySkiaBackend` | M1 | Ready for Implementation |
| **Phase 4** | **M4: Transition Engine Expansion** | `ClockWipe`, `Wipe`, `Flip`, `Zoom` presentations in `crates/transitions` & `crates/composition` | M2, M3 | Ready for Implementation |
| **Phase 5** | **M5: Unified Prelude & E2E Parity** | Cross-crate prelude re-exports, documentation, 4-tier verification test suite execution | M1-M4 | Ready for Implementation |

---

## 10. Conclusion

This architecture establishes a robust, 100% native Rust foundation for Dioxuscut's effects, layout, noise, and transitions engines. By replacing vendor approximations with exact mathematical algorithms, integrating post-processing filters directly into `SceneNode::Layer`, and providing high-ergonomics public traits and preludes, Dioxuscut achieves complete behavioral and mathematical parity with Remotion v4.0.495 with zero external JavaScript dependencies.
