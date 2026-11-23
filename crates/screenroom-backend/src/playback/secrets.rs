//! Immutable private files. A database reference must be committed before publication.
use crate::{Result, Uuid, connections::CredentialDirectory};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::PathBuf};

pub struct RetiredCredential {
    pub(super) reference: Uuid,
}

/// This directory must be a separate gateway-only mount, not the managed-key directory.
#[derive(Clone)]
pub struct PlaybackSecrets(CredentialDirectory, PathBuf);

impl std::fmt::Debug for PlaybackSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlaybackSecrets([REDACTED])")
    }
}

impl PlaybackSecrets {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self(CredentialDirectory::new(path.clone())?, path))
    }

    /// Never remove on a database error: commit outcomes can be ambiguous.
    pub fn publish(&self, reference: Uuid, secret: &str) -> Result<()> {
        self.0.publish(reference, secret)
    }

    pub fn read(&self, reference: Uuid) -> Result<String> {
        self.0.read(reference)
    }

    /// Accepts only repository-issued evidence for a task-owned, retired UUID.
    pub fn remove_retired(&self, retired: &RetiredCredential) -> Result<()> {
        remove_private(&self.1, retired.reference)
    }
}

fn remove_private(path: &std::path::Path, reference: Uuid) -> Result<()> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::Component,
    };
    let unavailable = || crate::Error::Unavailable;
    let mut directory = File::open("/").map_err(|_| unavailable())?;
    for part in path.components() {
        if let Component::Normal(name) = part {
            let name = CString::new(name.as_bytes()).map_err(|_| unavailable())?;
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(unavailable());
            }
            directory = unsafe { File::from_raw_fd(fd) };
        }
    }
    let metadata = directory.metadata().map_err(|_| unavailable())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(unavailable());
    }
    let name = CString::new(format!("{reference}.key")).map_err(|_| unavailable())?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(())
        } else {
            Err(unavailable())
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > 4096
    {
        return Err(unavailable());
    }
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(unavailable());
    }
    directory.sync_all().map_err(|_| unavailable())?;
    Ok(())
}

pub struct Capability(String);

impl std::fmt::Debug for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Capability([REDACTED])")
    }
}

impl Capability {
    pub fn generate() -> Result<Self> {
        let mut bytes = [0; 32];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|_| crate::Error::Unavailable)?;
        Ok(Self(hex::encode(bytes)))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> Vec<u8> {
        capability_hash(&self.0)
    }
}

pub fn capability_hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn independent_random_capabilities_and_redacted_debug() {
        let a = Capability::generate().unwrap();
        let b = Capability::generate().unwrap();
        assert_eq!(a.expose().len(), 64);
        assert_ne!(a.expose(), b.expose());
        assert_eq!(a.hash().len(), 32);
        assert!(!format!("{a:?}").contains(a.expose()));
    }

    #[test]
    fn private_store_never_overwrites_or_follows_aliases() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("playback-secrets-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = PlaybackSecrets::new(root.join("tokens")).unwrap();
        let id = Uuid::new_v4();
        store.publish(id, "private-replay").unwrap();
        assert!(store.publish(id, "replacement").is_err());
        assert_eq!(store.read(id).unwrap(), "private-replay");
        assert!(!format!("{store:?}").contains(root.to_str().unwrap()));
        let alias = Uuid::new_v4();
        let original = root.join("tokens").join(format!("{id}.key"));
        let alias_path = root.join("tokens").join(format!("{alias}.key"));
        symlink(&original, &alias_path).unwrap();
        assert!(store.read(alias).is_err());
        assert!(store.publish(alias, "replacement").is_err());
        assert!(
            store
                .remove_retired(&RetiredCredential { reference: alias })
                .is_err()
        );
        std::fs::remove_file(alias_path).unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.read(id).is_err());
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o600)).unwrap();
        let retired = RetiredCredential { reference: id };
        store.remove_retired(&retired).unwrap();
        store.remove_retired(&retired).unwrap();
        std::fs::remove_dir(root.join("tokens")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
