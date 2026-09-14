use std::{ffi::OsString, io};

#[derive(Debug)]
pub(crate) struct LocalAdvertisement {
    pub endpoint: OsString,
    pub capability: String,
}

/// Resolve local-session discovery without weakening an advertised authority failure into local
/// mode. Endpoint absence is the only successful `None` result.
pub(crate) fn advertisement_from_environment() -> io::Result<Option<LocalAdvertisement>> {
    resolve_advertisement(
        std::env::var_os("NICKEL_SESSION_CONTROL"),
        std::env::var("NICKEL_SESSION_TOKEN").ok(),
    )
}

fn resolve_advertisement(
    endpoint: Option<OsString>,
    capability: Option<String>,
) -> io::Result<Option<LocalAdvertisement>> {
    let Some(endpoint) = endpoint else {
        return Ok(None);
    };
    let capability = capability
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "session capability unavailable",
            )
        })?;
    Ok(Some(LocalAdvertisement {
        endpoint,
        capability,
    }))
}

#[cfg(test)]
mod tests {
    use super::resolve_advertisement;
    use std::{ffi::OsString, io};

    #[test]
    fn only_absent_advertisement_permits_local_mode() {
        assert!(resolve_advertisement(None, None).unwrap().is_none());
        for capability in [None, Some(String::new())] {
            assert_eq!(
                resolve_advertisement(Some(OsString::from("endpoint")), capability)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        let advertised = resolve_advertisement(
            Some(OsString::from("endpoint")),
            Some(String::from("capability")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(advertised.endpoint, OsString::from("endpoint"));
        assert_eq!(advertised.capability, "capability");
    }
}
