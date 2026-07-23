// Adapted from smithay-drm-extras 0.1.0. See NOTICE.md.

use std::collections::HashMap;

use smithay::reexports::drm::control::{Device, connector, crtc};

#[derive(Debug)]
pub enum DrmScanEvent {
    Connected {
        connector: connector::Info,
        crtc: Option<crtc::Handle>,
    },
    Disconnected {
        connector: connector::Info,
        crtc: Option<crtc::Handle>,
    },
}

#[derive(Debug, Default)]
pub struct DrmScanner {
    connectors: HashMap<connector::Handle, connector::Info>,
    crtcs: HashMap<connector::Handle, crtc::Handle>,
}

impl DrmScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan_connectors(&mut self, drm: &impl Device) -> std::io::Result<Vec<DrmScanEvent>> {
        let resources = drm.resource_handles()?;
        let current: Vec<_> = resources
            .connectors()
            .iter()
            .filter_map(|handle| drm.get_connector(*handle, true).ok())
            .collect();
        let mut connected = Vec::new();
        let mut disconnected = Vec::new();

        for connector in &current {
            let previous = self
                .connectors
                .insert(connector.handle(), connector.clone());
            match (
                previous.as_ref().map(connector::Info::state),
                connector.state(),
            ) {
                (None, connector::State::Connected)
                | (
                    Some(connector::State::Disconnected | connector::State::Unknown),
                    connector::State::Connected,
                ) => {
                    connected.push(connector.clone());
                }
                (Some(connector::State::Connected), connector::State::Disconnected) => {
                    disconnected.push(connector.clone());
                }
                _ => {}
            }
        }

        let mut events = disconnected
            .into_iter()
            .map(|connector| {
                let crtc = self.crtcs.remove(&connector.handle());
                DrmScanEvent::Disconnected { connector, crtc }
            })
            .collect::<Vec<_>>();

        for connector in current
            .iter()
            .filter(|connector| connector.state() != connector::State::Connected)
        {
            self.crtcs.remove(&connector.handle());
        }
        self.assign_crtcs(drm, &current);

        events.extend(
            connected
                .into_iter()
                .map(|connector| DrmScanEvent::Connected {
                    crtc: self.crtcs.get(&connector.handle()).copied(),
                    connector,
                }),
        );
        Ok(events)
    }

    fn assign_crtcs(&mut self, drm: &impl Device, connectors: &[connector::Info]) {
        let needs_crtc: Vec<_> = connectors
            .iter()
            .filter(|connector| connector.state() == connector::State::Connected)
            .filter(|connector| !self.crtcs.contains_key(&connector.handle()))
            .cloned()
            .collect();
        for connector in &needs_crtc {
            if let Some(crtc) = self
                .restored_crtc(drm, connector)
                .or_else(|| self.available_crtc(drm, connector))
            {
                self.crtcs.insert(connector.handle(), crtc);
            }
        }
    }

    fn restored_crtc(
        &self,
        drm: &impl Device,
        connector: &connector::Info,
    ) -> Option<crtc::Handle> {
        let encoder = drm.get_encoder(connector.current_encoder()?).ok()?;
        let crtc = encoder.crtc()?;
        (!self.crtcs.values().any(|assigned| assigned == &crtc)).then_some(crtc)
    }

    fn available_crtc(
        &self,
        drm: &impl Device,
        connector: &connector::Info,
    ) -> Option<crtc::Handle> {
        let resources = drm.resource_handles().ok()?;
        connector
            .encoders()
            .iter()
            .filter_map(|handle| drm.get_encoder(*handle).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .find(|candidate| !self.crtcs.values().any(|assigned| assigned == candidate))
    }
}
