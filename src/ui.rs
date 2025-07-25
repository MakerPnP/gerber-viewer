use egui::{Pos2, Rect, Response, Ui, Vec2};
use gerber_types::Unit;
use log::trace;
use nalgebra::Point2;

use crate::geometry::BoundingBox;
use crate::{Invert, ToPos2};

#[derive(Debug, Default)]
pub struct UiState {
    // these two values are invalid until 'update' has been called
    pub center_screen_pos: Pos2,
    pub origin_screen_pos: Pos2,

    // only valid if the mouse is over the viewport
    pub cursor_gerber_coords: Option<Point2<f64>>,
}

impl UiState {
    pub fn update(&mut self, ui: &Ui, viewport: &Rect, response: &Response, view_state: &mut ViewState) {
        view_state.handle_viewport_relocation(viewport);

        self.update_cursor_position(view_state, &response, ui);
        self.handle_panning(view_state, &response, ui);
        self.handle_zooming(view_state, &response, ui);

        self.center_screen_pos = viewport.center();
        self.origin_screen_pos = view_state.gerber_to_screen_coords(Point2::new(0.0, 0.0));

        trace!(
            "update. view_state: {:?}, viewport: {:?}, cursor_gerber_coords: {:?}",
            view_state, viewport, self.cursor_gerber_coords
        )
    }

    pub fn update_cursor_position(&mut self, view_state: &ViewState, response: &Response, ui: &Ui) {
        if !response.hovered() {
            return;
        }

        if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
            self.cursor_gerber_coords = Some(view_state.screen_to_gerber_coords(pointer_pos));
        } else {
            self.cursor_gerber_coords = None;
        }
    }

    pub fn handle_panning(&mut self, view_state: &mut ViewState, response: &Response, ui: &Ui) {
        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            view_state.translation += delta;
            ui.ctx().clear_animations();
        }
    }

    pub fn handle_zooming(&mut self, view_state: &mut ViewState, response: &Response, ui: &Ui) {
        // Only process zoom if the mouse pointer is actually over the viewport
        if !response.hovered() {
            return;
        }

        let zoom_factor = 1.1;
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);

        if scroll_delta != 0.0 {
            let old_scale = view_state.scale;
            let new_scale = if scroll_delta > 0.0 {
                old_scale * zoom_factor
            } else {
                old_scale / zoom_factor
            };

            if let Some(hover_pos) = response.hover_pos() {
                let mouse_world = (hover_pos - view_state.translation) / old_scale;
                view_state.translation = hover_pos - mouse_world * new_scale;
            }

            view_state.scale = new_scale;
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ViewState {
    pub translation: Vec2,
    pub scale: f32,
    pub base_scale: f32, // Scale that represents 100% zoom

    // used to track viewport relocation so that the translation can be updated
    pub previous_viewport_pos: Option<Pos2>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            translation: Vec2::ZERO,
            scale: 1.0,
            base_scale: 1.0,
            previous_viewport_pos: None,
        }
    }
}

impl ViewState {
    /// Convert to gerber coordinates using view transformation
    pub fn screen_to_gerber_coords(&self, screen_pos: Pos2) -> Point2<f64> {
        let gerber_pos = (screen_pos - self.translation) / self.scale;
        Point2::new(gerber_pos.x as f64, gerber_pos.y as f64).invert_y()
    }

    /// Convert from gerber coordinates using view transformation
    pub fn gerber_to_screen_coords(&self, gerber_pos: Point2<f64>) -> Pos2 {
        let gerber_pos = gerber_pos.invert_y();
        (gerber_pos * self.scale as f64).to_pos2() + self.translation
    }

    /// inputs, viewport of UI area to render.
    /// bounding box of all gerber layers to render.
    /// initial zoom factor, e.g. 0.5 for 50%.
    pub fn fit_view(&mut self, viewport: Rect, bbox: &BoundingBox, initial_zoom_factor: f32) {
        let content_width = bbox.width();
        let content_height = bbox.height();

        // Calculate scale to fit the content (100% zoom)
        self.base_scale = f32::min(
            viewport.width() / (content_width as f32),
            viewport.height() / (content_height as f32),
        ) * 0.95; // 0.95 to add margin

        let scale = self.base_scale * initial_zoom_factor;

        trace!(
            "Fit view. base_scale: {:.2}, scale: {:.2}, content_width: {:.2}, content_height: {:.2}",
            self.base_scale, scale, content_width, content_height
        );
        self.scale = scale;

        self.center_view(viewport, bbox);
    }

    pub fn center_view(&mut self, viewport: Rect, bbox: &BoundingBox) {
        let center = bbox.center();

        self.translation = Vec2::new(
            viewport.center().x - (center.x as f32 * self.scale),
            viewport.center().y + (center.y as f32 * self.scale),
        );

        // enssure the viewport is not relocated this frame
        self.previous_viewport_pos = None;
    }

    pub fn handle_viewport_relocation(&mut self, viewport: &Rect) {
        let viewport_pos = viewport.min; // Top-left corner

        if let Some(previous_viewport_pos) = self.previous_viewport_pos {
            let delta = viewport_pos - previous_viewport_pos;
            if delta != egui::Vec2::ZERO {
                // The viewport moved, update translation to compensate.
                self.translation += delta;
            }
        }

        self.previous_viewport_pos = Some(viewport_pos);
    }

    pub fn zoom_level_percent(&self, units: Unit, display_info: &DisplayInfo) -> f32 {
        // Get effective pixels per inch
        let device_ppi = display_info.effective_ppi();

        // Calculate what 100% zoom should be (reference scale)
        let reference_scale = match units {
            Unit::Millimeters => device_ppi / 25.4, // Convert to pixels per mm
            Unit::Inches => device_ppi,             // pixels per inch
        };

        // Calculate zoom percentage
        let zoom_level = (self.scale / reference_scale) * 100.0;
        trace!(
            "Zoom level: {:.1}%, scale: {:.2}, reference_scale: {:.2}",
            zoom_level, self.scale, reference_scale
        );

        zoom_level
    }

    pub fn set_zoom_level_percent(&mut self, zoom_level: f32, units: Unit, display_info: &DisplayInfo) -> f32 {
        // Get effective pixels per inch
        let device_ppi = display_info.effective_ppi();

        // Calculate the reference scale for 100% zoom
        let reference_scale = match units {
            Unit::Millimeters => device_ppi / 25.4, // Convert to pixels per mm
            Unit::Inches => device_ppi,             // pixels per inch
        };

        // Set the scale based on the desired zoom percentage
        self.scale = reference_scale * (zoom_level / 100.0);
        trace!("Set zoom level to: {:.1}%, new scale: {:.2}", zoom_level, self.scale);

        // Return the actual zoom level (might be different due to rounding)
        self.zoom_level_percent(units, display_info)
    }
}

/// Struct to hold display information including DPI values
#[derive(Debug, Clone, Copy)]
pub struct DisplayInfo {
    /// DPI along the horizontal axis (pixels per inch)
    pub dpi_x: f32,
    /// DPI along the vertical axis (pixels per inch)
    pub dpi_y: f32,
    /// UI scaling factor from egui
    pub pixels_per_point: f32,
}

impl DisplayInfo {
    /// Create a new DisplayInfo with default values
    pub fn new() -> Self {
        Self {
            dpi_x: 96.0,
            dpi_y: 96.0,
            pixels_per_point: 1.0,
        }
    }

    pub fn with_dpi(self, dpi_x: f32, dpi_y: f32) -> Self {
        Self {
            dpi_x,
            dpi_y,
            ..self
        }
    }

    /// Get the average DPI
    pub fn average_dpi(&self) -> f32 {
        (self.dpi_x + self.dpi_y) / 2.0
    }

    /// Get effective pixels per inch, accounting for UI scaling
    pub fn effective_ppi(&self) -> f32 {
        self.average_dpi() * self.pixels_per_point
    }

    /// Update the DisplayInfo with current system values
    pub fn update_ppi_from_system(&mut self) {
        self.pixels_per_point = egui::Context::default().pixels_per_point();
    }

    pub fn set_dpi(&mut self, dpi_x: f32, dpi_y: f32) {
        self.dpi_x = dpi_x;
        self.dpi_y = dpi_y;
    }
}
