//! Authority-neutral approval copy shared by application cards and shell notifications.
//!
//! This projection never owns decisions. A source authority must validate the live request
//! before dispatching any action displayed beside it.

pub const MAX_APPROVAL_PRESENTATION_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequesterIdentity {
    VerifiedLocal,
    SelfReportedRemote,
    BackendReported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalPresentation {
    pub requester: String,
    pub identity: RequesterIdentity,
    pub action: String,
    pub scope: Option<String>,
    pub duration: Option<String>,
    pub warning: Option<String>,
    pub detail: Option<String>,
}

impl ApprovalPresentation {
    pub fn is_within_budget(&self) -> bool {
        let bytes = self.requester.len()
            + self.action.len()
            + self.scope.as_ref().map_or(0, String::len)
            + self.duration.as_ref().map_or(0, String::len)
            + self.warning.as_ref().map_or(0, String::len)
            + self.detail.as_ref().map_or(0, String::len);
        bytes <= MAX_APPROVAL_PRESENTATION_BYTES
    }

    pub fn title(&self) -> String {
        format!("{} requests approval", self.requester)
    }

    pub fn scope_line(&self) -> String {
        match &self.scope {
            Some(scope) => format!("Scope: {scope}."),
            None => "Scope details were not supplied by the requester.".into(),
        }
    }

    pub fn identity_line(&self) -> &'static str {
        match self.identity {
            RequesterIdentity::VerifiedLocal => "Requester identity verified locally.",
            RequesterIdentity::SelfReportedRemote => "Requester label is self-reported.",
            RequesterIdentity::BackendReported => "Requester identity reported by the backend.",
        }
    }

    pub fn notification_body(&self) -> String {
        let mut body = format!("{}. {}", self.action, self.scope_line());
        if let Some(duration) = &self.duration {
            body.push_str(&format!(" Duration: {duration}."));
        }
        body.push(' ');
        body.push_str(self.identity_line());
        if let Some(warning) = &self.warning {
            body.push(' ');
            body.push_str(warning);
        }
        body
    }

    /// The notification's scrollable detail area exposes source-supplied consent facts.
    /// Callers must not put credentials or transport diagnostics in `detail`.
    pub fn notification_body_with_detail(&self) -> String {
        let mut body = self.notification_body();
        if let Some(detail) = &self.detail {
            body.push('\n');
            body.push_str(detail);
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_scope_and_self_reported_identity_are_explicit() {
        let request = ApprovalPresentation {
            requester: "Agent".into(),
            identity: RequesterIdentity::SelfReportedRemote,
            action: "Control Terminal".into(),
            scope: None,
            duration: Some("10 minutes".into()),
            warning: None,
            detail: None,
        };
        assert!(
            request
                .notification_body()
                .contains("Scope details were not supplied")
        );
        assert!(request.notification_body().contains("self-reported"));
        assert!(request.is_within_budget());
    }

    #[test]
    fn presentation_budget_rejects_unbounded_source_copy() {
        let request = ApprovalPresentation {
            requester: "Agent".into(),
            identity: RequesterIdentity::BackendReported,
            action: "x".repeat(MAX_APPROVAL_PRESENTATION_BYTES),
            scope: None,
            duration: None,
            warning: None,
            detail: None,
        };
        assert!(!request.is_within_budget());
    }

    #[test]
    fn consent_detail_is_explicitly_opted_into_notification_body() {
        let request = ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Run a command".into(),
            scope: Some("/projects/nickel".into()),
            duration: None,
            warning: None,
            detail: Some("Command: cargo test".into()),
        };
        assert!(!request.notification_body().contains("cargo test"));
        assert!(
            request
                .notification_body_with_detail()
                .contains("cargo test")
        );
    }
}
