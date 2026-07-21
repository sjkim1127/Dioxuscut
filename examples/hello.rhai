fn render(ctx, props) {
    let output = scene();
    let width = ctx.width.to_float();
    let height = ctx.height.to_float();

    output.rect(0.0, 0.0, width, height, props.background);

    let content = scene();
    let x = interpolate(
        ctx.frame.to_float(),
        0.0,
        (ctx.duration - 1).to_float(),
        -120.0,
        width * 0.12
    );
    content.circle(width * 0.5, height * 0.5, 96.0, props.accent);
    content.text_bold(x, height * 0.48, props.title, 56.0, "#ffffff");
    content.text(x, height * 0.58, props.subtitle, 24.0, "#9ee7ff");

    output.group(content, 0.0, 0.0, 1.0, ctx.progress * 4.0, 1.0);
    output
}
