// Bar Chart Race Data Visualization Template
// Props:
// - title: Main chart title string
// - bg: Background color (hex)
fn render(ctx, props) {
    let output = scene();
    let width = ctx.width.to_float();
    let height = ctx.height.to_float();
    let t = ctx.progress;

    let bg = if props.bg != () { props.bg } else { "#0d1117" };
    let title = if props.title != () { props.title } else { "Programming Language Popularity Index" };

    // 1. Dashboard dark background
    output.rect(0.0, 0.0, width, height, bg);

    // 2. Large timeline year counter in background (e.g. 2020 -> 2026)
    let year = 2020 + (t * 6.0).to_int();
    let year_font = if width < height { width * 0.16 } else { height * 0.22 };
    output.text_bold(width * 0.62, height * 0.78, "" + year, year_font, "#ffffff10");

    // 3. Header title
    let title_font = if width < height { width * 0.045 } else { width * 0.028 };
    output.text_bold(width * 0.08, height * 0.12, title, title_font, "#ffffff");

    // 4. Data items with interpolated growth
    // Rust: 20 -> 98
    // Python: 65 -> 88
    // TypeScript: 50 -> 78
    // Go: 35 -> 68
    let rust_val = 20.0 + (98.0 - 20.0) * t;
    let py_val = 65.0 + (88.0 - 65.0) * t;
    let ts_val = 50.0 + (78.0 - 50.0) * t;
    let go_val = 35.0 + (68.0 - 35.0) * t;

    let items = [
        ["Rust", rust_val, "#00f0ff"],
        ["Python", py_val, "#ffd600"],
        ["TypeScript", ts_val, "#3178c6"],
        ["Go", go_val, "#00add8"]
    ];

    // Simple bubble sort in Rhai to rank items descending by value
    for i in 0..4 {
        for j in 0..3 {
            let val_a = items[j][1];
            let val_b = items[j + 1][1];
            if val_a < val_b {
                let temp = items[j];
                items[j] = items[j + 1];
                items[j + 1] = temp;
            }
        }
    }

    // 5. Grid lines in background
    let chart_left = width * 0.22;
    let max_bar_w = width * 0.60;
    let chart_top = height * 0.24;
    let bar_h = (height * 0.085).clamp(28.0, 56.0);
    let gap = (height * 0.04).clamp(14.0, 28.0);

    for g in 0..5 {
        let gx = chart_left + g.to_float() * 0.25 * max_bar_w;
        output.rect(gx, chart_top - 10.0, 1.0, 4.0 * (bar_h + gap), "#ffffff15");
    }

    // 6. Render ranked bars
    for rank in 0..4 {
        let item = items[rank];
        let name = item[0];
        let val = item[1];
        let col = item[2];

        let y = chart_top + rank.to_float() * (bar_h + gap);
        let bar_w = (val / 100.0) * max_bar_w;

        // Label (left side)
        output.text_bold(width * 0.08, y + bar_h * 0.65, name, bar_h * 0.45, "#e2e8f0");

        // The animated bar
        output.round_rect(chart_left, y, bar_w, bar_h, col, bar_h * 0.2);

        // Value text (e.g. "85k")
        let val_int = val.to_int();
        output.text_bold(chart_left + bar_w + 12.0, y + bar_h * 0.65, "" + val_int + "k", bar_h * 0.42, col);
    }

    // 7. Progress bar
    output.rect(0.0, height - 6.0, width * t, 6.0, "#00f0ff");

    output
}
