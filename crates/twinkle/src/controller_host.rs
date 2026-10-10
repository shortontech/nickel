//! Explicit host ingress for controller execution. Native standalone input remains in Twinkle.
use crate::{
    ControllerAction, ControllerExecutionAuthority, ControllerExecutionBinding, ControllerFamily,
};

pub type HostedControllerDelivery = (
    Option<ControllerAction>,
    ControllerFamily,
    ControllerExecutionBinding,
    ControllerExecutionAuthority,
);

/// A trusted host provides bounded, nonblocking delivery and an independent execution oracle.
/// The engine still checks authority and pairing at dispatch. Implementations must never
/// derive the oracle by trusting fields in an unvalidated delivery.
pub trait HostedControllerSource {
    fn connected(&self) -> bool;
    fn poll_actions(&mut self) -> Vec<HostedControllerDelivery>;
    fn report_execution_overflow(&mut self, binding: ControllerExecutionBinding);
    fn relinquish(&mut self);
}

/// Source selection is explicit; transport failure cannot switch hosted input to native polling.
pub enum ControllerSource {
    Standalone,
    Disabled,
    Hosted(Box<dyn HostedControllerSource>),
}
