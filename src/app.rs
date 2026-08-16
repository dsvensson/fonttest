use std::sync::Arc;

use glam::DVec2;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorIcon, Window, WindowId},
};

#[cfg(target_arch = "wasm32")]
use winit::event_loop::EventLoopProxy;

use crate::{
    Args,
    renderer::{FrameError, Renderer},
};

pub struct App {
    args: Args,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    cursor_position: Option<PhysicalPosition<f64>>,
    dragging: bool,
    #[cfg(target_arch = "wasm32")]
    proxy: EventLoopProxy<AppEvent>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum AppEvent {
    RendererReady(Result<Renderer, String>),
}

impl App {
    pub fn new(args: Args, #[cfg(target_arch = "wasm32")] proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            args,
            window: None,
            renderer: None,
            cursor_position: None,
            dragging: false,
            #[cfg(target_arch = "wasm32")]
            proxy,
        }
    }

    fn finish_renderer(&mut self, event_loop: &ActiveEventLoop, result: Result<Renderer, String>) {
        match result {
            Ok(mut renderer) => {
                let Some(window) = self.window.as_ref() else {
                    return;
                };
                let size = window.inner_size();
                renderer.resize(size);
                renderer.set_scale_factor(window.scale_factor());
                window.set_cursor(CursorIcon::Grab);
                window.set_visible(true);
                window.request_redraw();
                self.renderer = Some(renderer);
                log::info!("renderer ready at {}×{}", size.width, size.height);
            }
            Err(error) => {
                log::error!("failed to initialize renderer: {error}");
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
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
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_append(true)
        };

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("failed to create window: {error:#}");
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window.clone());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let result = pollster::block_on(Renderer::new(window, &self.args.font))
                .map_err(|error| format!("{error:#}"));
            self.finish_renderer(event_loop, result);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let proxy = self.proxy.clone();
            let font = self.args.font.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = Renderer::new(window, &font)
                    .await
                    .map_err(|error| format!("{error:#}"));
                let _ = proxy.send_event(AppEvent::RendererReady(result));
            });
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::RendererReady(result) => self.finish_renderer(event_loop, result),
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
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.set_scale_factor(scale_factor);
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let (Some(previous), Some(renderer)) =
                        (self.cursor_position, self.renderer.as_mut())
                {
                    renderer.pan_by(DVec2::new(position.x - previous.x, position.y - previous.y));
                    window.request_redraw();
                }
                self.cursor_position = Some(position);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                window.set_cursor(if self.dragging {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let wheel_delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(position) => position.y / 120.0,
                };
                let size = window.inner_size();
                let cursor = self.cursor_position.unwrap_or(PhysicalPosition::new(
                    size.width as f64 * 0.5,
                    size.height as f64 * 0.5,
                ));
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.zoom_at(DVec2::new(cursor.x, cursor.y), wheel_delta);
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(
                        event.physical_key,
                        PhysicalKey::Code(KeyCode::Digit0 | KeyCode::Numpad0)
                    ) =>
            {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.reset_view();
                    window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => {
                self.dragging = false;
                self.cursor_position = None;
                window.set_cursor(CursorIcon::Grab);
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
