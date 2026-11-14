use super::*;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};

#[test]
fn private_regular_bounded_files_only_without_symlinks_or_traversal() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("screenroom-key-policy-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let key = root.join("key");
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key)
            .unwrap();
        file.write_all(b"private-test-key\n").unwrap();
    }
    assert_eq!(read_key(&key).unwrap().value(), "private-test-key");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(read_key(&key).unwrap_err(), Error::Secret);
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.join("link");
    symlink(&key, &link).unwrap();
    assert_eq!(read_key(&link).unwrap_err(), Error::Secret);
    let dirlink = root.join("dirlink");
    symlink(&root, &dirlink).unwrap();
    assert_eq!(read_key(&dirlink.join("key")).unwrap_err(), Error::Secret);
    assert_eq!(read_key(&root.join("../key")).unwrap_err(), Error::Secret);
    assert_eq!(
        read_key(Path::new("relative/key")).unwrap_err(),
        Error::Secret
    );
    assert_eq!(read_key(&root).unwrap_err(), Error::Secret);
    let hardlink = root.join("hardlink");
    std::fs::hard_link(&key, &hardlink).unwrap();
    assert_eq!(read_key(&key).unwrap_err(), Error::Secret);
    std::fs::remove_file(hardlink).unwrap();
    std::fs::write(&key, vec![b'x'; 4096]).unwrap();
    assert_eq!(read_key(&key).unwrap().value().len(), 4096);
    std::fs::write(&key, vec![b'x'; 4097]).unwrap();
    assert_eq!(read_key(&key).unwrap_err(), Error::Secret);
    std::fs::remove_file(link).unwrap();
    std::fs::remove_file(dirlink).unwrap();
    std::fs::remove_file(key).unwrap();
    std::fs::remove_dir(root).unwrap();
}
