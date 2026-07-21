use clap::Parser;
use dioxuscut_cli::{
    default_render_control, execute_render_command_with_control, Cli, Commands, RenderRequest,
};

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
            audio,
            width,
            height,
            fps,
            duration,
            backend,
            codec,
            frame_start,
            frame_end,
            timeout_seconds,
            crf,
            preset,
        } => {
            let request = RenderRequest {
                composition: composition.clone(),
                script: script.clone(),
                props: props.clone(),
                output: output.clone(),
                audio: audio.clone(),
                width: *width,
                height: *height,
                fps: *fps,
                duration: *duration,
                backend: *backend,
                codec: *codec,
                frame_start: *frame_start,
                frame_end: *frame_end,
                timeout_seconds: *timeout_seconds,
                crf: *crf,
                preset: preset.clone(),
            };
            let control = default_render_control(&request);
            let cancellation = control.cancellation_token();
            let signal_task = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    tracing::warn!("Cancellation requested; stopping render");
                    cancellation.cancel();
                }
            });
            let result = execute_render_command_with_control(&request, control).await;
            signal_task.abort();
            result?;
        }
    }

    Ok(())
}
