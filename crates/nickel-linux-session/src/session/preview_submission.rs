//! Preview frame submission shared by native and nested renderers.

/// Explicitly finish even a failed draw before returning its error. Smithay
/// waits when an unfinished frame is dropped, so a `?` inside the draw path
/// otherwise introduces an implicit compositor-thread wait. This only controls
/// error ordering: the renderer's own finish fallback can still block.
pub(super) fn finish_preview_submission<F, T, E>(
    mut frame: F,
    draw: impl FnOnce(&mut F) -> Result<(), E>,
    finish: impl FnOnce(F) -> Result<T, E>,
) -> Result<T, E> {
    let drawn = draw(&mut frame);
    let finished = finish(frame);
    drawn?;
    finished
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    struct Frame {
        finished: bool,
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Drop for Frame {
        fn drop(&mut self) {
            self.events.borrow_mut().push(if self.finished {
                "drop finished"
            } else {
                "implicit wait"
            });
        }
    }

    #[test]
    fn failed_preview_draw_finishes_before_error_propagation_without_drop_wait() {
        for (draw_result, finish_result, expected) in [
            (Ok(()), Ok(7), Ok(7)),
            (Err("draw"), Ok(7), Err("draw")),
            (Ok(()), Err("finish"), Err("finish")),
            (Err("draw"), Err("finish"), Err("draw")),
        ] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let frame = Frame {
                finished: false,
                events: events.clone(),
            };
            let result = finish_preview_submission(
                frame,
                |frame| {
                    frame.events.borrow_mut().push("draw");
                    draw_result
                },
                |mut frame| {
                    frame.events.borrow_mut().push("finish");
                    // Model Smithay's finished flag, which is set before its
                    // fallible completion work. No real GPU is exercised here.
                    frame.finished = true;
                    finish_result
                },
            );
            assert_eq!(result, expected);
            assert_eq!(*events.borrow(), ["draw", "finish", "drop finished"]);
        }
    }
}
