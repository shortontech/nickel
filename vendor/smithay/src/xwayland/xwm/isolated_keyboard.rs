//! One XInput master pair for recipient-bound synthetic keyboard delivery.
use x11rb::{
    connection::{Connection, RequestConnection},
    errors::{ConnectionError, ReplyError},
    protocol::xinput::{
        self, ConnectionExt, HierarchyChange, HierarchyChangeData, HierarchyChangeDataAddMaster,
        HierarchyChangeDataRemoveMaster,
    },
    rust_connection::RustConnection,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct IsolatedKeyboard {
    pub master: u16,
    pub core_pointer: u16,
    pub core_master: u16,
    pub slave: u8,
    pub event_base: u8,
}

impl IsolatedKeyboard {
    /// Setup round trips belong to XWM startup, never an input dispatch.
    pub fn create(conn: &RustConnection, identity: u32) -> Result<Self, ReplyError> {
        let extension = conn
            .extension_information(xinput::X11_EXTENSION_NAME)?
            .ok_or(ConnectionError::UnsupportedExtension)?;
        conn.xinput_xi_query_version(2, 0)?.reply()?;
        {
            use x11rb::protocol::xkb::ConnectionExt as _;
            if !conn.xkb_use_extension(1, 0)?.reply()?.supported {
                return Err(ConnectionError::UnsupportedExtension.into());
            }
        }
        let core_pointer = conn.xinput_xi_get_client_pointer(0u32)?.reply()?.deviceid;
        let core_master = conn
            .xinput_xi_query_device(core_pointer)?
            .reply()?
            .infos
            .first()
            .ok_or(ConnectionError::UnknownError)?
            .attachment;
        let name = format!("Smithay scoped keyboard {identity}").into_bytes();
        let data = HierarchyChangeDataAddMaster {
            // This master exists solely for recipient-bound synthetic input.
            // Advertising it as a core device makes Xwayland route ordinary
            // physical keyboard events through the isolated hierarchy too,
            // leaving legacy X11 clients without keyboard input.
            send_core: false,
            enable: true,
            name: name.clone(),
        };
        let len = (8 + name.len()).div_ceil(4) as u16;
        conn.xinput_xi_change_hierarchy(&[HierarchyChange {
            len,
            data: HierarchyChangeData::AddMaster(data),
        }])?
        .check()?;
        let devices = conn.xinput_xi_query_device(0u16)?.reply()?.infos;
        let keyboard_name = [name.as_slice(), b" keyboard"].concat();
        let master = devices
            .iter()
            .find(|device| {
                device.name == keyboard_name && device.type_ == xinput::DeviceType::MASTER_KEYBOARD
            })
            .ok_or(ConnectionError::UnknownError)?
            .deviceid;
        let keyboard = Self {
            master,
            core_pointer,
            core_master,
            slave: 0,
            event_base: extension.first_event,
        };
        let slave_name = [name.as_slice(), b" XTEST keyboard"].concat();
        let slave = devices
            .iter()
            .find(|device| {
                device.attachment == master
                    && device.type_ == xinput::DeviceType::SLAVE_KEYBOARD
                    && device.name == slave_name
            })
            .and_then(|device| u8::try_from(device.deviceid).ok())
            .filter(|id| *id < 128);
        // XTEST extension-event encoding has only seven device-ID bits.
        // Never leave a newly allocated pair behind when it cannot be used.
        match slave {
            Some(slave) => Ok(Self { slave, ..keyboard }),
            None => {
                keyboard.remove(conn);
                Err(ConnectionError::UnknownError.into())
            }
        }
    }

    pub fn observe_focus(self, conn: &RustConnection, window: u32) -> Result<(), ConnectionError> {
        // XIAddMaster can otherwise become the implicit client pointer for X11
        // clients connecting after the scoped pair is created. Keep ordinary
        // application input on Xwayland's virtual-core pointer/keyboard pair;
        // the scoped master remains available for explicit remote delivery.
        conn.xinput_xi_set_client_pointer(window, self.core_pointer)?;
        conn.xinput_xi_select_events(
            window,
            &[xinput::EventMask {
                deviceid: self.core_master,
                mask: vec![xinput::XIEventMask::FOCUS_IN | xinput::XIEventMask::FOCUS_OUT],
            }],
        )?;
        Ok(())
    }

    pub fn remove(self, conn: &RustConnection) {
        let _ = conn.xinput_xi_change_hierarchy(&[HierarchyChange {
            len: 3,
            data: HierarchyChangeData::RemoveMaster(HierarchyChangeDataRemoveMaster {
                deviceid: self.master,
                return_mode: xinput::ChangeMode::FLOAT,
                return_pointer: 0,
                return_keyboard: 0,
            }),
        }]);
        let _ = conn.flush();
    }
}
