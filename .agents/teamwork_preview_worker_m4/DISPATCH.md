# Dispatch for Worker M4 (Milestone 4: R5 Text Measurement & Fitting API)

- Working Directory: /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_worker_m4
- Original Request: /Users/sjkim1127/Dioxuscut/.agents/ORIGINAL_REQUEST.md
- Scope Document: /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_orchestrator_1/PROJECT.md
- Survey Report: /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_survey_2/handoff.md
- Exclusive File Ownership: `crates/rasterizer/**` (No other worker will touch `crates/rasterizer`)

## Tasks to Implement
1. Ensure all public text measurement & fitting items are properly implemented, exported, and documented in `crates/rasterizer/src/lib.rs` and `crates/rasterizer/src/font.rs`:
   - `fit_text(text: &str, max_width: f64, font_sources: &[String], min_font_size: f64, max_font_size: f64) -> Result<f64, RasterError>`
   - `measure_text_width`, `layout_text_box`, `TextBox`, `TextBoxLayout`, `PositionedTextLine`, `TextHorizontalAlign`, `TextVerticalAlign`, `TextOverflow`.
2. Harden `fit_text`:
   - Add input validation for non-finite values (`!max_width.is_finite()`, `!min_font_size.is_finite()`, etc.) and invalid bounds.
   - Ensure binary search converges reliably using bundled `NotoSans-Regular.ttf` font fallback.
3. Fix any clippy warnings in `crates/rasterizer/src/font.rs` (e.g. `manual-range-contains` on line 817).
4. Implement comprehensive unit tests for `fit_text` (empty string, short text, long text, boundary font sizes, custom font sources, invalid inputs) and text measurement functions.
5. Verify with:
   - `cargo check --locked -p dioxuscut-rasterizer`
   - `cargo clippy --locked -p dioxuscut-rasterizer -- -D warnings`
   - `cargo test --locked -p dioxuscut-rasterizer`
   - `cargo fmt --all -- --check`
6. Write full verification commands, output, and status into handoff.md and send completion message.
