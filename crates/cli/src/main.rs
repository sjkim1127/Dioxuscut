use clap::Parser;
use dioxuscut_cli::{execute_render_command, Cli, Commands, RenderRequest};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,dioxuscut_renderer=debug,dioxuscut_rasterizer=debug")
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Render {
            composition,
            script,
            props,
            output,
            width,
            height,
            fps,
            duration,
            backend,
        } => {
            let request = RenderRequest {
                composition: composition.clone(),
                script: script.clone(),
                props: props.clone(),
                output: output.clone(),
                width: *width,
                height: *height,
                fps: *fps,
                duration: *duration,
                backend: *backend,
            };
            execute_render_command(&request).await?;
        }
    }

    Ok(())
}
