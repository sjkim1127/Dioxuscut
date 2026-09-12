use clap::Parser;
use dioxuscut_cli::{
    default_render_control, execute_render_command_with_control, serve::ServeConfig, Cli, Commands,
    RenderRequest,
};

fn project_codec(output: &std::path::Path) -> anyhow::Result<dioxuscut_cli::RenderCodec> {
    match output.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => Ok(dioxuscut_cli::RenderCodec::Png),
        "jpg" | "jpeg" => Ok(dioxuscut_cli::RenderCodec::Jpeg),
        "webp" => Ok(dioxuscut_cli::RenderCodec::Webp),
        "mp4" => Ok(dioxuscut_cli::RenderCodec::H264),
        "webm" => Ok(dioxuscut_cli::RenderCodec::Vp9),
        "mov" => Ok(dioxuscut_cli::RenderCodec::ProRes),
        "gif" => Ok(dioxuscut_cli::RenderCodec::Gif),
        _ => anyhow::bail!("Cannot infer project codec from output extension; use .mp4, .webm, .mov, .gif, .png, .jpg, or .webp"),
    }
}

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
            frame_step,
            timeout_seconds,
            crf,
            preset,
            hw_accel,
            sandbox_roots,
            permissive,
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
                frame_step: *frame_step,
                timeout_seconds: *timeout_seconds,
                crf: *crf,
                preset: preset.clone(),
                hw_accel: (*hw_accel).into(),
                sandbox_roots: sandbox_roots.clone(),
                permissive: *permissive,
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
            if let Err(error) = result {
                if std::env::var_os("DIOXUSCUT_JSON").is_some() {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": false,
                            "error": error.to_string(),
                            "output": request.output,
                            "backend": format!("{:?}", request.backend).to_ascii_lowercase(),
                            "codec": format!("{:?}", request.codec).to_ascii_lowercase(),
                        })
                    );
                }
                return Err(error);
            }
        }
        Commands::Migrate {
            input,
            target,
            output,
            experimental: _,
        } => {
            eprintln!(
                "⚠️  [EXPERIMENTAL] dioxuscut migrate generates a starting scaffold template."
            );
            eprintln!(
                "    Complex TypeScript logic and full CSS layouts require manual adaptation.\n"
            );

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
        Commands::ListCompositions => {
            let compositions = dioxuscut_cli::built_in_registry()
                .ids()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if std::env::var_os("DIOXUSCUT_JSON").is_some() {
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "compositions": compositions})
                );
            } else {
                for composition in compositions {
                    println!("{composition}");
                }
            }
        }
        Commands::ValidateProject { input } => {
            let project = dioxuscut_project::Project::load(input)
                .map_err(|error| anyhow::anyhow!("Project validation failed: {error}"))?;
            println!("{}", serde_json::to_string_pretty(&project)?);
        }
        Commands::RenderProject { input, output } => {
            let project = dioxuscut_project::Project::load(input)
                .map_err(|error| anyhow::anyhow!("Project validation failed: {error}"))?;
            let props_path = std::env::temp_dir().join(format!(
                "dioxuscut-project-props-{}.json",
                std::process::id()
            ));
            std::fs::write(&props_path, serde_json::to_vec(&project.props)?)?;
            let request = RenderRequest {
                composition: Some(project.composition.clone()),
                script: None,
                props: Some(props_path.clone()),
                output: output.clone(),
                audio: vec![],
                width: project.settings.width,
                height: project.settings.height,
                fps: project.settings.fps,
                duration: project.settings.duration,
                backend: match project.settings.backend {
                    dioxuscut_project::BackendKind::Native => dioxuscut_cli::RenderBackend::Native,
                    dioxuscut_project::BackendKind::Browser => {
                        dioxuscut_cli::RenderBackend::Browser
                    }
                    dioxuscut_project::BackendKind::Gpu => dioxuscut_cli::RenderBackend::Gpu,
                },
                codec: project_codec(output)?,
                frame_start: project.settings.frame_start.unwrap_or(0),
                frame_end: project.settings.frame_end,
                frame_step: 1,
                timeout_seconds: None,
                crf: 18,
                preset: "fast".into(),
                hw_accel: dioxuscut_rasterizer::HwAccel::Auto,
                sandbox_roots: vec![],
                permissive: true,
            };
            let previous_browser_assets = std::env::var_os("DIOXUSCUT_BROWSER_ASSETS");
            let previous_browser_timeline = std::env::var_os("DIOXUSCUT_BROWSER_TIMELINE");
            if request.backend == dioxuscut_cli::RenderBackend::Browser {
                let asset_separator = if cfg!(windows) { ';' } else { ':' };
                std::env::set_var(
                    "DIOXUSCUT_BROWSER_ASSETS",
                    project
                        .assets
                        .iter()
                        .map(|asset| asset.path.as_str())
                        .collect::<Vec<_>>()
                        .join(&asset_separator.to_string()),
                );
                let timeline = project
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter())
                    .map(|clip| dioxuscut_rasterizer::WebTimelineClip {
                        id: clip.id.clone(),
                        composition: clip.composition.clone(),
                        start: clip.start,
                        duration: clip.duration,
                        props: clip.props.clone(),
                    })
                    .collect::<Vec<_>>();
                std::env::set_var(
                    "DIOXUSCUT_BROWSER_TIMELINE",
                    serde_json::to_string(&timeline)?,
                );
            }
            let result = dioxuscut_cli::execute_project_render_command_with_control(
                &request,
                &project,
                dioxuscut_cli::default_render_control(&request),
            )
            .await;
            match previous_browser_assets {
                Some(value) => std::env::set_var("DIOXUSCUT_BROWSER_ASSETS", value),
                None => std::env::remove_var("DIOXUSCUT_BROWSER_ASSETS"),
            }
            match previous_browser_timeline {
                Some(value) => std::env::set_var("DIOXUSCUT_BROWSER_TIMELINE", value),
                None => std::env::remove_var("DIOXUSCUT_BROWSER_TIMELINE"),
            }
            let _ = std::fs::remove_file(props_path);
            result?;
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
        Commands::Serve {
            script,
            props,
            port,
            frame,
            width,
            height,
            fps,
            duration,
        } => {
            dioxuscut_cli::serve::run(ServeConfig {
                script: script.clone(),
                props: props.clone(),
                port: *port,
                default_frame: *frame,
                width: *width,
                height: *height,
                fps: *fps,
                duration: *duration,
            })
            .await?;
        }
    }

    Ok(())
}
