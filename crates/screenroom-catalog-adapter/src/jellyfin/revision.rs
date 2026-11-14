use super::{Error, JellyfinConfig, LiveJellyfin, Result, validate_config};
use sha2::{Digest, Sha256};
use std::path::{Component, PathBuf};

/// Opaque SHA-256 identity of private Jellyfin locators and request limits.
///
/// This is not an authentication token or a dataset identity. It excludes token
/// contents, filesystem metadata, symlink targets, and remote server state, so
/// credential rotation and a replaced server at the same URL do not change it.
/// Store it privately: a digest does not conceal guessable configuration inputs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigurationRevision(String);

impl std::fmt::Debug for ConfigurationRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfigurationRevision([REDACTED])")
    }
}

impl ConfigurationRevision {
    /// Validates persisted lowercase hexadecimal SHA-256, not its provenance.
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::InvalidConfig);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn from_config(config: &JellyfinConfig) -> Result<Self> {
        let mut base = validate_config(config)?;
        // Request construction removes exactly one terminal empty path segment.
        if base.path().len() > 1 && base.path().ends_with('/') {
            let path = base.path()[..base.path().len() - 1].to_owned();
            base.set_path(&path);
        }
        let path = &config.secret_file;
        if !path.is_absolute()
            || path.file_name().is_none()
            || path
                .to_str()
                .is_none_or(|s| s.chars().any(char::is_control))
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(Error::InvalidConfig);
        }
        // Lexical only: never canonicalize, stat, or open the token locator.
        let path: PathBuf = path.components().collect();
        let path = path.to_str().ok_or(Error::InvalidConfig)?;
        let mut digest = Sha256::new();
        digest.update(b"screenroom/jellyfin/configuration-revision/v1\0");
        for value in [
            config.source_key.as_bytes(),
            base.as_str().as_bytes(),
            path.as_bytes(),
            &config.timeout_ms.to_be_bytes(),
            &(config.max_response_bytes as u64).to_be_bytes(),
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value);
        }
        Ok(Self(format!("{:x}", digest.finalize())))
    }
}

impl LiveJellyfin {
    /// Revision of this instance's immutable configuration, without token IO.
    ///
    /// Requires an absolute, control-free token locator without parent segments;
    /// legacy direct configurations using relative locators cannot be bound.
    /// This identifies locators, not token contents or the remote dataset.
    pub fn configuration_revision(&self) -> Result<ConfigurationRevision> {
        ConfigurationRevision::from_config(&self.config)
    }
}

#[cfg(test)]
mod tests;
