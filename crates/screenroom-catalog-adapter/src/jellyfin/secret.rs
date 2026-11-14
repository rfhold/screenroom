use super::{ApiKey, Error, Result};
use std::path::Path;
#[cfg(unix)]
use std::{io::Read, path::Component};

#[cfg(unix)]
pub(super) fn read_key(path: &Path) -> Result<ApiKey> {
    use std::{
        ffi::CString,
        fs::File,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path
            .to_str()
            .is_none_or(|s| s.chars().any(char::is_control))
        || path
            .as_os_str()
            .as_bytes()
            .split(|b| *b == b'/')
            .any(|s| s == b"." || s == b"..")
    {
        return Err(Error::Secret);
    }
    let parts: Vec<_> = path.components().collect();
    if parts.len() < 2
        || !parts
            .iter()
            .skip(1)
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(Error::Secret);
    }
    let mut parent = File::open("/").map_err(|_| Error::Secret)?;
    for (index, part) in parts.iter().enumerate().skip(1) {
        let Component::Normal(part) = part else {
            return Err(Error::Secret);
        };
        let name = CString::new(part.as_bytes()).map_err(|_| Error::Secret)?;
        let last = index == parts.len() - 1;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if last { 0 } else { libc::O_DIRECTORY };
        // The directory descriptor pins each component; no symlink can race path traversal.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::Secret);
        }
        // openat returned a new owned descriptor, including on every error path below.
        let file = unsafe { File::from_raw_fd(fd) };
        if last {
            let metadata = file.metadata().map_err(|_| Error::Secret)?;
            if !metadata.is_file()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.len() > 4096
            {
                return Err(Error::Secret);
            }
            let mut bytes = Vec::new();
            file.take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Secret)?;
            if bytes.len() > 4096 {
                return Err(Error::Secret);
            }
            let value = String::from_utf8(bytes).map_err(|_| Error::Secret)?;
            return ApiKey::new(value.trim_end_matches(['\r', '\n']).to_owned());
        }
        parent = file;
    }
    Err(Error::Secret)
}

#[cfg(not(unix))]
pub(super) fn read_key(_: &Path) -> Result<ApiKey> {
    // No weaker fallback on platforms without descriptor-relative no-follow opens.
    Err(Error::Secret)
}

#[cfg(all(test, unix))]
mod tests;
