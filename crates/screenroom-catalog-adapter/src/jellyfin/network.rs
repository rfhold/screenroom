use super::{Error, Result};
use reqwest::{
    Url,
    dns::{Addrs, Name, Resolve, Resolving},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

#[derive(Debug)]
struct NetworkFailure(Error);
impl std::fmt::Display for NetworkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("provider destination unavailable")
    }
}
impl std::error::Error for NetworkFailure {}

pub(super) fn classify(error: &reqwest::Error) -> Option<Error> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(failure) = current.downcast_ref::<NetworkFailure>() {
            return Some(failure.0.clone());
        }
        source = current.source();
    }
    None
}

pub(super) fn allowed(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            a != 0
                && !v4.is_multicast()
                && a < 240
                && !(a == 169 && b == 254)
                && d != 255
                && [a, b, c, d] != [100, 100, 100, 200]
                && [a, b, c, d] != [192, 0, 0, 192]
                && [a, b, c, d] != [168, 63, 129, 16]
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return allowed(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            !v6.is_unspecified() && !v6.is_multicast()
                && (segments[0] & 0xffc0 != 0xfe80)
                && segments != [0xfd00,0xec2,0,0,0,0,0,0x254]
                && segments != [0xfd20,0xce,0,0,0,0,0,0x254]
                // Do not admit IPv4 transition/translation paths around the IPv4 policy.
                && (v6.is_loopback() || segments[..6] != [0,0,0,0,0,0])
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && segments[1] == 0)
                && !(segments[0] == 0x64 && segments[1] == 0xff9b)
        }
    }
}

pub(super) fn validate_url(url: &Url) -> Result<()> {
    let host = url.host_str().ok_or(Error::InvalidConfig)?;
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if !allowed(ip) {
            return Err(Error::DestinationRejected);
        }
    } else {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty()
            || host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(Error::InvalidConfig);
        }
    }
    if url.port() == Some(0) {
        return Err(Error::InvalidConfig);
    }
    Ok(())
}

trait Lookup: Send + Sync {
    fn lookup(&self, host: String) -> Resolving;
}
struct SystemLookup;
impl Lookup for SystemLookup {
    fn lookup(&self, host: String) -> Resolving {
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let owned: Vec<_> = addresses.take(65).collect();
            Ok(Box::new(owned.into_iter()) as Addrs)
        })
    }
}

pub(super) struct GuardedResolver {
    lookup: Arc<dyn Lookup>,
    timeout: Duration,
}
impl GuardedResolver {
    pub(super) fn new(timeout_ms: u64) -> Self {
        Self {
            lookup: Arc::new(SystemLookup),
            timeout: Duration::from_millis(timeout_ms.min(5000)),
        }
    }
}
impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = self.lookup.clone();
        let timeout = self.timeout;
        Box::pin(async move {
            let addresses = tokio::time::timeout(timeout, lookup.lookup(name.as_str().to_owned()))
                .await
                .map_err(|_| NetworkFailure(Error::Deadline))?
                .map_err(|_| NetworkFailure(Error::Resolution))?;
            let mut approved: Vec<SocketAddr> = Vec::new();
            for address in addresses {
                if !allowed(address.ip()) {
                    return Err(NetworkFailure(Error::DestinationRejected).into());
                }
                if approved.len() == 64 {
                    return Err(NetworkFailure(Error::Resolution).into());
                }
                approved.push(address);
            }
            if approved.is_empty() {
                return Err(NetworkFailure(Error::Resolution).into());
            }
            // Hyper connects directly to these sockets, retaining the URL host for TLS/Host.
            Ok(Box::new(approved.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests;
