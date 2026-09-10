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
            hw_accel,
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
                hw_accel: (*hw_accel).into(),
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
        Commands::Migrate {
            input,
            target,
            output,
        } => {
            let target_mode: dioxuscut_cli::MigrationTarget =
                target.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            let source = std::fs::read_to_string(input)
                .map_err(|e| anyhow::anyhow!("Failed to read '{input:?}': {e}"))?;

            let (code, stats) = dioxuscut_cli::transpile_remotion(&source, target_mode)
                .map_err(|e| anyhow::anyhow!("Migration failed: {e}"))?;

            if let Some(out_path) = output {
                std::fs::write(out_path, &code)
                    .map_err(|e| anyhow::anyhow!("Failed to write to '{out_path:?}': {e}"))?;
                println!("✨ Successfully migrated {input:?} -> {out_path:?}");
            } else {
                println!("{code}");
            }

            eprintln!(
                "📊 Migration summary: {} hooks, {} interpolations, {} springs, {} sequences, {} loops converted",
                stats.hooks_converted,
                stats.interpolations_converted,
                stats.springs_converted,
                stats.sequences_converted,
                stats.loops_converted
            );
        }
        Commands::Probe { path } => {
            match dioxuscut_cli::get_video_metadata(path) {
                Ok(meta) => {
                    println!("{}", serde_json::to_string_pretty(&meta)?);
                }
                Err(e) => {
                    // Try audio probe
                    match dioxuscut_cli::get_audio_metadata(path, 30.0) {
                        Ok(audio_meta) => {
                            println!("{}", serde_json::to_string_pretty(&audio_meta)?);
                        }
                        Err(_) => {
                            anyhow::bail!("Probe failed for {path:?}: {e}");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
