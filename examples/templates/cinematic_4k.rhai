// 4K Ultra-HD Cinematic Motion Graphics Template (3840x2160)
fn render(ctx, props) {
    let output = scene();
    let width = ctx.width.to_float();
    let height = ctx.height.to_float();
    let frame = ctx.frame.to_float();
    let progress = ctx.progress;

    // 1. Deep Space Dark Background
    output.rect(0.0, 0.0, width, height, "#070913");

    // 2. Animated Ambient Glow Orbs
    let orb1_x = width * 0.3 + (frame * 1.5);
    let orb1_y = height * 0.4;
    output.circle(orb1_x, orb1_y, 500.0, "#00f0ff18");

    let orb2_x = width * 0.7 - (frame * 1.2);
    let orb2_y = height * 0.6;
    output.circle(orb2_x, orb2_y, 600.0, "#ff007f18");

    // 3. Cinematic Grid Guide Lines
    for i in 0..6 {
        let y = 300.0 + i.to_float() * 300.0;
        output.rect(200.0, y, width - 400.0, 2.0, "#1e293b44");
    }

    // 4. Top Technical Header / Badge
    let badge_w = 480.0;
    let badge_h = 72.0;
    let badge_x = (width - badge_w) * 0.5;
    output.round_rect(badge_x, 380.0, badge_w, badge_h, "#ffffff10", 36.0);
    output.text_bold(badge_x + 55.0, 428.0, "NATIVE VIDEO INFRASTRUCTURE", 28.0, "#00f0ff");

    let title_str = if type_of(props.title) == "string" { props.title } else { "4K ULTRA-HD NATIVE RENDERING" };
    let subtitle_str = if type_of(props.subtitle) == "string" { props.subtitle } else { "Sub-millisecond Rust compositing engine delivering unmatched performance." };

    // 5. Main Hero Title (Multi-line Auto-fitting in 4K)
    let title_y = 650.0;
    output.text_box(
        300.0,
        title_y,
        width - 600.0,
        500.0,
        title_str,
        140.0,  // Max 4K font size
        80.0,   // Min font size
        3,
        "#ffffff",
        "",
        "center"
    );

    // 6. Subtitle & Architecture Statement Card
    let card_w = 2000.0;
    let card_h = 220.0;
    let card_x = (width - card_w) * 0.5;
    let card_y = 1350.0;
    output.round_rect(card_x, card_y, card_w, card_h, "#0f172a88", 32.0);

    output.text_box(
        card_x + 80.0,
        card_y + 60.0,
        card_w - 160.0,
        card_h - 100.0,
        subtitle_str,
        52.0,
        36.0,
        2,
        "#94a3b8",
        "",
        "center"
    );

    // 7. Bottom Technical Specs Telemetry
    let telemetry = "RESOLUTION: 3840 x 2160  |  FPS: 30.0  |  BACKEND: tiny-skia CPU  |  ZERO CHROMIUM";
    output.text(500.0, 1920.0, telemetry, 32.0, "#475569");

    // 8. Animated Cyan Progress Bar across bottom
    let bar_w = width * progress;
    output.rect(0.0, height - 24.0, bar_w, 24.0, "#00f0ff");

    output
}
