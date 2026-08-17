use glam::DVec2;
use winit::dpi::PhysicalSize;

use crate::text::Rect;

const RESET_TOP_MARGIN: f64 = 34.0;
const LOG2_ZOOM_LIMIT: f64 = 40.0;
const ZOOM_SENSITIVITY: f64 = 0.18;
const ZOOM_EASING_RATE: f64 = 14.0;
const ZOOM_EPSILON: f64 = 1.0e-5;
const PAN_VELOCITY_RESPONSE: f64 = 24.0;
const PAN_FRICTION: f64 = 6.5;
const PAN_STOP_SPEED: f64 = 5.0;
const MAX_PAN_SPEED: f64 = 8_000.0;

#[derive(Debug, Clone)]
pub struct Camera {
    center: DVec2,
    log2_zoom: f64,
    target_log2_zoom: f64,
    zoom_anchor_screen: DVec2,
    zoom_anchor_world: DVec2,
    pan_velocity: DVec2,
    dragging: bool,
    viewport: PhysicalSize<u32>,
    scale_factor: f64,
    bounds: Rect,
}

impl Camera {
    pub fn new(viewport: PhysicalSize<u32>, scale_factor: f64, bounds: Rect) -> Self {
        let mut camera = Self {
            center: bounds.min,
            log2_zoom: 0.0,
            target_log2_zoom: 0.0,
            zoom_anchor_screen: DVec2::ZERO,
            zoom_anchor_world: bounds.min,
            pan_velocity: DVec2::ZERO,
            dragging: false,
            viewport,
            scale_factor: valid_scale_factor(scale_factor),
            bounds,
        };
        camera.reset();
        camera
    }

    pub fn screen_scale(&self) -> f64 {
        self.scale_factor * self.log2_zoom.exp2()
    }

    pub fn ui_scale(&self) -> f64 {
        self.scale_factor
    }

    pub fn world_to_screen(&self, world: DVec2) -> DVec2 {
        (world - self.center) * self.screen_scale() + self.viewport_center()
    }

    pub fn screen_to_world(&self, screen: DVec2) -> DVec2 {
        self.center + (screen - self.viewport_center()) / self.screen_scale()
    }

    pub fn visible_world_rect(&self) -> Rect {
        let half_extent = self.viewport_extent() * 0.5 / self.screen_scale();
        Rect {
            min: self.center - half_extent,
            max: self.center + half_extent,
        }
    }

    pub fn resize(&mut self, viewport: PhysicalSize<u32>) {
        self.viewport = viewport;
        self.stop_motion();
        self.constrain();
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = valid_scale_factor(scale_factor);
        self.stop_motion();
        self.constrain();
    }

    pub fn zoom_at(&mut self, screen: DVec2, wheel_delta: f64) -> bool {
        if !screen.is_finite() || !wheel_delta.is_finite() || wheel_delta.abs() < f64::EPSILON {
            return false;
        }

        let next_zoom = (self.target_log2_zoom + wheel_delta * ZOOM_SENSITIVITY)
            .clamp(-LOG2_ZOOM_LIMIT, LOG2_ZOOM_LIMIT);
        if next_zoom == self.target_log2_zoom {
            return false;
        }

        self.zoom_anchor_screen = screen;
        self.zoom_anchor_world = self.screen_to_world(screen);
        self.target_log2_zoom = next_zoom;
        true
    }

    #[cfg(test)]
    pub fn pan_by(&mut self, screen_delta: DVec2) -> bool {
        self.pan_by_internal(screen_delta)
    }

    pub fn begin_pan(&mut self) {
        self.target_log2_zoom = self.log2_zoom;
        self.pan_velocity = DVec2::ZERO;
        self.dragging = true;
    }

    pub fn drag_by(&mut self, screen_delta: DVec2, elapsed_seconds: f64) -> bool {
        let changed = self.pan_by_internal(screen_delta);
        if screen_delta.is_finite() && elapsed_seconds.is_finite() && elapsed_seconds > 0.0 {
            let observed = (screen_delta / elapsed_seconds).clamp_length_max(MAX_PAN_SPEED);
            let blend = exponential_ease(PAN_VELOCITY_RESPONSE, elapsed_seconds);
            self.pan_velocity = self.pan_velocity.lerp(observed, blend);
        }
        changed
    }

    pub fn end_pan(&mut self, idle_seconds: f64) {
        self.dragging = false;
        if idle_seconds.is_finite() && idle_seconds > 0.0 {
            self.pan_velocity *= (-PAN_FRICTION * idle_seconds).exp();
        }
        self.stop_slow_pan();
    }

    pub fn cancel_pan(&mut self) {
        self.dragging = false;
        self.pan_velocity = DVec2::ZERO;
    }

    pub fn animate(&mut self, elapsed_seconds: f64) -> bool {
        if !elapsed_seconds.is_finite() || elapsed_seconds <= 0.0 {
            return false;
        }

        let before_center = self.center;
        let before_zoom = self.log2_zoom;

        let zoom_difference = self.target_log2_zoom - self.log2_zoom;
        if zoom_difference != 0.0 {
            if zoom_difference.abs() > ZOOM_EPSILON {
                let blend = exponential_ease(ZOOM_EASING_RATE, elapsed_seconds);
                self.log2_zoom += zoom_difference * blend;
            }
            if (self.target_log2_zoom - self.log2_zoom).abs() <= ZOOM_EPSILON {
                self.log2_zoom = self.target_log2_zoom;
            }
            self.center = self.zoom_anchor_world
                - (self.zoom_anchor_screen - self.viewport_center()) / self.screen_scale();
            self.constrain();
        }

        if !self.dragging && self.pan_velocity.length_squared() > 0.0 {
            let decay = (-PAN_FRICTION * elapsed_seconds).exp();
            let screen_delta = self.pan_velocity * ((1.0 - decay) / PAN_FRICTION);
            let before_pan = self.center;
            self.pan_by_internal(screen_delta);
            self.pan_velocity *= decay;
            if self.center.x == before_pan.x {
                self.pan_velocity.x = 0.0;
            }
            if self.center.y == before_pan.y {
                self.pan_velocity.y = 0.0;
            }
            self.stop_slow_pan();
        }

        self.center != before_center || self.log2_zoom != before_zoom
    }

    pub fn is_animating(&self) -> bool {
        (self.target_log2_zoom - self.log2_zoom).abs() > ZOOM_EPSILON
            || (!self.dragging && self.pan_velocity.length() >= PAN_STOP_SPEED)
    }

    fn pan_by_internal(&mut self, screen_delta: DVec2) -> bool {
        if !screen_delta.is_finite() {
            return false;
        }
        let before = self.center;
        self.center -= screen_delta / self.screen_scale();
        self.constrain();
        if (self.target_log2_zoom - self.log2_zoom).abs() > ZOOM_EPSILON {
            self.zoom_anchor_world = self.screen_to_world(self.zoom_anchor_screen);
        }
        self.center != before
    }

    pub fn reset(&mut self) {
        self.log2_zoom = 0.0;
        self.target_log2_zoom = 0.0;
        self.pan_velocity = DVec2::ZERO;
        self.dragging = false;
        let visible_size = self.viewport_extent() / self.screen_scale();
        self.center.x = (self.bounds.min.x + self.bounds.max.x) * 0.5;
        self.center.y = if self.bounds.size().y > visible_size.y {
            self.bounds.min.y + visible_size.y * 0.5 - RESET_TOP_MARGIN
        } else {
            (self.bounds.min.y + self.bounds.max.y) * 0.5
        };
        self.constrain();
    }

    fn stop_motion(&mut self) {
        self.target_log2_zoom = self.log2_zoom;
        self.pan_velocity = DVec2::ZERO;
        self.dragging = false;
    }

    fn stop_slow_pan(&mut self) {
        if self.pan_velocity.length() < PAN_STOP_SPEED {
            self.pan_velocity = DVec2::ZERO;
        }
    }

    fn constrain(&mut self) {
        let visible_size = self.viewport_extent() / self.screen_scale();
        let world_margin = RESET_TOP_MARGIN * self.scale_factor / self.screen_scale();
        self.center.x = constrain_axis(
            self.center.x,
            self.bounds.min.x,
            self.bounds.max.x,
            visible_size.x,
            world_margin,
        );
        self.center.y = constrain_axis(
            self.center.y,
            self.bounds.min.y,
            self.bounds.max.y,
            visible_size.y,
            world_margin,
        );
    }

    fn viewport_center(&self) -> DVec2 {
        self.viewport_extent() * 0.5
    }

    fn viewport_extent(&self) -> DVec2 {
        DVec2::new(self.viewport.width as f64, self.viewport.height as f64)
    }
}

fn exponential_ease(rate: f64, elapsed_seconds: f64) -> f64 {
    1.0 - (-rate * elapsed_seconds).exp()
}

fn constrain_axis(center: f64, min: f64, max: f64, visible_size: f64, margin: f64) -> f64 {
    let content_size = max - min;
    if content_size <= visible_size {
        return (min + max) * 0.5;
    }

    let half_visible = visible_size * 0.5;
    center.clamp(min + half_visible - margin, max - half_visible + margin)
}

fn valid_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: f64, height: f64) -> Rect {
        Rect {
            min: DVec2::ZERO,
            max: DVec2::new(width, height),
        }
    }

    fn assert_near(actual: DVec2, expected: DVec2) {
        assert!(
            (actual - expected).length() < 1.0e-8,
            "{actual:?} != {expected:?}"
        );
    }

    fn animate_until_idle(camera: &mut Camera) {
        for _ in 0..1_000 {
            if !camera.is_animating() {
                return;
            }
            camera.animate(1.0 / 120.0);
        }
        panic!("camera animation did not settle");
    }

    #[test]
    fn zoom_keeps_world_point_beneath_cursor() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(4000.0, 3000.0));
        let cursor = DVec2::new(220.0, 170.0);
        let anchor = camera.screen_to_world(cursor);

        assert!(camera.zoom_at(cursor, 2.5));
        assert!(camera.is_animating());
        camera.animate(1.0 / 60.0);
        assert_near(camera.screen_to_world(cursor), anchor);
        animate_until_idle(&mut camera);

        assert_near(camera.screen_to_world(cursor), anchor);
    }

    #[test]
    fn wheel_zoom_eases_toward_its_target() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(4000.0, 3000.0));
        let initial_scale = camera.screen_scale();
        let target_scale = initial_scale * (4.0 * ZOOM_SENSITIVITY).exp2();

        camera.zoom_at(DVec2::new(400.0, 300.0), 4.0);
        assert_eq!(camera.screen_scale(), initial_scale);
        camera.animate(1.0 / 60.0);
        assert!(camera.screen_scale() > initial_scale);
        assert!(camera.screen_scale() < target_scale);

        animate_until_idle(&mut camera);
        assert!((camera.screen_scale() - target_scale).abs() < 1.0e-8);
    }

    #[test]
    fn content_that_fits_stays_centered() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(100.0, 80.0));

        assert!(!camera.pan_by(DVec2::new(300.0, -200.0)));
        assert_near(
            camera.world_to_screen(DVec2::new(50.0, 40.0)),
            DVec2::new(400.0, 300.0),
        );
    }

    #[test]
    fn panning_is_clamped_to_page_edges() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(2000.0, 1800.0));

        camera.pan_by(DVec2::splat(1.0e9));
        let visible = camera.visible_world_rect();
        assert!((visible.min.x + RESET_TOP_MARGIN).abs() < 1.0e-8);
        assert!((visible.min.y + RESET_TOP_MARGIN).abs() < 1.0e-8);

        camera.pan_by(DVec2::splat(-2.0e9));
        let visible = camera.visible_world_rect();
        assert!((visible.max.x - 2000.0 - RESET_TOP_MARGIN).abs() < 1.0e-8);
        assert!((visible.max.y - 1800.0 - RESET_TOP_MARGIN).abs() < 1.0e-8);
    }

    #[test]
    fn released_drag_coasts_and_stops_under_friction() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(4000.0, 3000.0));
        camera.begin_pan();
        camera.drag_by(DVec2::new(80.0, 0.0), 1.0 / 60.0);
        let center_at_release = camera.center;
        camera.end_pan(0.0);

        assert!(camera.is_animating());
        camera.animate(1.0 / 60.0);
        assert!(camera.center.x < center_at_release.x);

        animate_until_idle(&mut camera);
        assert!(!camera.is_animating());
        assert_eq!(camera.pan_velocity, DVec2::ZERO);
    }

    #[test]
    fn edge_margin_stays_screen_sized_while_zooming() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.5, rect(2000.0, 1800.0));
        camera.zoom_at(DVec2::new(400.0, 300.0), 12.0);
        animate_until_idle(&mut camera);
        camera.pan_by(DVec2::splat(1.0e9));

        let page_origin = camera.world_to_screen(DVec2::ZERO);
        assert!((page_origin.x - RESET_TOP_MARGIN * 1.5).abs() < 1.0e-7);
        assert!((page_origin.y - RESET_TOP_MARGIN * 1.5).abs() < 1.0e-7);
    }

    #[test]
    fn reset_shows_page_with_top_margin() {
        let camera = Camera::new(PhysicalSize::new(800, 600), 2.0, rect(1080.0, 1600.0));

        assert!((camera.world_to_screen(DVec2::ZERO).y - 68.0).abs() < 1.0e-8);
    }

    #[test]
    fn extreme_wheel_input_remains_finite() {
        let mut camera = Camera::new(PhysicalSize::new(800, 600), 1.0, rect(2000.0, 1800.0));
        let cursor = DVec2::new(400.0, 300.0);

        camera.zoom_at(cursor, 1.0e12);
        animate_until_idle(&mut camera);
        assert!(camera.screen_scale().is_finite());
        camera.zoom_at(cursor, -2.0e12);
        animate_until_idle(&mut camera);
        assert!(camera.screen_scale().is_finite());
        assert!(camera.visible_world_rect().min.is_finite());
    }
}
