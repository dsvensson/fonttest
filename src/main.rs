mod app;
mod renderer;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use winit::event_loop::EventLoop;

#[derive(Debug, Clone, Parser)]
#[command(about = "Interactive wgpu MSDF typography sample")]
pub struct Args {
    /// Installed font family or path to a TTF/OTF/TTC file.
    #[arg(long, default_value = "Arial")]
    pub font: String,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let event_loop = EventLoop::new().context("creating the window event loop")?;
    let mut app = App::new(args);
    event_loop
        .run_app(&mut app)
        .context("running the window event loop")
}
