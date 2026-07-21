use clap::Parser;
use dioxuscut_cli::{execute_render_command, Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,dioxuscut_renderer=debug,dioxuscut_rasterizer=debug")
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Render {
            composition,
            props,
            output,
            width,
            height,
            fps,
            duration,
            backend,
            port,
            web_dir,
            server_url,
        } => {
            execute_render_command(
                composition,
                props.as_ref(),
                output,
                *width,
                *height,
                *fps,
                *duration,
                *backend,
                *port,
                web_dir.as_ref(),
                server_url.clone(),
            )
            .await?;
        }
    }

    Ok(())
}
