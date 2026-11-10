use super::super::*;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

fn failure(value: JsValue) -> ClientError {
    ClientError::Unavailable {
        reason: format!("artwork fetch failed: {value:?}"),
    }
}
fn get(object: &JsValue, field: &str) -> Result<JsValue, ClientError> {
    Reflect::get(object, &field.into()).map_err(failure)
}
fn invoke(object: &JsValue, name: &str, args: &Array) -> Result<JsValue, ClientError> {
    get(object, name)?
        .dyn_into::<Function>()
        .map_err(failure)?
        .apply(object, args)
        .map_err(failure)
}
async fn promise(value: JsValue) -> Result<JsValue, ClientError> {
    JsFuture::from(value.dyn_into::<Promise>().map_err(failure)?)
        .await
        .map_err(failure)
}
struct Abort(JsValue);
impl Drop for Abort {
    fn drop(&mut self) {
        let _ = invoke(&self.0, "abort", &Array::new());
    }
}

// Fetch's redirect:error is required: validating response.url would be too late.
pub(crate) async fn fetch_artwork(url: &str) -> Result<Vec<u8>, ClientError> {
    let global = js_sys::global();
    let constructor = get(&global, "AbortController")?
        .dyn_into::<Function>()
        .map_err(failure)?;
    let abort = Abort(Reflect::construct(&constructor, &Array::new()).map_err(failure)?);
    let init = Object::new();
    for (name, value) in [
        ("redirect", JsValue::from_str("error")),
        ("credentials", JsValue::from_str("omit")),
        ("signal", get(&abort.0, "signal")?),
    ] {
        Reflect::set(&init, &name.into(), &value).map_err(failure)?;
    }
    let args = Array::new();
    args.push(&url.into());
    args.push(&init);
    let response = promise(invoke(&global, "fetch", &args)?).await?;
    let status = get(&response, "status")?.as_f64().unwrap_or(0.0) as u16;
    if status == 404 {
        return Err(ClientError::NotFound {
            reason: "artwork not found".into(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(ClientError::Unavailable {
            reason: format!("artwork HTTP {status}"),
        });
    }
    let headers = get(&response, "headers")?;
    let header = |name: &str| -> Result<Option<String>, ClientError> {
        let args = Array::new();
        args.push(&name.into());
        Ok(invoke(&headers, "get", &args)?.as_string())
    };
    if header("content-length")?
        .and_then(|n| n.parse::<u64>().ok())
        .is_some_and(|n| n > 4 * 1024 * 1024)
    {
        return Err(ClientError::ResourceExhausted {
            reason: "artwork exceeds 4 MiB".into(),
        });
    }
    if !["image/jpeg", "image/png", "image/svg+xml"]
        .contains(&header("content-type")?.unwrap_or_default().as_str())
    {
        return Err(ClientError::InvalidResponse {
            reason: "unsupported artwork MIME".into(),
        });
    }
    let reader = invoke(&get(&response, "body")?, "getReader", &Array::new())?;
    let mut bytes = Vec::new();
    loop {
        let chunk = promise(invoke(&reader, "read", &Array::new())?).await?;
        if get(&chunk, "done")?.as_bool() == Some(true) {
            break;
        }
        let value = Uint8Array::new(&get(&chunk, "value")?);
        if bytes.len() + value.length() as usize > 4 * 1024 * 1024 {
            return Err(ClientError::ResourceExhausted {
                reason: "artwork exceeds 4 MiB".into(),
            });
        }
        bytes.extend(value.to_vec());
    }
    Ok(bytes)
}
