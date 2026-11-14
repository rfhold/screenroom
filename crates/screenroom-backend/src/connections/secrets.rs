use crate::{Error, Result};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::{Component, PathBuf},
};
use uuid::Uuid;

fn unavailable() -> Error {
    Error::Connection("credential_store_unavailable")
}
fn component(value: &std::ffi::OsStr) -> Result<CString> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(value.as_bytes()).map_err(|_| unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn private_descriptor_publication_rejects_aliases_and_never_overwrites() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("connection-files-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory = CredentialDirectory::new(root.join("keys")).unwrap();
        let id = Uuid::new_v4();
        directory.publish(id, "first-key").unwrap();
        assert_eq!(directory.read(id).unwrap(), "first-key");
        assert!(directory.publish(id, "replacement-key").is_err());
        assert_eq!(directory.read(id).unwrap(), "first-key");
        let path = root.join("keys").join(format!("{id}.key"));
        let alias = Uuid::new_v4();
        let alias_path = root.join("keys").join(format!("{alias}.key"));
        symlink(&path, &alias_path).unwrap();
        assert!(directory.read(alias).is_err());
        assert!(directory.publish(alias, "other").is_err());
        std::fs::remove_file(&alias_path).unwrap();
        std::fs::hard_link(&path, &alias_path).unwrap();
        assert!(directory.read(id).is_err());
        std::fs::remove_file(&alias_path).unwrap();
        let fifo = CString::new(alias_path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(directory.read(alias).is_err());
        std::fs::remove_file(&alias_path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(directory.read(id).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, vec![b'x'; 4097]).unwrap();
        assert!(directory.read(id).is_err());
        let link = root.join("link");
        symlink(root.join("keys"), &link).unwrap();
        assert!(
            CredentialDirectory::new(link.clone())
                .unwrap()
                .publish(Uuid::new_v4(), "key")
                .is_err()
        );
        assert!(
            CredentialDirectory::new(link.join("child"))
                .unwrap()
                .publish(Uuid::new_v4(), "key")
                .is_err()
        );
        assert!(CredentialDirectory::new(PathBuf::from("relative")).is_err());
        assert!(CredentialDirectory::new(root.join("../escape")).is_err());
        std::fs::set_permissions(root.join("keys"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert!(directory.publish(Uuid::new_v4(), "key").is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir(root.join("keys")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
fn open_at(parent: &File, name: &CString, flags: i32, mode: u32) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(unavailable());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn metadata(file: &File, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().map_err(|_| unavailable())?;
    if m.uid() != unsafe { libc::geteuid() }
        || (directory && (!m.is_dir() || m.mode() & 0o777 != 0o700))
        || (!directory
            && (!m.is_file() || m.mode() & 0o777 != 0o600 || m.nlink() != 1 || m.len() > 4096))
    {
        return Err(unavailable());
    }
    Ok(())
}

#[derive(Clone)]
pub struct CredentialDirectory(PathBuf);
impl std::fmt::Debug for CredentialDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialDirectory([REDACTED])")
    }
}
impl CredentialDirectory {
    pub fn new(path: PathBuf) -> Result<Self> {
        if !path.is_absolute()
            || path.file_name().is_none()
            || path
                .to_str()
                .is_none_or(|s| s.chars().any(char::is_control))
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(unavailable());
        }
        Ok(Self(path))
    }
    fn open(&self, create: bool) -> Result<File> {
        let mut directory = File::open("/").map_err(|_| unavailable())?;
        let mut names = self
            .0
            .components()
            .filter_map(|c| {
                if let Component::Normal(n) = c {
                    Some(n)
                } else {
                    None
                }
            })
            .peekable();
        while let Some(name) = names.next() {
            let name = component(name)?;
            if create && names.peek().is_none() {
                let status = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if status != 0
                    && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
                {
                    return Err(unavailable());
                }
            }
            directory = open_at(&directory, &name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        }
        metadata(&directory, true)?;
        Ok(directory)
    }
    pub(crate) fn read(&self, id: Uuid) -> Result<String> {
        let directory = self.open(false)?;
        let name = CString::new(format!("{id}.key")).map_err(|_| unavailable())?;
        let mut file = open_at(&directory, &name, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
        metadata(&file, false)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        metadata(&file, false)?;
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(unavailable());
        }
        String::from_utf8(bytes).map_err(|_| unavailable())
    }
    pub(crate) fn publish(&self, id: Uuid, key: &str) -> Result<()> {
        if key.is_empty() || key.len() > 4096 {
            return Err(unavailable());
        }
        let directory = self.open(true)?;
        let temporary =
            CString::new(format!(".{}.tmp", Uuid::new_v4())).map_err(|_| unavailable())?;
        let final_name = CString::new(format!("{id}.key")).map_err(|_| unavailable())?;
        let mut file = open_at(
            &directory,
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let result = (|| {
            metadata(&file, false)?;
            file.write_all(key.as_bytes()).map_err(|_| unavailable())?;
            file.sync_all().map_err(|_| unavailable())?;
            if unsafe {
                libc::linkat(
                    directory.as_raw_fd(),
                    temporary.as_ptr(),
                    directory.as_raw_fd(),
                    final_name.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(unavailable());
            }
            Ok(())
        })();
        // Only this newly-created temporary name is removed; published files are immutable.
        if unsafe { libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0) } != 0 {
            return Err(unavailable());
        }
        result?;
        directory.sync_all().map_err(|_| unavailable())?;
        Ok(())
    }
}
