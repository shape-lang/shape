//! Core event types and simple state container for interactive charting
//!
//! This module intentionally keeps the API lightweight so it can be used by
//! *any* frontend (terminal, native window, etc.) while remaining
//! completely platform-agnostic.

/// High-level user-interaction events that the chart core understands.
#[derive(Debug, Clone)]
pub enum ChartEvent {
    /// Pan by screen-space pixel delta.
    Pan { dx: f32, dy: f32 },
    /// Zoom by factor around a screen-space centre.
    Zoom {
        factor: f32,
        center_x: f32,
        center_y: f32,
    },
    /// Mouse move in screen-space.
    MouseMove { x: f32, y: f32 },
    /// Resize the output surface.
    Resize { width: u32, height: u32 },
    /// Signal that data has been updated (frontend manages the actual data).
    DataUpdated,
}

/// Minimal mutable state shared between render frames in interactive mode.
/// The goal is *not* to be a full chart implementation – the existing `Chart`
/// struct already handles that.  Instead this type acts as a convenient
/// scratch-pad that front-ends can own and mutate while delegating heavy work
/// to `Chart`.
///
/// Note: Data management is the responsibility of the frontend. This state
/// only tracks interaction state (pan, zoom, etc.).
#[derive(Clone)]
pub struct ChartState {
    /// Pending pan.
    pan_dx: f32,
    pan_dy: f32,
    /// Pending zoom.
    zoom_factor: f32,
    zoom_center_x: f32,
    zoom_center_y: f32,
    /// Dirty flag so callers know when to re-render.
    dirty: bool,
}

impl Default for ChartState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartState {
    pub fn new() -> Self {
        Self {
            pan_dx: 0.0,
            pan_dy: 0.0,
            zoom_factor: 1.0,
            zoom_center_x: 0.0,
            zoom_center_y: 0.0,
            dirty: true,
        }
    }

    /// Front-ends call this whenever an interaction happens.
    pub fn handle_event(&mut self, ev: ChartEvent) {
        match ev {
            ChartEvent::Pan { dx, dy } => {
                self.pan_dx += dx;
                self.pan_dy += dy;
                self.dirty = true;
            }
            ChartEvent::Zoom {
                factor,
                center_x,
                center_y,
            } => {
                self.zoom_factor *= factor;
                self.zoom_center_x = center_x;
                self.zoom_center_y = center_y;
                self.dirty = true;
            }
            ChartEvent::MouseMove { .. } => {
                // For now mouse move does not mutate state but callers might
                // still want to redraw cross-hair etc.
                self.dirty = true;
            }
            ChartEvent::Resize { .. } => {
                self.dirty = true; // External code will recreate renderer.
            }
            ChartEvent::DataUpdated => {
                self.dirty = true;
            }
        }
    }

    /// True if something changed since the last time the flag was queried.
    pub fn check_needs_redraw(&mut self) -> bool {
        let d = self.dirty;
        self.dirty = false;
        d
    }

    /// Get accumulated pan delta and reset it.
    pub fn take_pan(&mut self) -> (f32, f32) {
        let pan = (self.pan_dx, self.pan_dy);
        self.pan_dx = 0.0;
        self.pan_dy = 0.0;
        pan
    }

    /// Get zoom state.
    pub fn zoom(&self) -> (f32, f32, f32) {
        (self.zoom_factor, self.zoom_center_x, self.zoom_center_y)
    }

    /// Reset zoom factor to 1.0.
    pub fn reset_zoom(&mut self) {
        self.zoom_factor = 1.0;
    }
}
