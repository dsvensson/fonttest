mod app;
mod atlas;
mod camera;
mod font;
mod gpu_text;
mod renderer;
mod text;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use app::{App, AppEvent};
use clap::Parser;
use winit::event_loop::EventLoop;

#[derive(Debug, Clone, Parser)]
#[command(about = "Interactive wgpu MSDF typography sample")]
pub struct Args {
    /// Installed/Google font family or path to a TTF/OTF/TTC file.
    #[arg(long, default_value = "Playfair Display")]
    pub font: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .context("creating the window event loop")?;
    let mut app = App::new(args);
    event_loop
        .run_app(&mut app)
        .context("running the window event loop")
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn run_web() -> std::result::Result<(), wasm_bindgen::JsValue> {
    use winit::platform::web::EventLoopExtWebSys;

    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    let args = web_args();
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    let proxy = event_loop.create_proxy();
    let app = App::new(args, proxy);
    event_loop.spawn_app(app);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn web_args() -> Args {
    let font = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|parameters| parameters.get("font"))
        .filter(|font| !font.trim().is_empty())
        .unwrap_or_else(|| "Playfair Display".to_owned());
    Args { font }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_playfair_display() {
        let args = Args::try_parse_from(["fonttest"]).expect("default arguments should parse");
        assert_eq!(args.font, "Playfair Display");
    }
}
