use screenroom_backend::{Uuid, config::ServiceDatabaseConfig, db};
use screenroom_user::repository::{Repository, bounded};
use serde::Deserialize;
use std::io::Read;

const CONFIG_LIMIT: u64 = 1024 * 1024;

fn arguments(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(bool, std::path::PathBuf), &'static str> {
    let usage = "usage: screenroom-user-init [--create-missing] <users.json>";
    let mut args = args.into_iter();
    let first = args.next().ok_or(usage)?;
    let create_missing = first == "--create-missing";
    let path = if create_missing {
        args.next().ok_or(usage)?
    } else {
        first
    };
    if path.to_string_lossy().starts_with('-') || args.next().is_some() {
        return Err(usage);
    }
    Ok((create_missing, path.into()))
}

fn read_config(reader: impl Read) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    reader.take(CONFIG_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Err("user configuration too large".into());
    }
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct User {
    id: Uuid,
    display_name: String,
    #[serde(default, rename = "links", deserialize_with = "unsupported_links")]
    _links: (),
}

fn unsupported_links<'de, D: serde::Deserializer<'de>>(_: D) -> Result<(), D::Error> {
    Err(serde::de::Error::custom(
        "user configuration links are unsupported; select a validated Jellyfin profile through the public user service",
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (create_missing, path) = arguments(std::env::args_os().skip(1))?;
    let bytes = read_config(std::fs::File::open(path)?)?;
    let users: Vec<User> = serde_json::from_slice(&bytes)?;
    if users.len() > 1000 {
        return Err("too many bootstrap users".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for user in &users {
        if !ids.insert(user.id) {
            return Err("duplicate bootstrap user".into());
        }
        bounded(&user.display_name)?;
    }
    let repository =
        Repository::new(db::connect_service(&ServiceDatabaseConfig::from_env()?).await?);
    repository.ready().await?;
    for user in users {
        if create_missing {
            repository
                .create_missing_user(user.id, &user.display_name)
                .await?;
            continue;
        }
        repository.init_user(user.id, &user.display_name).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_missing_is_explicit_and_arguments_are_bounded() {
        let parse = |args: &[&str]| arguments(args.iter().map(std::ffi::OsString::from));
        assert_eq!(
            parse(&["users.json"]).unwrap(),
            (false, "users.json".into())
        );
        assert_eq!(
            parse(&["--create-missing", "users.json"]).unwrap(),
            (true, "users.json".into())
        );
        for args in [
            &[][..],
            &["--create-missing"][..],
            &["--unknown", "users.json"][..],
            &["users.json", "extra"][..],
            &["--create-missing", "users.json", "extra"][..],
        ] {
            assert!(parse(args).is_err());
        }
    }

    #[test]
    fn input_is_bounded_before_parsing() {
        let mut oversized = std::io::Cursor::new(vec![b' '; CONFIG_LIMIT as usize + 100]);
        assert_eq!(
            read_config(&mut oversized).unwrap_err().to_string(),
            "user configuration too large"
        );
        assert_eq!(oversized.position(), CONFIG_LIMIT + 1);
        assert_eq!(
            read_config(std::io::repeat(b' ').take(CONFIG_LIMIT))
                .unwrap()
                .len(),
            CONFIG_LIMIT as usize
        );
        assert_eq!(read_config(&b"[]"[..]).unwrap(), b"[]");
    }
}
