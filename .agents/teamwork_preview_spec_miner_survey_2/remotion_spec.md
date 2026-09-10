# Remotion v4.0.495 Porting Specification Report
**Target System:** Dioxuscut Pure Rust Native Architecture  
**Author:** Specification Miner 2 (`teamwork_preview_spec_miner_survey_2`)  
**Timestamp:** 2026-08-21T06:35:00Z  
**Reference Sources:** `vendor/remotion-4.0.495/packages/` (`noise`, `transitions`, `effects`, `layout-utils`, `rounded-text-box`, `shapes`, `paths`, `core`)

---

## Executive Summary & Scope

This specification provides the exhaustive mathematical, algorithmic, and architectural blueprint for porting Remotion v4.0.495 packages (`@remotion/noise`, `@remotion/transitions`, `@remotion/effects`, `@remotion/layout-utils`, `@remotion/rounded-text-box`) to 100% native Rust implementations across Dioxuscut crates (`crates/noise`, `crates/transitions`, `crates/rasterizer`, `crates/core`).

All features are designed to execute in headless server environments, CI test runners, WASM web targets, and desktop native binaries with **zero runtime dependency on NodeJS, Bun, browser DOM, WebGL contexts, or the `vendor/` directory**.

---

## Features Discovered

| # | Category | Feature | Description | Inputs | Outputs | Error Behavior | Discovered Via |
|---|----------|---------|-------------|--------|---------|----------------|----------------|
| 1 | Noise | `noise2D` | Deterministic 2D Simplex noise generator evaluated at coordinate $(x, y)$ | `seed: &str \| i64 \| f64`, `x: f64`, `y: f64` | `f64` in `[-1.0, 1.0]` | Non-finite coords propagate NaN/clamped | `@remotion/noise/src/index.ts` |
| 2 | Noise | `noise3D` | Deterministic 3D Simplex noise generator evaluated at $(x, y, z)$ | `seed: &str \| i64 \| f64`, `x: f64`, `y: f64`, `z: f64` | `f64` in `[-1.0, 1.0]` | Non-finite coords propagate NaN/clamped | `@remotion/noise/src/index.ts` |
| 3 | Noise | `noise4D` | Deterministic 4D Simplex noise generator evaluated at $(x, y, z, w)$ | `seed: &str \| i64 \| f64`, `x: f64`, `y: f64`, `z: f64`, `w: f64` | `f64` in `[-1.0, 1.0]` | Non-finite coords propagate NaN/clamped | `@remotion/noise/src/index.ts` |
| 4 | Noise | `random` & `mulberry32` | Remotion deterministic pseudo-random number generator (Mulberry32 + 32-bit HashCode) | `seed: &str \| f64 \| None` | `f64` in `[0.0, 1.0)` | Invalid types error; `None` / `null` produces non-deterministic random | `core/src/random.ts` |
| 5 | Noise | Fractal Brownian Motion (`fBm`) | Multi-octave harmonic noise superposition with persistence & lacunarity | `point`, `octaves: u32`, `lacunarity: f64`, `gain: f64` | `f64` in `[-1.0, 1.0]` | `octaves == 0` returns 0.0 | `effects/src/liquid-contours.ts`, `roughen-edges.ts` |
| 6 | Noise | Turbulent Flow & Domain Warping | Directional domain perturbation and curl/turbulence deformation for paths & shapes | `point`, `strength: f64`, `octaves: u32` | `(f64, f64)` displaced vector | Negative strength inverts displacement | `effects/src/liquid-contours.ts`, `noise-displacement.ts` |
| 7 | Noise | `<NoiseBackground />` | Procedural animated SVG / canvas background component | `seed`, `base_color`, `accent_color`, `speed`, `style` | Dioxus RSX Element | Fallback to default styling | `crates/noise/src/noise_bg.rs`, `docs/noise-visualization.mdx` |
| 8 | Transitions | `ClockWipe` | Circular sector wipe revealing new scene clockwise from top center | `width: f64`, `height: f64`, `progress: f64`, `direction: PresentationDirection` | SVG clip-path / raster mask | Clamp `progress` to `[0.0, 1.0]` | `transitions/src/presentations/clock-wipe.tsx` |
| 9 | Transitions | `LinearWipe` (`wipe`) | 8-direction polygonal wipe transition | `direction: WipeDirection`, `progress: f64` | Polygon clip path | Unknown direction errors | `transitions/src/presentations/wipe.tsx` |
| 10 | Transitions | `Flip` | 3D perspective flip (180° rotation) around X or Y axis with backface culling | `direction: FlipDirection`, `perspective: f64`, `progress: f64` | 3D transform matrix + cull flag | Perspective <= 0 rejected | `transitions/src/presentations/flip.tsx` |
| 11 | Transitions | `ZoomInOut` | Dual-phase smoothstep zoom-in / zoom-out transition | `progress: f64`, textures: `u_prev`, `u_next` | Blended pixel buffer | Clamped progress | `transitions/src/presentations/zoom-in-out.tsx` |
| 12 | Transitions | `CrossZoom` | Directional center zoom with exponential crossfade and sinusoidal blur strength | `strength: f64` (def: 0.4), `progress: f64` | Multi-sampled blended pixel buffer | Negative strength clamped | `transitions/src/presentations/cross-zoom.tsx` |
| 13 | Transitions | `Slide` | 4-direction push slide with sub-pixel epsilon gap compensation | `direction: SlideDirection`, `progress: f64` | Translation offsets `(dx, dy)` | Invalid direction errors | `transitions/src/presentations/slide.tsx` |
| 14 | Transitions | `Fade` | Linear alpha crossfade with optional exiting scene fade-out suppression | `should_fade_out_exiting_scene: bool`, `progress: f64` | Alpha opacities `(a_in, a_out)` | Clamped progress | `transitions/src/presentations/fade.tsx` |
| 15 | Transitions | `Dissolve` | Luminance-driven burn dissolution with spread and hot flame color highlights | `line_width`, `spread_color`, `hot_color`, `pow`, `intensity`, `progress` | Shader composite output | Invalid hex color errors | `transitions/src/presentations/dissolve.tsx` |
| 16 | Transitions | `Iris` | Center-expanding circular aperture mask | `width: f64`, `height: f64`, `progress: f64` | Circular clip path | Clamped progress | `transitions/src/presentations/iris.tsx` |
| 17 | Transitions | `Ripple` | Radial sinusoidal wave displacement crossfade from center | `amplitude: f64` (def: 100.0), `speed: f64` (def: 50.0), `progress: f64` | Displaced texture blend | Negative amplitude / speed handled | `transitions/src/presentations/ripple.tsx` |
| 18 | Transitions | `BookFlip` | 3D page curl / skew transition across 4 directions | `direction: BookFlipDirection`, `progress: f64` | Skew-transformed pixel blend | Epsilon division avoidance | `transitions/src/presentations/book-flip.tsx` |
| 19 | Transitions | `LinearBlur` | Multi-pass 2D box blur crossfade peaking at mid-transition | `intensity: f64` (def: 0.1), `progress: f64` | 20x20 sample blurred composite | Clamped sample grid | `transitions/src/presentations/linear-blur.tsx` |
| 20 | Transitions | `linearTiming` | Time interpolation with custom easing curves and clamp extrapolation | `duration_in_frames: u32`, `easing: Option<EasingFn>` | Frame-to-progress mapping `[0.0, 1.0]` | Zero duration returns 1.0 | `transitions/src/timings/linear-timing.ts` |
| 21 | Transitions | `springTiming` | Damped harmonic oscillator timing with mass, tension, friction | `config: SpringConfig`, `fps: f64`, `duration_in_frames: Option<u32>` | Physics progress curve `[0.0, 1.0]` | Unstable physics clamped to rest | `transitions/src/timings/spring-timing.ts` |
| 22 | Transitions | `TransitionSeries` | Timeline sequence organizer computing track overlaps and local frame intervals | `sequences: Vec<Sequence>`, `transitions: Vec<Transition>` | Overlapped composite timeline | Overlap > duration errors | `transitions/src/TransitionSeries.tsx` |
| 23 | Visual Effects | `ChromaticAberration` | RGB channel spatial separation along an angle vector | `amount: f32` (def: 8.0px), `angle: f32` (def: 0.0 deg) | Color-shifted layer pixmap | Negative amount clamped | `effects/src/chromatic-aberration/` |
| 24 | Visual Effects | `Vignette` | Parametric edge darkening / alpha fade with roundness interpolation | `amount`, `radius`, `feather`, `roundness`, `color`, `mode`, `center` | Masked / blended layer pixmap | Invalid enum / non-finite errors | `effects/src/vignette.ts` |
| 25 | Visual Effects | `Brightness` | Per-pixel RGB addition/subtraction preserving alpha | `amount: f32` in `[-1.0, 1.0]` (def: 0.0) | Brightness adjusted pixmap | Clamped color channels `[0, 255]` | `effects/src/brightness.ts` |
| 26 | Visual Effects | `Contrast` | Midpoint (128) scaled color contrast expansion/contraction | `amount: f32` in `[0.0, inf)` (def: 1.0) | Contrast adjusted pixmap | Negative amount rejected | `effects/src/contrast.ts` |
| 27 | Visual Effects | `Grayscale` | Standard Rec.709 / Rec.601 luminance matrix mixing | `amount: f32` in `[0.0, 1.0]` (def: 1.0) | Desaturated pixmap | Out-of-bounds amount clamped | `effects/src/grayscale.ts` |
| 28 | Visual Effects | `HueRotate` | RGB color rotation in 3D color space by degree angle | `degrees: f32` (def: 0.0 deg) | Hue rotated pixmap | Modulo 360 rotation | `effects/src/hue.ts` |
| 29 | Visual Effects | `Invert` | Linear RGB channel color inversion | `amount: f32` in `[0.0, 1.0]` (def: 1.0) | Inverted pixmap | Unit interval clamped | `effects/src/invert.ts` |
| 30 | Visual Effects | `Saturation` | Color saturation scaling matrix | `amount: f32` in `[0.0, inf)` (def: 1.0) | Saturated pixmap | Negative amount rejected | `effects/src/saturation.ts` |
| 31 | Visual Effects | `Tint` | Flat color overlay respecting source alpha mask (`source-atop`) | `color: Color`, `amount: f32` in `[0.0, 1.0]` (def: 0.5) | Tinted pixmap | Invalid color rejected | `effects/src/tint.ts` |
| 32 | Visual Effects | `Duotone` | Two-tone color mapping based on luminance threshold | `dark_color: Color`, `light_color: Color`, `threshold: f32` (def: 0.5) | Two-tone recolored pixmap | Alpha <= 0.001 preserved | `effects/src/duotone.ts` |
| 33 | Visual Effects | `ColorKey` | Green/Blue/Red screen chroma keying with smoothstep and spill suppression | `key_color: Color`, `similarity: f32`, `smoothness: f32`, `spill_suppression: f32` | Keyed transparent pixmap | SQRT(3) normalized distance | `effects/src/color-key.ts` |
| 34 | Visual Effects | `NoiseDisplacement` | Localized noise grain texture displacement with directional bias | `center`, `radius`, `strength`, `seed`, `grain_size`, `passes`, `blur`, `feather`, `bias_direction`, `bias_amount` | Distorted pixmap | Clamped UV & passes `[1, 12]` | `effects/src/noise-displacement.ts` |
| 35 | Visual Effects | `WhiteNoise` | Uniform PRNG grain layer blend | `amount: f32` (def: 1.0), `seed: f32` (def: 0.0) | Grain-mixed pixmap | Clamped amount `[0.0, 1.0]` | `effects/src/white-noise.ts` |
| 36 | Layout Utils | `fit_text_on_n_lines` | Binary search font auto-scaling to fit multi-line word-wrapped text into bounding box | `text: &str`, `max_lines: usize`, `max_box_width: f64`, `font_family`, `max_font_size` | `{ font_size: f64, lines: Vec<String> }` | Unbreakable word forces min font size | `layout-utils/src/layouts/fit-text-on-n-lines.ts` |
| 37 | Layout Utils | `fill_text_box` | Greedy word-by-word line-wrapping algorithm respecting `max_box_width` and `max_lines` | `max_box_width: f64`, `max_lines: usize` | State machine `add(word) -> { exceeds_box, new_line }` | Overflow on last line triggers `exceeds_box: true` | `layout-utils/src/layouts/fill-text-box.ts` |
| 38 | Layout Utils | `fit_text` | Single line font size scaling via ratio against reference sample size (100px) | `text: &str`, `within_width: f64`, `font_family: &str` | `{ font_size: f64 }` | Non-positive width errors | `layout-utils/src/layouts/fit-text.ts` |
| 39 | Layout Utils | `create_rounded_text_box` | Parametric rounded text badge background path generation with per-corner radii and width deltas | `text_measurements: Vec<Dimensions>`, `text_align: TextAlign`, `horizontal_padding: f64`, `border_radius: f64` | `{ d: String, bounding_box: Rect, instructions: Vec<ReducedInstruction> }` | Clamps corner radius to line height / 2 | `rounded-text-box/src/create-rounded-text-box.ts` |

---

## 1. Procedural Noise, Seeding & Shaders

### 1.1 Seeding Mathematics & PRNG

Remotion's deterministic pseudo-random number generator consists of two core components:
1. `hashCode(str)`: A 32-bit integer string hash function equivalent to Java's string hashing algorithm:
   $$\text{hash}_{k} = (\text{hash}_{k-1} \cdot 31 + c_k) \pmod{2^{32}}$$
   In 32-bit signed two's complement integer arithmetic:
   $$\text{hash} = ((\text{hash} \ll 5) - \text{hash} + \text{charCode}) \mid 0$$

2. `mulberry32(seed)`: A 32-bit generator producing uniform pseudo-random floats in $[0.0, 1.0)$:
   $$t_0 = (\text{seed} + \text{0x6D2B79F5}) \pmod{2^{32}}$$
   $$t_1 = (t_0 \oplus (t_0 \gg 15)) \cdot (t_0 \mid 1) \pmod{2^{32}}$$
   $$t_2 = t_1 \oplus (t_1 + (t_1 \oplus (t_1 \gg 7)) \cdot (t_1 \mid 61)) \pmod{2^{32}}$$
   $$\text{output} = \frac{(t_2 \oplus (t_2 \gg 14)) \gg 0}{4294967296.0}$$

When `seed` is a number: $a = \lfloor \text{seed} \cdot 10^{10} \rfloor \pmod{2^{32}}$.

### 1.2 Simplex Noise (2D, 3D, 4D) Algorithms

Remotion delegates to `simplex-noise 4.0.1`. The permutation table $P$ of length 512 is initialized using Fisher-Yates shuffle with `mulberry32`:
1. Initialize $p[i] = i$ for $i \in [0, 255]$.
2. For $i$ from 0 to 254:
   $$r = i + \lfloor \text{random}() \cdot (256 - i) \rfloor$$
   $$\text{swap}(p[i], p[r])$$
3. Populate table $T[i] = p[i \ \& \ 255]$ for $i \in [0, 511]$.

#### 2D Simplex Noise Formulation
- **Skew factor:** $F_2 = \frac{\sqrt{3} - 1}{2} \approx 0.3660254037844386$
- **Unskew factor:** $G_2 = \frac{3 - \sqrt{3}}{6} \approx 0.21132486540518713$
- Skew coordinates: $s = (x + y) \cdot F_2$, $i = \lfloor x + s \rfloor$, $j = \lfloor y + s \rfloor$.
- Unskewed origin: $t = (i + j) \cdot G_2$, $X_0 = i - t$, $Y_0 = j - t$.
- Internal offset: $x_0 = x - X_0$, $y_0 = y - Y_0$.
- Simplex step: if $x_0 > y_0$, $(i_1, j_1) = (1, 0)$ else $(i_1, j_1) = (0, 1)$.
- Corner offsets:
  - $(x_1, y_1) = (x_0 - i_1 + G_2, y_0 - j_1 + G_2)$
  - $(x_2, y_2) = (x_0 - 1.0 + 2G_2, y_0 - 1.0 + 2G_2)$
- Gradient contributions: For each corner $k \in \{0, 1, 2\}$:
  $$t_k = 0.5 - x_k^2 - y_k^2$$
  $$\text{if } t_k > 0: n_k = t_k^4 \cdot (\vec{g}_k \cdot (x_k, y_k)) \text{ else } 0.0$$
- Scaling factor: $\text{noise2D}(x, y) = 70.0 \cdot (n_0 + n_1 + n_2)$, normalized to $[-1.0, 1.0]$.

#### 3D Simplex Noise Formulation
- **Skew factor:** $F_3 = \frac{1}{3}$, **Unskew factor:** $G_3 = \frac{1}{6}$.
- 4 simplex corners, contribution threshold $t_k = 0.6 - x_k^2 - y_k^2 - z_k^2$.
- Scaling factor: $\text{noise3D}(x, y, z) = 32.0 \cdot \sum_{k=0}^3 n_k \in [-1.0, 1.0]$.

#### 4D Simplex Noise Formulation
- **Skew factor:** $F_4 = \frac{\sqrt{5} - 1}{4} \approx 0.30901699437494745$, **Unskew factor:** $G_4 = \frac{5 - \sqrt{5}}{20} \approx 0.1381966011250105$.
- 5 simplex corners, contribution threshold $t_k = 0.6 - x_k^2 - y_k^2 - z_k^2 - w_k^2$.
- Scaling factor: $\text{noise4D}(x, y, z, w) = 27.0 \cdot \sum_{k=0}^4 n_k \in [-1.0, 1.0]$.

### 1.3 Fractal Brownian Motion (fBm) & Turbulent Flow

- **fBm formula:**
  $$\text{fBm}(\vec{p}) = \frac{\sum_{o=0}^{O-1} A \cdot \gamma^o \cdot \text{noise}(\mathbf{R}^o \cdot \vec{p} \cdot \lambda^o + \vec{\delta}_o)}{\sum_{o=0}^{O-1} A \cdot \gamma^o}$$
  where $\lambda = 2.03$ (lacunarity), $\gamma = 0.55$ (gain/persistence), $\mathbf{R} = \begin{pmatrix} 0.80 & 0.60 \\ -0.60 & 0.80 \end{pmatrix}$.

- **Turbulent Flow formula:**
  $$\text{Turbulence}(\vec{p}) = \sum_{o=0}^{O-1} \gamma^o \cdot |\text{noise}(\lambda^o \cdot \vec{p})|$$

- **Domain Warping:**
  $$\vec{q} = \begin{pmatrix} \text{fBm}(\vec{p} + (5.2, 1.3)) \\ \text{fBm}(\vec{p} + (1.7, 9.2)) \end{pmatrix} - \begin{pmatrix} 0.5 \\ 0.5 \end{pmatrix}$$
  $$\vec{r} = \begin{pmatrix} \text{fBm}(\vec{p} + 4.0\vec{q} + (1.7, 9.2)) \\ \text{fBm}(\vec{p} + 4.0\vec{q} + (8.3, 2.8)) \end{pmatrix}$$
  $$\text{WarpedField}(\vec{p}) = \text{fBm}(\vec{p} + \text{strength} \cdot \vec{r})$$

---

## 2. Transitions & Easing Engine

### 2.1 Presentation Wipe Transitions

#### 1. Clock Wipe (`clockWipe`)
- Center origin: $(x_c, y_c) = (w/2, h/2)$.
- Radius: $R = \frac{\sqrt{w^2 + h^2}}{2}$.
- Geometry: Arc from $-90^\circ$ (top 12 o'clock) clockwise through angle $\theta = \text{progress} \cdot 360^\circ$.
- SVG Path:
  - Move to center: `M (w/2, h/2)`
  - Line to top: `L (w/2, h/2 - R)`
  - Arc to end coordinate: `A R R 0 largeArcFlag 1 (x_end, y_end)`
  - Close to center: `Z`
- Clip applied strictly to entering slide; exiting slide remains fully visible below.

#### 2. Linear Wipe (`wipe`)
Given $p = \text{progress} \cdot 100$:
- `from-left`: Entering polygon `polygon(0% 0%, p% 0%, p% 100%, 0% 100%)`.
- `from-top`: Entering polygon `polygon(0% 0%, 100% 0%, 100% p%, 0% p%)`.
- `from-right`: Entering polygon `polygon(100% 0%, 100% 100%, (100-p)% 100%, (100-p)% 0%)`.
- `from-bottom`: Entering polygon `polygon(0% 100%, 100% 100%, 100% (100-p)%, 0% (100-p)%)`.
- `from-top-left`: Triangular wedge `polygon(0% 0%, (2p)% 0%, 0% (2p)%)`.
- `from-top-right`: Triangular wedge `polygon(100% 0%, (100-2p)% 0%, 100% (2p)%)`.
- `from-bottom-right`: Triangular wedge `polygon(100% 100%, (100-2p)% 100%, 100% (100-2p)%)`.
- `from-bottom-left`: Triangular wedge `polygon(0% 100%, 0% (100-2p)%, (2p)% 100%)`.

#### 3. 3D Perspective Flip (`flip`)
- Perspective distance: $d = 1000\text{px}$.
- Rotation axis: `rotateY` for horizontal directions (`from-left`, `from-right`); `rotateX` for vertical (`from-top`, `from-bottom`).
- Rotation angles:
  - Entering: rotates from $-180^\circ \to 0^\circ$ (or $180^\circ \to 0^\circ$).
  - Exiting: rotates from $0^\circ \to 180^\circ$ (or $0^\circ \to -180^\circ$).
- Backface culling: Render layer only when surface normal satisfies $\cos(\theta) \ge 0$.

#### 4. Zoom-In-Out (`zoomInOut`)
- Inverted time: $t = 1.0 - \text{time}$.
- Zoom scales:
  $$\text{zoomFrom} = \text{smoothstep}(0.0, 1.0, t \cdot 2.0)$$
  $$\text{zoomTo} = \text{smoothstep}(0.0, 1.0, (1.0 - t) \cdot 2.0)$$
  $$\text{crossfade} = \text{smoothstep}(0.4, 0.6, t)$$
- Texture sampling:
  $$\vec{uv}_{\text{prev}} = 0.5 + (\vec{uv} - 0.5) \cdot (1.0 - \text{zoomFrom})$$
  $$\vec{uv}_{\text{next}} = 0.5 + (\vec{uv} - 0.5) \cdot (1.0 - \text{zoomTo})$$
  $$\text{color} = \text{mix}(\text{sample}(\text{prev}, \vec{uv}_{\text{prev}}), \text{sample}(\text{next}, \vec{uv}_{\text{next}}), \text{crossfade})$$

#### 5. Cross-Zoom (`crossZoom`)
- Center drift: $\text{center} = (\text{linearEase}(0.25, 0.5, 1.0, p), 0.5)$.
- Dissolve curve: $\text{dissolve} = \text{exponentialEaseInOut}(0.0, 1.0, 1.0, p)$.
- Strength envelope: $\text{strength} = \text{sinusoidalEaseInOut}(0.0, u_{\text{strength}}, 0.5, p)$.
- Multi-tap radial zoom blur: 40 samples along $\vec{uv} \to \text{center}$ with parabolic tap weighting $w_i = 4 \cdot (\tau - \tau^2)$.

#### 6. Push Slide (`slide`)
- Directional translation with subpixel epsilon seam compensation ($\epsilon = 0.01\%$):
  - Exiting `from-left`: $x = \text{progress} \cdot 100 - \epsilon$.
  - Entering `from-left`: $x = -100 + \text{progress} \cdot 100$.

### 2.2 Timing Functions & Physics Springs

1. **`linearTiming`:**
   $$p(f) = \text{clamp}\left(\frac{f}{D}, 0.0, 1.0\right)$$
   Optionally mapped through easing $E(p(f))$.

2. **`springTiming`:**
   Damped harmonic oscillator equation:
   $$m \frac{d^2 x}{dt^2} + c \frac{dx}{dt} + k x = 0$$
   - Mass $m$, Stiffness/Tension $k$, Damping/Friction $c$.
   - Undamped angular frequency $\omega_0 = \sqrt{k / m}$, Damping ratio $\zeta = \frac{c}{2\sqrt{m k}}$.
   - Behavior for $\zeta < 1$ (underdamped):
     $$x(t) = 1.0 - e^{-\zeta \omega_0 t} \left( \cos(\omega_d t) + \frac{\zeta \omega_0}{\omega_d} \sin(\omega_d t) \right)$$
     where $\omega_d = \omega_0 \sqrt{1 - \zeta^2}$.
   - Rest measurement threshold: $|x(t) - 1.0| < \epsilon_{\text{rest}} \land |v(t)| < \epsilon_{\text{rest}}$.

### 2.3 Timeline Transition Overlap Algorithm

Given a sequence of video clips $S_0, S_1, \dots, S_{N-1}$ with durations $D_0, D_1, \dots, D_{N-1}$ and transitions $T_0, T_1, \dots, T_{N-2}$ with transition durations $\Delta_0, \Delta_1, \dots, \Delta_{N-2}$:
- Start time for clip $S_k$:
  $$\text{start}_0 = 0$$
  $$\text{start}_k = \text{start}_{k-1} + D_{k-1} - \Delta_{k-1}$$
- Total composition duration:
  $$D_{\text{total}} = \sum_{k=0}^{N-1} D_k - \sum_{j=0}^{N-2} \Delta_j$$
- Transition active window for $T_k$:
  $$\text{time} \in [\text{start}_{k+1}, \text{start}_{k+1} + \Delta_k]$$
  Local transition frame $f_{\text{local}} = \text{time} - \text{start}_{k+1} \in [0, \Delta_k]$.

---

## 3. Post-Processing & Visual Effects Engine

All visual effects operate directly on offscreen `tiny-skia::Pixmap` (CPU) and WebGPU fragment shaders with exact pixel formula parity.

### 3.1 Chromatic Aberration
- Parameters: `amount` (pixels), `angle` (degrees).
- Vector offset in UV space:
  $$\Delta u_x = \frac{\text{amount} \cdot \cos(\text{angle} \cdot \pi / 180)}{W}$$
  $$\Delta u_y = \frac{\text{amount} \cdot \sin(\text{angle} \cdot \pi / 180)}{H}$$
- Output RGBA:
  $$R_{\text{out}} = \text{sample}(u - \Delta u, v - \Delta u).r$$
  $$G_{\text{out}} = \text{sample}(u, v).g$$
  $$B_{\text{out}} = \text{sample}(u + \Delta u, v + \Delta u).b$$
  $$A_{\text{out}} = \text{sample}(u, v).a$$

### 3.2 Vignette
- Parameters: `amount` $\in [0, 1]$, `radius` $\in [0, 1]$, `feather` $\in [0, 1]$, `roundness` $\in [0, 1]$, `color`, `mode` (`Color` | `Alpha`), `center` $(u_c, v_c)$.
- Centered coordinates: $\vec{p} = 2 \cdot |(u, v) - (u_c, v_c)|$.
- Distance metric:
  $$d_{\text{rect}} = \max(p_x, p_y)$$
  $$d_{\text{ellipse}} = \sqrt{p_x^2 + p_y^2}$$
  $$d = (1 - \text{roundness}) \cdot d_{\text{rect}} + \text{roundness} \cdot d_{\text{ellipse}}$$
- Mask evaluation:
  $$\text{if } \text{feather} \le 10^{-4}: M = \text{if } d \ge \text{radius} \text{ then } \text{amount} \text{ else } 0.0$$
  $$\text{else}: M = \text{smoothstep}(\text{radius}, \text{radius} + \text{feather}, d) \cdot \text{amount}$$
- Blending:
  - **Alpha mode:** $A_{\text{out}} = A_{\text{src}} \cdot (1.0 - M)$, $\text{RGB}_{\text{out}} = \text{RGB}_{\text{src}} \cdot (1.0 - M)$.
  - **Color mode:**
    $$\alpha_{\text{over}} = M \cdot A_{\text{color}}$$
    $$\text{RGB}_{\text{out}} = \text{RGB}_{\text{color}} \cdot \alpha_{\text{over}} + \text{RGB}_{\text{src}} \cdot (1.0 - \alpha_{\text{over}})$$
    $$A_{\text{out}} = \alpha_{\text{over}} + A_{\text{src}} \cdot (1.0 - \alpha_{\text{over}})$$

### 3.3 Color Grading Suite

| Filter | Formula (Operating on unpremultiplied RGB in $[0, 255]$) |
|---|---|
| **Brightness** | $C' = \text{clamp}(C + \text{round}(\text{amount} \cdot 255), 0, 255)$ |
| **Contrast** | $C' = \text{clamp}((C - 128) \cdot \text{amount} + 128, 0, 255)$ |
| **Grayscale** | $L = 0.2126 R + 0.7152 G + 0.0722 B$; $C' = C + (L - C) \cdot \text{amount}$ |
| **Invert** | $C' = C + (255 - 2C) \cdot \text{amount}$ |
| **Saturation** | $L = 0.2126 R + 0.7152 G + 0.0722 B$; $C' = \text{clamp}(L + (C - L) \cdot \text{amount}, 0, 255)$ |
| **Tint** | Respects alpha mask: $\text{RGB}' = \text{mix}(\text{RGB}, \text{color}_{\text{tint}}, \text{amount})$ |
| **Duotone** | $L = 0.299 R + 0.587 G + 0.114 B$; $\text{if } L/255 < \text{threshold}: C_{\text{dark}} \text{ else } C_{\text{light}}$ |
| **ColorKey** | $d = \frac{\|\text{RGB} - \text{KeyRGB}\|}{\sqrt{3}}$; $M = \text{smoothstep}(\text{sim} - \text{smooth}, \text{sim} + \text{smooth}, d)$; Spill: limit dominant channel to average of other two |

---

## 4. Typography & Layout Fitting Engine

### 4.1 Multi-Line Auto-Scaling (`fit_text_on_n_lines`)

```rust
// Pseudocode Algorithm:
let min_font_size = 0.1;
let max_font_size = max_font_size.unwrap_or(2000.0);
let precision = 100.0;

let mut left = (min_font_size * precision).floor() as i64;
let mut right = (max_font_size * precision).floor() as i64;
let mut optimal_font_size = min_font_size;
let mut optimal_lines = Vec::new();

while left <= right {
    let mid = (left + right) / 2;
    let font_size = mid as f64 / precision;
    
    let words = text.split(' ').collect::<Vec<_>>();
    let mut lines = vec![String::new()];
    let mut current_line = 0;
    let mut exceeds_box = false;
    
    for word in words {
        let test_line = if lines[current_line].is_empty() {
            word.to_string()
        } else {
            format!("{} {}", lines[current_line], word)
        };
        
        let width = measure_text_width(&test_line, font_size);
        if width.ceil() <= max_box_width {
            lines[current_line] = test_line;
        } else {
            if current_line == max_lines - 1 {
                exceeds_box = true;
                break;
            }
            let word_alone_width = measure_text_width(word, font_size);
            if word_alone_width.ceil() > max_box_width {
                exceeds_box = true;
                break;
            }
            lines.push(word.to_string());
            current_line += 1;
        }
    }
    
    if !exceeds_box && current_line < max_lines {
        optimal_font_size = font_size;
        optimal_lines = lines;
        left = mid + 1;
    } else {
        right = mid - 1;
    }
}
```

### 4.2 Parametric Rounded Text Box (`create_rounded_text_box`)

Generates continuous smooth SVG paths bounding multi-line text blocks with custom padding, alignment, and per-corner curvature:
- Computes `maxWidth = max(line_width_i + 2 * padding)`.
- For each line $i$:
  - Alignment offset: $x_{\text{offset}} = 0$ (left), $(w_{\max} - w_i)/2$ (center), $w_{\max} - w_i$ (right).
  - Top-Right corner radius based on width difference $\Delta w = w_{i-1} - w_i$.
  - Bottom-Right corner radius based on $\Delta w = w_{i+1} - w_i$.
  - Bottom-Left corner radius based on reverse width differential.
  - Top-Left corner radius based on reverse width differential.
- Arc commands (`A rx ry 0 largeArc sweep x y`) automatically set `sweepFlag = (radius < 0)` for concave vs convex corners.

---

## 5. Edge Cases Specification Table

| # | Feature | Input Condition | Observed & Expected Behavior |
|---|---------|-----------------|------------------------------|
| 1 | `noise2D` | `seed = 1, x = 0, y = 0` | Returns exactly `0.0` |
| 2 | `noise2D` | `seed = "my-seed", x = 0.5, y = 0.5` | Returns deterministic float `0.3071565136272162` |
| 3 | `noise3D` | `seed = "my-seed", x = 0.7, y = 0.5, z = 0.5` | Returns deterministic float `0.6402128434567901` |
| 4 | `noise4D` | `seed = "my-seed", x = 0.7, y = 0.5, z = 0.5, w = 0.9` | Returns deterministic float `0.2714290963058814` |
| 5 | `random` | String seeds with unicode / emojis | Uses 32-bit `hashCode` integer overflow arithmetic identically across platforms |
| 6 | `fit_text_on_n_lines` | Unbreakable word longer than `maxBoxWidth` | Shrinks entire text size down to ensure the single long token fits on one line |
| 7 | `fit_text_on_n_lines` | Empty string `""` | Returns `max_font_size` with single empty line `[""]` |
| 8 | `fit_text` | `max_width <= 0` or non-finite inputs | Returns validation error `RasterError::Scene("parameter must be positive/finite")` |
| 9 | `create_rounded_text_box` | Single-line text box | Emits standard 4-corner rounded rectangle path |
| 10 | `create_rounded_text_box` | Stepped multi-line text (e.g. line 1 wide, line 2 narrow) | Generates inward concave arcs at step transitions with inverted sweep flags |
| 11 | `Slide` transition | Progress at boundary `0.0` or `1.0` | Epsilon subtraction omitted at `1.0` to eliminate visual boundary seams |
| 12 | `Flip` transition | Layer rotation past $90^\circ$ | Normal reverses; backface culling hides back face to prevent mirror artifacts |
| 13 | `Vignette` | `radius = 0, feather = 0` | Full coverage mask applied over the entire surface |
| 14 | `ColorKey` | Transparent source pixels ($A \le 0.001$) | Early-exits with `vec4(0.0)` to prevent division-by-zero on unpremultiplication |
| 15 | `ChromaticAberration` | `amount = 0.0` | No-op passthrough preserving original pixel buffer |

---

## 6. Pure Rust Translation Architecture Blueprint

### 6.1 `crates/noise` Pure Rust Implementation
```rust
pub struct SimplexNoise {
    perm: [u8; 512],
    perm_mod12: [u8; 512],
}

impl SimplexNoise {
    pub fn new(seed: &str) -> Self { ... }
    pub fn noise_2d(&self, x: f64, y: f64) -> f64 { ... }
    pub fn noise_3d(&self, x: f64, y: f64, z: f64) -> f64 { ... }
    pub fn noise_4d(&self, x: f64, y: f64, z: f64, w: f64) -> f64 { ... }
    pub fn fbm_2d(&self, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 { ... }
    pub fn turbulence_2d(&self, x: f64, y: f64, octaves: u32) -> f64 { ... }
}
```

### 6.2 `crates/transitions` Pure Rust Implementation
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionPresentation {
    ClockWipe { width: f64, height: f64 },
    LinearWipe { direction: WipeDirection },
    Flip { direction: FlipDirection, perspective: f64 },
    ZoomInOut,
    CrossZoom { strength: f64 },
    Slide { direction: SlideDirection },
    Fade { should_fade_out_exiting_scene: bool },
    Dissolve { line_width: f64, spread_color: Color, hot_color: Color, pow: f64, intensity: f64 },
    Iris { width: f64, height: f64 },
    Ripple { amplitude: f64, speed: f64 },
    BookFlip { direction: BookFlipDirection },
    LinearBlur { intensity: f64 },
}

pub trait TransitionTiming: Send + Sync {
    fn duration_in_frames(&self, fps: f64) -> u32;
    fn progress(&self, frame: u32, fps: f64) -> f64;
}
```

### 6.3 `crates/rasterizer` Visual Filter Extensions
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneFilter {
    Blur { sigma: f32 },
    Brightness { amount: f32 },
    Contrast { amount: f32 },
    Grayscale { amount: f32 },
    HueRotate { degrees: f32 },
    Invert { amount: f32 },
    Saturation { amount: f32 },
    Tint { color: Color, amount: f32 },
    Duotone { dark_color: Color, light_color: Color, threshold: f32 },
    ColorKey { key_color: Color, similarity: f32, smoothness: f32, spill_suppression: f32 },
    ChromaticAberration { amount: f32, angle: f32 },
    Vignette {
        amount: f32,
        radius: f32,
        feather: f32,
        roundness: f32,
        color: Color,
        mode: VignetteMode,
        center: (f32, f32),
    },
    Noise { amount: f32, seed: f32, premultiply: bool },
    NoiseDisplacement {
        center: (f32, f32),
        radius: f32,
        strength: f32,
        seed: f32,
        grain_size: f32,
        passes: u32,
        blur: f32,
        feather: f32,
        bias_direction: f32,
        bias_amount: f32,
    },
    WhiteNoise { amount: f32, seed: f32 },
    Opacity { amount: f32 },
}
```

### 6.4 `crates/rasterizer` & `crates/core` Layout Fitting Engine
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct FitTextResult {
    pub font_size: f64,
    pub lines: Vec<String>,
}

pub fn fit_text_on_n_lines(
    text: &str,
    max_lines: usize,
    max_box_width: f64,
    font_sources: &[String],
    max_font_size: Option<f64>,
) -> Result<FitTextResult, RasterError>;

pub fn create_rounded_text_box(
    text_measurements: &[Dimensions],
    text_align: TextHorizontalAlign,
    horizontal_padding: f64,
    border_radius: f64,
) -> RoundedTextBoxResult;
```
