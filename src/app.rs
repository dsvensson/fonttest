use std::sync::Arc;

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

use crate::{
    Args,
    renderer::{FrameError, Renderer},
};

pub struct App {
    args: Args,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
}

impl App {
    pub fn new(args: Args) -> Self {
        Self {
            args,
            window: None,
            renderer: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        event_loop.set_control_flow(ControlFlow::Wait);
        let attributes = Window::default_attributes()
            .with_title(format!("MSDF Font Explorer — {}", self.args.font))
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_visible(false);

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("failed to create window: {error:#}");
                event_loop.exit();
                return;
            }
        };

        match pollster::block_on(Renderer::new(window.clone(), &self.args.font)) {
            Ok(renderer) => {
                window.set_visible(true);
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(error) => {
                log::error!("failed to initialize renderer: {error:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render() {
                    Ok(()) => {}
                    Err(FrameError::Lost | FrameError::Outdated) => {
                        renderer.reconfigure();
                        window.request_redraw();
                    }
                    Err(FrameError::Timeout) => {
                        log::warn!("surface frame timed out");
                        window.request_redraw();
                    }
                    Err(FrameError::Occluded) => {}
                    Err(FrameError::Validation) => {
                        log::error!("surface frame acquisition failed validation");
                        window.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }
}
