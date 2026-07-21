use clap::{Parser, Subcommand};
use dioxuscut_renderer::{render_frames, RenderConfig, encode_frames, EncodeConfig};
use std::path::PathBuf;
use std::fs;

/// Dioxuscut CLI — render videos from code
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render a composition to a video file
    Render {
        /// Name of the composition to render (currently ignored, but useful for router mapping)
        #[arg(long, short)]
        composition: String,

        /// Path to a JSON file containing the input props
        #[arg(long, short)]
        props: Option<PathBuf>,

        /// Output video file path
        #[arg(long, short, default_value = "out.mp4")]
        output: PathBuf,

        /// Resolution width
        #[arg(long, default_value_t = 1920)]
        width: u32,

        /// Resolution height
        #[arg(long, default_value_t = 1080)]
        height: u32,

        /// Frames per second
        #[arg(long, default_value_t = 30.0)]
        fps: f64,

        /// Duration in frames
        #[arg(long, default_value_t = 150)]
        duration: u32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,dioxuscut_renderer=debug")
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
        } => {
            tracing::info!("Starting render for composition '{}'", composition);

            // 1. Read props JSON
            let props_json = if let Some(p) = props {
                fs::read_to_string(p)?
            } else {
                "{}".to_string()
            };

            // 2. Set environment variable so the child Dioxus process can read it
            // (In a real implementation, we would spawn a server/child process here)
            std::env::set_var("DIOXUSCUT_PROPS", &props_json);

            // For this test, we assume the Dioxus app is running on localhost:8080
            // In a real CLI, we would spawn `dx serve` on an ephemeral port here.
            let url = "http://localhost:8080".to_string();
            
            let out_dir = std::env::temp_dir().join("dioxuscut_render_frames");
            if out_dir.exists() {
                fs::remove_dir_all(&out_dir)?;
            }
            
            let render_cfg = RenderConfig::new(
                url, 
                &out_dir, 
                *width, 
                *height, 
                *fps, 
                *duration
            );

            // 4. Render frames (will invoke Headless Chrome in the renderer crate)
            render_frames(&render_cfg).await?;

            // 5. Encode frames to video
            let encode_cfg = EncodeConfig::h264(&out_dir, output, *fps);
            encode_frames(&encode_cfg).await?;

            tracing::info!("Successfully rendered video to {}", output.display());
            
            // Cleanup
            if out_dir.exists() {
                let _ = fs::remove_dir_all(&out_dir);
            }
        }
    }

    Ok(())
}
