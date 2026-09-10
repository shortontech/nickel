//! Process-owned listener configuration. MCP and Settings cannot mutate these values.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct ListenerConfig {
    pub(crate) address: SocketAddr,
    pub(crate) environment_override: bool,
    pub(crate) certificate: Option<PathBuf>,
    pub(crate) private_key: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error(
        "NICKEL_MCP_LISTEN_ADDR must be a numeric IP and nonzero port; wildcard addresses are not allowed"
    )]
    Address,
    #[error("non-loopback MCP requires NICKEL_MCP_TLS_CERT and NICKEL_MCP_TLS_KEY")]
    TlsRequired,
    #[error("configure both NICKEL_MCP_TLS_CERT and NICKEL_MCP_TLS_KEY")]
    TlsIncomplete,
}

pub(crate) struct TlsMaterial {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
    pub fingerprint: String,
}

impl ListenerConfig {
    pub(crate) fn tls_material(&self) -> Result<Option<TlsMaterial>, std::io::Error> {
        use sha2::Digest;
        use std::io::Read;
        let (Some(cert), Some(key)) = (&self.certificate, &self.private_key) else {
            return Ok(None);
        };
        let read = |path| -> Result<Vec<u8>, std::io::Error> {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 {
                return Err(std::io::Error::other("TLS material exceeds size limit"));
            }
            Ok(bytes)
        };
        let cert = read(cert)?;
        let key = read(key)?;
        let der = rustls_pemfile::certs(&mut cert.as_slice())
            .next()
            .ok_or_else(|| std::io::Error::other("certificate chain is empty"))??;
        let fingerprint = crate::hex(&sha2::Sha256::digest(der.as_ref()));
        Ok(Some(TlsMaterial {
            certificate: cert,
            private_key: key,
            fingerprint,
        }))
    }
    pub fn from_environment() -> Result<Self, ListenerError> {
        let address =
            std::env::var("NICKEL_MCP_LISTEN_ADDR")
                .map(Some)
                .or_else(|error| match error {
                    std::env::VarError::NotPresent => Ok(None),
                    _ => Err(ListenerError::Address),
                })?;
        Self::parse(
            address.as_deref(),
            std::env::var_os("NICKEL_MCP_TLS_CERT").map(PathBuf::from),
            std::env::var_os("NICKEL_MCP_TLS_KEY").map(PathBuf::from),
        )
    }

    pub fn parse(
        address: Option<&str>,
        certificate: Option<PathBuf>,
        private_key: Option<PathBuf>,
    ) -> Result<Self, ListenerError> {
        let parsed = address
            .map(str::parse::<SocketAddr>)
            .transpose()
            .map_err(|_| ListenerError::Address)?
            .unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 42637));
        if parsed.port() == 0 || parsed.ip().is_unspecified() || parsed.ip().is_multicast() {
            return Err(ListenerError::Address);
        }
        if certificate.is_some() != private_key.is_some() {
            return Err(ListenerError::TlsIncomplete);
        }
        if !parsed.ip().is_loopback() && certificate.is_none() {
            return Err(ListenerError::TlsRequired);
        }
        Ok(Self {
            address: parsed,
            environment_override: address.is_some(),
            certificate,
            private_key,
        })
    }

    pub fn endpoint(&self) -> String {
        format!(
            "{}://{}/mcp",
            if self.certificate.is_some() {
                "https"
            } else {
                "http"
            },
            self.address
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_numeric_override_never_accepts_ambiguous_or_unprotected_binding() {
        for address in [
            "localhost:42637",
            "0.0.0.0:42637",
            "[::]:42637",
            "127.0.0.1:0",
            "https://127.0.0.1:42637",
            "127.0.0.1",
            "127.0.0.1:42637 ",
            "127.0.0.1:42637junk",
        ] {
            assert!(
                ListenerConfig::parse(Some(address), None, None).is_err(),
                "{address}"
            );
        }
        assert!(matches!(
            ListenerConfig::parse(Some("192.168.1.40:42637"), None, None),
            Err(ListenerError::TlsRequired)
        ));
        let config = ListenerConfig::parse(
            Some("192.168.1.40:42638"),
            Some("cert.pem".into()),
            Some("key.pem".into()),
        )
        .unwrap();
        assert_eq!(config.endpoint(), "https://192.168.1.40:42638/mcp");
        assert!(config.environment_override);
        assert_eq!(
            ListenerConfig::parse(Some("[::1]:42638"), None, None)
                .unwrap()
                .endpoint(),
            "http://[::1]:42638/mcp"
        );
    }
}
