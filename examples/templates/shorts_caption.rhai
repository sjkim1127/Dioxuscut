// Vertical 9:16 Shorts / Reels Template
// Supports dynamic headline text fitting, badge, quote box, and animated progress bar.
fn render(ctx, props) {
    let output = scene();
    let width = ctx.width.to_float();
    let height = ctx.height.to_float();

    // 1. Dark Background
    let bg_color = if props.background != () { props.background } else { "#0b0d19" };
    output.rect(0.0, 0.0, width, height, bg_color);

    // 2. Glowing accent circle in background
    let accent = if props.accent != () { props.accent } else { "#ff007f33" };
    output.circle(width * 0.5, height * 0.45, 320.0, accent);

    // 3. Category Badge (Top)
    let badge_text = if props.tag != () { props.tag } else { "AI INSIGHT" };
    let badge_w = 260.0;
    let badge_h = 56.0;
    let badge_x = (width - badge_w) * 0.5;
    output.round_rect(badge_x, 240.0, badge_w, badge_h, "#ffffff15", 28.0);
    output.text_bold(badge_x + 35.0, 276.0, badge_text, 24.0, "#00f0ff");

    // 4. Main Headline with Auto-Fitting text box
    let title = if props.headline != () { props.headline } else { "Future of Autonomous Video" };
    let title_color = if props.text_color != () { props.text_color } else { "#ffffff" };
    output.text_box(
        80.0,              // x
        420.0,             // y
        width - 160.0,     // width
        500.0,             // height
        title,             // content
        72.0,              // max font size
        40.0,              // min font size
        5,                 // max lines
        title_color,       // color
        "",                // font source
        "center"           // align
    );

    // 5. Highlight Subtitle / Caption Card
    let caption = if props.caption != () { props.caption } else { "Created natively in Rust without Chrome." };
    let card_x = 80.0;
    let card_y = 1100.0;
    let card_w = width - 160.0;
    let card_h = 240.0;
    output.round_rect(card_x, card_y, card_w, card_h, "#1e293b88", 20.0);
    output.text_box(
        card_x + 40.0,
        card_y + 40.0,
        card_w - 80.0,
        card_h - 80.0,
        caption,
        36.0,
        24.0,
        3,
        "#94a3b8",
        "",
        "center"
    );

    // 6. Bottom animated progress bar
    let progress_w = width * ctx.progress;
    output.rect(0.0, height - 16.0, progress_w, 16.0, "#00f0ff");

    output
}
