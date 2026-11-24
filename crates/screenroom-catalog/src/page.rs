use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use screenroom_backend::catalog::Scope;
use screenroom_proto::{catalog::v1::Page, common::v1::SourceLibraryScope};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tonic::Status;

pub(crate) fn scope(source: &str, library: &str) -> Result<Scope, Status> {
    let scope = Scope {
        source_keys: if source.is_empty() {
            vec![]
        } else {
            vec![source.into()]
        },
        library_ids: if library.is_empty() || library == "all" {
            vec![]
        } else {
            vec![library.into()]
        },
        ..Scope::default()
    };
    scope.validate().map_err(crate::status)?;
    Ok(scope)
}
pub(crate) fn request_scope(value: Option<SourceLibraryScope>) -> Result<Scope, Status> {
    let value = value.unwrap_or_default();
    scope(&value.source_id, &value.library_id)
}
#[derive(Serialize, Deserialize)]
struct Token<T> {
    binding: String,
    cursor: T,
}
pub(crate) fn binding(operation: &str, target: &str, scope: &Scope) -> String {
    serde_json::json!([operation, target, scope]).to_string()
}
pub(crate) fn decode<T: DeserializeOwned>(
    page: Option<&Page>,
    binding: &str,
) -> Result<(u32, Option<T>), Status> {
    decode_with_legacy(page, binding, None)
}

pub(crate) fn decode_with_legacy<T: DeserializeOwned>(
    page: Option<&Page>,
    binding: &str,
    legacy: Option<&str>,
) -> Result<(u32, Option<T>), Status> {
    let size = page.map_or(0, |p| p.size);
    let size = if size == 0 { 50 } else { size };
    if size > 200 {
        return Err(Status::invalid_argument("page size exceeds 200"));
    }
    let Some(token) = page.map(|p| p.token.as_str()).filter(|s| !s.is_empty()) else {
        return Ok((size, None));
    };
    if token.len() > 8192 {
        return Err(Status::invalid_argument("page token"));
    }
    let token: Token<T> = URL_SAFE_NO_PAD
        .decode(token)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Status::invalid_argument("page token"))?;
    if token.binding != binding && legacy != Some(token.binding.as_str()) {
        return Err(Status::invalid_argument("page token scope"));
    }
    Ok((size, Some(token.cursor)))
}
pub(crate) fn encode<T: Serialize>(binding: &str, cursor: &T) -> Result<String, Status> {
    let bytes = serde_json::to_vec(&Token {
        binding: binding.into(),
        cursor,
    })
    .map_err(|_| Status::internal("page token"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_bind_operation_target_scope_and_bound_size() {
        let a = binding("children", "a", &scope("source", "all").unwrap());
        let b = binding("credits", "a", &scope("source", "all").unwrap());
        let token = encode(&a, &"cursor").unwrap();
        let page = Page { size: 0, token };
        assert_eq!(
            decode::<String>(Some(&page), &a).unwrap(),
            (50, Some("cursor".into()))
        );
        assert!(decode::<String>(Some(&page), &b).is_err());
        assert!(
            decode::<String>(
                Some(&Page {
                    size: 201,
                    token: String::new()
                }),
                &a
            )
            .is_err()
        );
        assert!(scope("", "all").unwrap().library_ids.is_empty());
    }
}
