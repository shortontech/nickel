//! Security authority for Nickel's optional remote-control service.
//!
//! Transport adapters may present pairing challenges and MCP requests, but only this state
//! machine can turn a locally approved client into a scoped capability.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

mod server;
pub use server::{DesktopAuthority, RemoteControlServer, ServerError, WindowSummary};
mod settings;
pub use settings::{RemoteAiControlSettings, SettingsError};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveState {
    Disabled,
    Enabled,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub requested_enabled: bool,
    pub effective: EffectiveState,
    pub generation: u64,
    pub acknowledged_generation: u64,
    pub endpoint: &'static str,
    pub diagnostic: Option<String>,
}

pub struct RemoteControlRuntime {
    control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
    server: Option<RemoteControlServer>,
    status: RuntimeStatus,
}

impl Default for RemoteControlRuntime {
    fn default() -> Self {
        Self {
            control: Default::default(),
            server: None,
            status: RuntimeStatus {
                requested_enabled: false,
                effective: EffectiveState::Disabled,
                generation: 0,
                acknowledged_generation: 0,
                endpoint: MCP_ENDPOINT,
                diagnostic: None,
            },
        }
    }
}

impl RemoteControlRuntime {
    pub fn status(&self) -> &RuntimeStatus {
        &self.status
    }

    pub fn set_diagnostic(&mut self, diagnostic: impl Into<String>) {
        self.status.diagnostic = Some(diagnostic.into().chars().take(256).collect());
    }

    pub fn control(&self) -> std::sync::Arc<std::sync::Mutex<ControlPlane>> {
        self.control.clone()
    }

    pub fn apply(
        &mut self,
        settings: &RemoteAiControlSettings,
        desktop: std::sync::Arc<dyn DesktopAuthority>,
    ) {
        if settings.generation < self.status.acknowledged_generation {
            return;
        }
        self.status.requested_enabled = settings.requested_enabled;
        self.status.generation = settings.generation;
        self.status.acknowledged_generation = settings.generation;
        self.status.diagnostic = None;
        if !settings.requested_enabled {
            self.stop(EffectiveState::Disabled);
            return;
        }
        if self.server.is_some() {
            self.status.effective = EffectiveState::Enabled;
            return;
        }
        self.control.lock().unwrap().set_enabled(true);
        match RemoteControlServer::start(self.control.clone(), desktop) {
            Ok(server) => {
                self.server = Some(server);
                self.status.effective = EffectiveState::Enabled;
            }
            Err(error) => {
                self.control.lock().unwrap().set_enabled(false);
                self.status.effective = EffectiveState::Rejected;
                self.status.diagnostic = Some(error.to_string().chars().take(256).collect());
            }
        }
    }

    pub fn emergency_stop(&mut self) {
        let generation = self.status.generation.saturating_add(1);
        self.emergency_stop_at(generation);
    }

    pub fn emergency_stop_at(&mut self, generation: u64) {
        self.stop(EffectiveState::Disabled);
        self.status.requested_enabled = false;
        self.status.generation = generation;
        self.status.acknowledged_generation = generation;
    }

    /// End authority for this login session without changing the user's persisted preference.
    pub fn shutdown_session(&mut self) {
        self.stop(EffectiveState::Disabled);
    }

    pub fn lock(&mut self) {
        self.control.lock().unwrap().lock();
    }

    fn stop(&mut self, effective: EffectiveState) {
        if let Some(server) = self.server.take() {
            server.stop();
        }
        self.control.lock().unwrap().set_enabled(false);
        self.status.effective = effective;
    }
}

pub const MCP_ENDPOINT: &str = "http://127.0.0.1:42637/mcp";
pub const PAIRING_LIFETIME_SECS: u64 = 5 * 60;
pub const MAX_PAIRING_ATTEMPTS: u8 = 6;
const GLOBAL_PAIRING_ATTEMPTS: usize = 24;
const GLOBAL_PAIRING_WINDOW_SECS: u64 = 60;
pub const MAX_PENDING_CLIENTS: usize = 16;
const TOKEN_BYTES: usize = 32;
const CEREMONY_BYTES: usize = 16;
const CODE_SYMBOLS: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Observe,
    WindowManagement,
    SettingsRead,
    SettingsChange,
    ApplicationLaunch,
    PointerInput,
    KeyboardInput,
    ScreenCapture,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PairingDisplay {
    pub ceremony_id: String,
    pub qr_payload: String,
    pub short_code: String,
    pub expires_at: u64,
}

impl std::fmt::Debug for PairingDisplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingDisplay")
            .field("ceremony_id", &self.ceremony_id)
            .field("qr_payload", &"<redacted>")
            .field("short_code", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingClient {
    pub id: String,
    pub label: String,
    pub requested: Vec<Capability>,
    pub connected_at: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub struct GrantedClient {
    pub id: String,
    pub label: String,
    pub capabilities: Vec<Capability>,
    pub remembered: bool,
    token: [u8; TOKEN_BYTES],
    token_claimable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GrantedClientSummary {
    pub id: String,
    pub label: String,
    pub capabilities: Vec<Capability>,
    pub remembered: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct IssuedCapability {
    pub client_id: String,
    pub token: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Approval {
    Deny,
    AllowOnce,
    Remember,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ControlError {
    #[error("remote control is disabled")]
    Disabled,
    #[error("pairing ceremony is missing or expired")]
    PairingExpired,
    #[error("pairing challenge was rejected")]
    PairingRejected,
    #[error("pairing attempt limit reached")]
    AttemptLimit,
    #[error("pairing attempts are temporarily rate limited")]
    RateLimited,
    #[error("pending-client capacity reached")]
    Capacity,
    #[error("client is no longer pending")]
    StaleClient,
    #[error("randomness is unavailable")]
    Randomness,
    #[error("capability is invalid or revoked")]
    Unauthorized,
    #[error("remembering a client requires secure credential storage, which is not available yet")]
    PersistentApprovalUnavailable,
}

#[derive(Clone)]
struct PairingCeremony {
    id: String,
    qr_secret: [u8; TOKEN_BYTES],
    code_digest: [u8; 32],
    expires_at: u64,
    attempts: u8,
}

#[derive(Default)]
pub struct ControlPlane {
    enabled: bool,
    generation: u64,
    pairing: Option<PairingCeremony>,
    pending: BTreeMap<String, PendingClient>,
    granted: BTreeMap<String, GrantedClient>,
    pairing_attempts: VecDeque<u64>,
}

/// Stateful recognizer fed only by native physical-key adapters. Remote/synthetic reducers must
/// never call this recognizer, which makes the emergency chord unavailable to a controlled client.
#[derive(Default)]
pub struct EmergencyChord {
    left: bool,
    right: bool,
    fired: bool,
}

impl EmergencyChord {
    pub fn handle_physical(
        &mut self,
        key: nickel_input::KeyCode,
        edge: nickel_input::KeyEdge,
        remote_control_enabled: bool,
    ) -> bool {
        if !remote_control_enabled {
            self.left = false;
            self.right = false;
            self.fired = false;
            return false;
        }
        let pressed = edge == nickel_input::KeyEdge::Pressed;
        match key {
            nickel_input::KeyCode::ControlLeft => self.left = pressed,
            nickel_input::KeyCode::ControlRight => self.right = pressed,
            _ => return false,
        }
        if !self.left || !self.right {
            self.fired = false;
            return false;
        }
        if self.fired {
            return false;
        }
        self.fired = true;
        true
    }
}

impl ControlPlane {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.generation = self.generation.saturating_add(1);
        if !enabled {
            self.revoke_runtime_authority();
        }
    }

    pub fn start_pairing(&mut self, now: u64) -> Result<PairingDisplay, ControlError> {
        if !self.enabled {
            return Err(ControlError::Disabled);
        }
        let ceremony_random = random::<CEREMONY_BYTES>()?;
        let qr_secret = random::<TOKEN_BYTES>()?;
        let code_random = random::<5>()?;
        let ceremony_id = hex(&ceremony_random);
        let short_code = encode_code(code_random);
        let expires_at = now.saturating_add(PAIRING_LIFETIME_SECS);
        let qr_payload = format!(
            "nickel://pair/v1?ceremony={ceremony_id}&secret={}",
            hex(&qr_secret)
        );
        self.pairing = Some(PairingCeremony {
            id: ceremony_id.clone(),
            qr_secret,
            code_digest: digest(short_code.as_bytes()),
            expires_at,
            attempts: 0,
        });
        self.generation = self.generation.saturating_add(1);
        Ok(PairingDisplay {
            ceremony_id,
            qr_payload,
            short_code,
            expires_at,
        })
    }

    pub fn cancel_pairing(&mut self) {
        if self.pairing.take().is_some() {
            self.generation = self.generation.saturating_add(1);
        }
    }

    pub fn exchange_short_code(
        &mut self,
        ceremony_id: &str,
        short_code: &str,
        label: &str,
        requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        let candidate = digest(short_code.trim().to_ascii_uppercase().as_bytes());
        self.exchange(ceremony_id, &candidate, false, label, requested, now)
    }

    pub fn exchange_qr_secret(
        &mut self,
        ceremony_id: &str,
        qr_secret_hex: &str,
        label: &str,
        requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        let secret = decode_hex::<TOKEN_BYTES>(qr_secret_hex).unwrap_or([0; TOKEN_BYTES]);
        self.exchange(ceremony_id, &secret, true, label, requested, now)
    }

    fn exchange(
        &mut self,
        ceremony_id: &str,
        candidate: &[u8; 32],
        qr: bool,
        label: &str,
        mut requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        if !self.enabled {
            return Err(ControlError::Disabled);
        }
        while self
            .pairing_attempts
            .front()
            .is_some_and(|attempt| now.saturating_sub(*attempt) >= GLOBAL_PAIRING_WINDOW_SECS)
        {
            self.pairing_attempts.pop_front();
        }
        if self.pairing_attempts.len() >= GLOBAL_PAIRING_ATTEMPTS {
            return Err(ControlError::RateLimited);
        }
        self.pairing_attempts.push_back(now);
        let ceremony = self.pairing.as_mut().ok_or(ControlError::PairingExpired)?;
        if now > ceremony.expires_at {
            self.pairing = None;
            return Err(ControlError::PairingExpired);
        }
        if ceremony.attempts >= MAX_PAIRING_ATTEMPTS {
            self.pairing = None;
            return Err(ControlError::AttemptLimit);
        }
        ceremony.attempts += 1;
        let expected = if qr {
            &ceremony.qr_secret
        } else {
            &ceremony.code_digest
        };
        let valid_id = ceremony.id.as_bytes().ct_eq(ceremony_id.as_bytes()).into();
        let valid_secret: bool = expected.ct_eq(candidate).into();
        if !(valid_id && valid_secret) {
            if ceremony.attempts >= MAX_PAIRING_ATTEMPTS {
                self.pairing = None;
                return Err(ControlError::AttemptLimit);
            }
            return Err(ControlError::PairingRejected);
        }
        if self.pending.len() >= MAX_PENDING_CLIENTS {
            return Err(ControlError::Capacity);
        }
        validate_label(label)?;
        requested.sort();
        requested.dedup();
        let client = PendingClient {
            id: hex(&random::<CEREMONY_BYTES>()?),
            label: label.to_owned(),
            requested,
            connected_at: now,
        };
        self.pairing = None;
        self.pending.insert(client.id.clone(), client.clone());
        self.generation = self.generation.saturating_add(1);
        Ok(client)
    }

    pub fn pending_clients(&self) -> impl Iterator<Item = &PendingClient> {
        self.pending.values()
    }

    pub fn granted_clients(&self) -> impl Iterator<Item = GrantedClientSummary> + '_ {
        self.granted.values().map(|client| GrantedClientSummary {
            id: client.id.clone(),
            label: client.label.clone(),
            capabilities: client.capabilities.clone(),
            remembered: client.remembered,
        })
    }

    pub fn approve(
        &mut self,
        client_id: &str,
        approval: Approval,
        locally_confirmed_capabilities: Vec<Capability>,
    ) -> Result<Option<IssuedCapability>, ControlError> {
        let pending = self
            .pending
            .get(client_id)
            .cloned()
            .ok_or(ControlError::StaleClient)?;
        if approval == Approval::Deny {
            self.pending.remove(client_id);
            self.generation = self.generation.saturating_add(1);
            return Ok(None);
        }
        if approval == Approval::Remember {
            return Err(ControlError::PersistentApprovalUnavailable);
        }
        let mut capabilities = locally_confirmed_capabilities;
        capabilities.sort();
        capabilities.dedup();
        if capabilities
            .iter()
            .any(|capability| !pending.requested.contains(capability))
        {
            return Err(ControlError::Unauthorized);
        }
        let token = random::<TOKEN_BYTES>()?;
        self.pending.remove(client_id);
        self.generation = self.generation.saturating_add(1);
        let issued = IssuedCapability {
            client_id: pending.id.clone(),
            token: hex(&token),
            capabilities: capabilities.clone(),
        };
        self.granted.insert(
            pending.id.clone(),
            GrantedClient {
                id: pending.id,
                label: pending.label,
                capabilities,
                remembered: false,
                token,
                token_claimable: true,
            },
        );
        Ok(Some(issued))
    }

    /// Deliver a newly approved credential exactly once to the client that holds the random
    /// pending-client identity returned by the challenge exchange.
    pub fn claim_issued(&mut self, client_id: &str) -> Option<IssuedCapability> {
        let grant = self.granted.get_mut(client_id)?;
        if !grant.token_claimable {
            return None;
        }
        grant.token_claimable = false;
        Some(IssuedCapability {
            client_id: grant.id.clone(),
            token: hex(&grant.token),
            capabilities: grant.capabilities.clone(),
        })
    }

    pub fn authorize(&self, client_id: &str, token_hex: &str, capability: Capability) -> bool {
        let candidate = decode_hex::<TOKEN_BYTES>(token_hex).unwrap_or([0; TOKEN_BYTES]);
        self.granted.get(client_id).is_some_and(|grant| {
            bool::from(grant.token.ct_eq(&candidate)) && grant.capabilities.contains(&capability)
        })
    }

    pub fn authenticate(&self, client_id: &str, token_hex: &str) -> bool {
        let candidate = decode_hex::<TOKEN_BYTES>(token_hex).unwrap_or([0; TOKEN_BYTES]);
        self.granted
            .get(client_id)
            .is_some_and(|grant| bool::from(grant.token.ct_eq(&candidate)))
    }

    pub fn revoke(&mut self, client_id: &str) -> bool {
        let removed = self.granted.remove(client_id).is_some();
        if removed {
            self.generation = self.generation.saturating_add(1);
        }
        removed
    }

    pub fn lock(&mut self) {
        self.revoke_runtime_authority();
        self.generation = self.generation.saturating_add(1);
    }

    pub fn emergency_stop(&mut self) {
        self.enabled = false;
        self.revoke_runtime_authority();
        self.generation = self.generation.saturating_add(1);
    }

    fn revoke_runtime_authority(&mut self) {
        self.pairing = None;
        self.pending.clear();
        self.granted.clear();
    }
}

fn validate_label(label: &str) -> Result<(), ControlError> {
    (!label.trim().is_empty() && label.len() <= 128)
        .then_some(())
        .ok_or(ControlError::PairingRejected)
}

fn random<const N: usize>() -> Result<[u8; N], ControlError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| ControlError::Randomness)?;
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn encode_code(bytes: [u8; 5]) -> String {
    let value = u64::from_be_bytes([0, 0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]]);
    let symbols = (0..8)
        .rev()
        .map(|shift| CODE_SYMBOLS[((value >> (shift * 5)) & 31) as usize] as char)
        .collect::<String>();
    format!("{}-{}", &symbols[..4], &symbols[4..])
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_digit(chunk[0])? << 4) | hex_digit(chunk[1])?;
    }
    Some(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disabled_and_pairing_never_grants_before_local_approval() {
        let mut plane = ControlPlane::default();
        assert_eq!(plane.start_pairing(10), Err(ControlError::Disabled));
        plane.set_enabled(true);
        let display = plane.start_pairing(10).unwrap();
        let secret = display.qr_payload.split("secret=").nth(1).unwrap();
        let pending = plane
            .exchange_qr_secret(
                &display.ceremony_id,
                secret,
                "Phone",
                vec![Capability::Observe, Capability::PointerInput],
                11,
            )
            .unwrap();
        assert!(!plane.authorize(&pending.id, secret, Capability::Observe));
        let issued = plane
            .approve(&pending.id, Approval::AllowOnce, vec![Capability::Observe])
            .unwrap()
            .unwrap();
        assert!(plane.authorize(&pending.id, &issued.token, Capability::Observe));
        assert!(!plane.authorize(&pending.id, &issued.token, Capability::PointerInput));
    }

    #[test]
    fn short_code_is_single_use_expiring_and_attempt_bounded() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let display = plane.start_pairing(20).unwrap();
        plane
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code.to_ascii_lowercase(),
                "Phone",
                vec![Capability::Observe],
                21,
            )
            .unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "Replay",
                vec![],
                22
            ),
            Err(ControlError::PairingExpired)
        );

        let expired = plane.start_pairing(100).unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &expired.ceremony_id,
                &expired.short_code,
                "Late",
                vec![],
                100 + PAIRING_LIFETIME_SECS + 1,
            ),
            Err(ControlError::PairingExpired)
        );

        let limited = plane.start_pairing(200).unwrap();
        for _ in 0..MAX_PAIRING_ATTEMPTS - 1 {
            assert_eq!(
                plane.exchange_short_code(&limited.ceremony_id, "NOPE-NOPE", "Bad", vec![], 201),
                Err(ControlError::PairingRejected)
            );
        }
        assert_eq!(
            plane.exchange_short_code(&limited.ceremony_id, "NOPE-NOPE", "Bad", vec![], 201),
            Err(ControlError::AttemptLimit)
        );
    }

    #[test]
    fn pairing_attempt_budget_survives_challenge_rotation() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        for index in 0..GLOBAL_PAIRING_ATTEMPTS {
            let display = plane.start_pairing(100 + index as u64).unwrap();
            assert_eq!(
                plane.exchange_short_code(
                    &display.ceremony_id,
                    "NOPE-NOPE",
                    "Guess",
                    vec![],
                    100 + index as u64,
                ),
                Err(ControlError::PairingRejected)
            );
        }
        let blocked = plane.start_pairing(125).unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &blocked.ceremony_id,
                &blocked.short_code,
                "Phone",
                vec![],
                125,
            ),
            Err(ControlError::RateLimited)
        );

        let recovered = plane.start_pairing(200).unwrap();
        assert!(
            plane
                .exchange_short_code(
                    &recovered.ceremony_id,
                    &recovered.short_code,
                    "Phone",
                    vec![],
                    200,
                )
                .is_ok()
        );
    }

    #[test]
    fn lock_disable_and_emergency_stop_revoke_every_transient_authority() {
        for action in 0..3 {
            let mut plane = ControlPlane::default();
            plane.set_enabled(true);
            let display = plane.start_pairing(1).unwrap();
            let pending = plane
                .exchange_short_code(
                    &display.ceremony_id,
                    &display.short_code,
                    "Phone",
                    vec![Capability::KeyboardInput],
                    2,
                )
                .unwrap();
            let issued = plane
                .approve(
                    &pending.id,
                    Approval::AllowOnce,
                    vec![Capability::KeyboardInput],
                )
                .unwrap()
                .unwrap();
            match action {
                0 => plane.lock(),
                1 => plane.set_enabled(false),
                _ => plane.emergency_stop(),
            }
            assert!(!plane.authorize(&pending.id, &issued.token, Capability::KeyboardInput));
            assert_eq!(plane.pending_clients().count(), 0);
        }
    }

    #[test]
    fn remember_is_rejected_until_a_secure_persistent_store_exists() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let display = plane.start_pairing(1).unwrap();
        let pending = plane
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "Phone",
                vec![Capability::Observe],
                2,
            )
            .unwrap();
        assert!(matches!(
            plane.approve(&pending.id, Approval::Remember, vec![Capability::Observe]),
            Err(ControlError::PersistentApprovalUnavailable)
        ));
        assert_eq!(plane.pending_clients().count(), 1);
        assert_eq!(plane.granted_clients().count(), 0);
    }

    #[test]
    fn emergency_chord_requires_both_physical_sides_and_fires_once_per_hold() {
        use nickel_input::{KeyCode, KeyEdge};
        let mut chord = EmergencyChord::default();
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, false));
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, true));
        assert!(chord.handle_physical(KeyCode::ControlRight, KeyEdge::Pressed, true));
        assert!(!chord.handle_physical(KeyCode::ControlRight, KeyEdge::Pressed, true));
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Released, true));
        assert!(chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, true));
    }
}
