use super::NickelSession;

impl NickelSession {
    /// Called outside control-plane transactions. Local controller activity owns
    /// the seat before any further remote input, including asynchronous repeats.
    pub(super) fn poll_remote_controller_ownership(&mut self) -> bool {
        let observation = self
            .remote_controller_observer
            .observe(std::time::Instant::now());
        let busy = !observation.available
            || observation.held
            || observation.activity
            || observation.backlog
            || self.native_controller_input_held().unwrap_or(true);
        if busy {
            self.cancel_remote_pointer();
            self.cancel_remote_keyboard();
        }
        busy
    }

    /// EVIOCGKEY/EVIOCGABS observe input held before the event reader started. Reading
    /// through a separate fd neither grabs the device nor consumes another reader's events.
    fn native_controller_input_held(&self) -> Result<bool, &'static str> {
        for controller in self.remote_controller_observer.connected_devices()? {
            let device =
                evdev::Device::open(controller.path).map_err(|_| "controller state unavailable")?;
            let keys = device
                .get_key_state()
                .map_err(|_| "controller button state unavailable")?;
            if keys.iter().next().is_some() {
                return Ok(true);
            }
            let state = device
                .get_abs_state()
                .map_err(|_| "controller axis state unavailable")?;
            for axis in device
                .supported_absolute_axes()
                .into_iter()
                .flat_map(|axes| axes.iter())
            {
                // Gilrs' Linux event code representation is (event type << 16) | code.
                let code = (u32::from(evdev::EventType::ABSOLUTE.0) << 16) | u32::from(axis.0);
                let button_axis = controller.button_codes.contains(&code);
                let info = &state[usize::from(axis.0)];
                if native_axis_held(info.value, info.minimum, info.maximum, button_axis)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn native_axis_held(
    value: i32,
    minimum: i32,
    maximum: i32,
    button_axis: bool,
) -> Result<bool, &'static str> {
    if maximum <= minimum || value < minimum || value > maximum {
        return Err("controller axis range unavailable");
    }
    let mut range = f64::from(maximum) - f64::from(minimum);
    let mut offset = f64::from(value) - f64::from(minimum);
    let normalized = if button_axis {
        offset / range
    } else {
        // Match Gilrs' centering of odd integer ranges (e.g. -32768..32767).
        if maximum
            .checked_sub(minimum)
            .is_some_and(|range| range % 2 == 1)
        {
            range += 1.0;
            offset += 1.0;
        }
        (offset / range * 2.0 - 1.0).abs()
    };
    let threshold =
        f64::from(nickel_input::controller::ControllerConfig::default().release_threshold_milli)
            / 1000.0;
    Ok(normalized >= threshold)
}

#[cfg(test)]
mod tests {
    use super::native_axis_held;

    #[test]
    fn startup_axis_snapshot_distinguishes_centered_sticks_and_mapped_triggers() {
        for (minimum, maximum, neutral) in [(-32768, 32767, 0), (0, 255, 127), (-1, 1, 0)] {
            assert_eq!(
                native_axis_held(neutral, minimum, maximum, false),
                Ok(false)
            );
            assert_eq!(native_axis_held(minimum, minimum, maximum, false), Ok(true));
            assert_eq!(native_axis_held(maximum, minimum, maximum, false), Ok(true));
        }
        // The same raw range has a different neutral point when mapped to a button.
        assert_eq!(native_axis_held(0, 0, 255, true), Ok(false));
        assert_eq!(native_axis_held(127, 0, 255, true), Ok(true));
        assert_eq!(native_axis_held(-32768, -32768, 32767, true), Ok(false));
        assert_eq!(native_axis_held(32767, -32768, 32767, true), Ok(true));
    }

    #[test]
    fn startup_axis_snapshot_preserves_release_threshold_and_handles_wide_ranges() {
        assert_eq!(native_axis_held(34, -100, 100, false), Ok(false));
        assert_eq!(native_axis_held(35, -100, 100, false), Ok(true));
        assert_eq!(native_axis_held(-35, -100, 100, false), Ok(true));
        assert_eq!(native_axis_held(0, i32::MIN, i32::MAX, false), Ok(false));
        assert_eq!(
            native_axis_held(i32::MIN, i32::MIN, i32::MAX, false),
            Ok(true)
        );
        assert_eq!(
            native_axis_held(i32::MAX, i32::MIN, i32::MAX, true),
            Ok(true)
        );
    }

    #[test]
    fn startup_axis_snapshot_refuses_invalid_range_or_sample() {
        for (value, minimum, maximum) in [(0, 0, 0), (0, 1, -1), (2, -1, 1), (-2, -1, 1)] {
            assert!(native_axis_held(value, minimum, maximum, false).is_err());
            assert!(native_axis_held(value, minimum, maximum, true).is_err());
        }
    }
}
