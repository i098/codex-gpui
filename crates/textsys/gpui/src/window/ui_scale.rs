use super::*;

impl Window {
    /// Returns the UI magnification, independently of the display's pixel density.
    pub fn ui_scale(&self) -> f32 {
        self.ui_scale
    }

    /// Scales layout, painting, and platform input together, without resizing the native window.
    /// Values outside the supported 50%–300% range leave the current scale unchanged.
    pub fn set_ui_scale(&mut self, scale: f32) -> bool {
        if !scale.is_finite() || !(0.5..=3.).contains(&scale) || self.ui_scale == scale {
            return false;
        }
        self.ui_scale = scale;
        self.scale_factor = self.device_scale_factor * scale;
        self.viewport_size = self
            .platform_window
            .content_size()
            .map(|value| value / scale);
        self.mouse_position = self
            .platform_window
            .mouse_position()
            .map(|value| value / scale);
        self.refresh();
        true
    }
}

pub(super) fn input(mut event: PlatformInput, scale: f32) -> PlatformInput {
    if scale == 1. {
        return event;
    }
    let point = |point: Point<Pixels>| point.map(|value| value / scale);
    match &mut event {
        PlatformInput::MouseDown(event) => event.position = point(event.position),
        PlatformInput::MouseUp(event) => event.position = point(event.position),
        PlatformInput::MouseMove(event) => event.position = point(event.position),
        PlatformInput::MouseExited(event) => event.position = point(event.position),
        PlatformInput::MousePressure(event) => event.position = point(event.position),
        PlatformInput::ScrollWheel(event) => {
            event.position = point(event.position);
            if let crate::ScrollDelta::Pixels(delta) = &mut event.delta {
                *delta = point(*delta);
            }
        }
        PlatformInput::Pinch(event) => event.position = point(event.position),
        PlatformInput::LongPress(event) => {
            event.start_position = point(event.start_position);
            event.position = point(event.position);
        }
        PlatformInput::TouchDrag(event) => {
            event.start_position = point(event.start_position);
            event.position = point(event.position);
        }
        PlatformInput::Touch(event) => {
            event.position = point(event.position);
            event.predicted_position = event.predicted_position.map(point);
        }
        PlatformInput::FileDrop(
            FileDropEvent::Entered { position, .. }
            | FileDropEvent::Pending { position }
            | FileDropEvent::Submit { position },
        ) => *position = point(*position),
        PlatformInput::FileDrop(FileDropEvent::Exited | FileDropEvent::Ended)
        | PlatformInput::KeyDown(_)
        | PlatformInput::KeyUp(_)
        | PlatformInput::ModifiersChanged(_) => {}
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_pointer_and_pixel_scroll_in_the_same_logical_space() {
        let event = PlatformInput::ScrollWheel(crate::ScrollWheelEvent {
            position: point(px(300.), px(120.)),
            delta: crate::ScrollDelta::Pixels(point(px(0.), px(60.))),
            ..Default::default()
        });
        let PlatformInput::ScrollWheel(event) = input(event, 1.5) else {
            panic!("scroll event changed type")
        };
        assert_eq!(event.position, point(px(200.), px(80.)));
        assert!(
            matches!(event.delta, crate::ScrollDelta::Pixels(delta) if delta == point(px(0.), px(40.)))
        );
    }
}
