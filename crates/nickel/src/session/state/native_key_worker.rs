use super::RemoteDesktopRequest;
use smithay::{
    input::keyboard::KeyboardSource, reexports::calloop::channel::SyncSender, xwayland::X11Surface,
};
use std::{io, sync::mpsc};

pub(super) enum NativeKeyObservation {
    Pressed(bool),
    Modifiers(
        smithay::xwayland::xwm::X11IsolatedModifiers,
        Box<smithay::xwayland::xwm::X11IsolatedKeymap>,
    ),
}

pub(super) struct NativeKeyQuery {
    pub prepare_keyboard: bool,
    pub surface: X11Surface,
    pub source: KeyboardSource,
    pub sequence: u64,
    pub code: u8,
}

pub(super) struct NativeKeyWorker(mpsc::SyncSender<NativeKeyQuery>);

impl NativeKeyWorker {
    pub(super) fn start(reply: SyncSender<RemoteDesktopRequest>) -> io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel::<NativeKeyQuery>(1);
        std::thread::Builder::new()
            .name("native-key-state".into())
            .spawn(move || {
                while let Ok(query) = requests.recv() {
                    let result = if query.prepare_keyboard {
                        query.surface.isolated_modifiers().and_then(|modifiers| {
                            query.surface.isolated_keymap().map(|keymap| {
                                NativeKeyObservation::Modifiers(modifiers, Box::new(keymap))
                            })
                        })
                    } else {
                        query
                            .surface
                            .isolated_key_is_pressed(query.code)
                            .map(NativeKeyObservation::Pressed)
                    }
                    .map_err(|_| "native keyboard state unavailable".to_owned());
                    let _ = reply.try_send(RemoteDesktopRequest::NativeKeyboardState {
                        source: query.source,
                        sequence: query.sequence,
                        result,
                    });
                }
            })?;
        Ok(Self(sender))
    }

    pub(super) fn submit(&self, query: NativeKeyQuery) -> Result<(), String> {
        self.0
            .try_send(query)
            .map_err(|_| "native keyboard verifier is busy or stopped".to_owned())
    }
}
