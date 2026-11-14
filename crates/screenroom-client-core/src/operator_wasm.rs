use crate::*;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = include_str!("operator-models.ts");

fn error(error: OperatorError) -> JsValue {
    let value = js_sys::Error::new(&error.to_string());
    let (code, ambiguous) = match &error {
        OperatorError::Stale => ("stale", false),
        OperatorError::SourceBusy { run_id } => {
            let _ = Reflect::set(&value, &"run_id".into(), &run_id.clone().into());
            ("source_busy", false)
        }
        OperatorError::Invalid => ("invalid", false),
        OperatorError::NotFound => ("not_found", false),
        OperatorError::Forbidden => ("forbidden", false),
        OperatorError::Unavailable { ambiguous } => ("unavailable", *ambiguous),
        OperatorError::Cancelled { ambiguous } => ("cancelled", *ambiguous),
        OperatorError::Deadline { ambiguous } => ("deadline", *ambiguous),
        OperatorError::InvalidResponse { ambiguous } => ("invalid_response", *ambiguous),
        OperatorError::Connection { code, ambiguous } => (code.as_str(), *ambiguous),
    };
    let _ = Reflect::set(&value, &"code".into(), &code.into());
    let _ = Reflect::set(&value, &"ambiguous".into(), &ambiguous.into());
    value.into()
}
fn from<T: serde::de::DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|_| error(OperatorError::Invalid))
}
fn to<T: serde::Serialize>(value: Result<T, OperatorError>) -> Result<JsValue, JsValue> {
    value
        .map_err(error)?
        .serialize(
            &serde_wasm_bindgen::Serializer::json_compatible()
                .serialize_large_number_types_as_bigints(true),
        )
        .map_err(|_| error(OperatorError::InvalidResponse { ambiguous: false }))
}

#[wasm_bindgen(js_name = OperatorClient)]
pub struct WasmOperatorClient {
    inner: Arc<OperatorClient>,
}
#[wasm_bindgen(js_class = OperatorClient)]
impl WasmOperatorClient {
    #[wasm_bindgen(constructor)]
    pub fn new(
        #[wasm_bindgen(unchecked_param_type = "OperatorClientConfig")] config: JsValue,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            inner: OperatorClient::new(from(config)?).map_err(error)?,
        })
    }
    pub fn cancel_all(&self) {
        self.inner.cancel_all();
    }
    pub fn configuration_namespace(&self) -> String {
        self.inner.configuration_namespace()
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorClientConfig")]
    pub fn configuration(&self) -> Result<JsValue, JsValue> {
        to(Ok(self.inner.configuration()))
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorHealth")]
    pub async fn health(&self) -> Result<JsValue, JsValue> {
        to(self.inner.health().await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorSourcePage")]
    pub async fn sources(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.sources(from(page)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorLibraryPage")]
    pub async fn libraries(
        &self,
        source_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.libraries(source_id, from(page)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRunPage")]
    pub async fn indexing_runs(
        &self,
        source_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_runs(source_id, from(page)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorCandidatePage")]
    pub async fn indexing_candidates(
        &self,
        source_id: String,
        run_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
        pending_only: bool,
    ) -> Result<JsValue, JsValue> {
        to(self
            .inner
            .indexing_candidates(source_id, run_id, from(page)?, pending_only)
            .await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorConnectionDiscovery")]
    pub async fn discover_connection(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OperatorDiscoverConnectionRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.discover_connection(from(input)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorSavedConnection")]
    pub async fn save_connection(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OperatorSaveConnectionRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.save_connection(from(input)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorConnectionPage")]
    pub async fn connections(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.connections(from(page)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorIndexingPolicyPage")]
    pub async fn indexing_policies(
        &self,
        source_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_policies(source_id, from(page)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorIndexingPolicy")]
    pub async fn indexing_configure(
        &self,
        source_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorIndexingConfigureRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        let configuration = Reflect::get(&input, &"configuration".into())
            .map_err(|_| error(OperatorError::Invalid))?;
        if Reflect::has(&configuration, &"operator_profile_id".into()).unwrap_or(true) {
            return Err(error(OperatorError::Invalid));
        }
        to(self.inner.indexing_configure(source_id, from(input)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorIndexingProgress | null")]
    pub async fn indexing_progress(
        &self,
        source_id: String,
        policy_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_progress(source_id, policy_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRun")]
    pub async fn indexing_start(
        &self,
        source_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorStartRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_start(source_id, from(input)?).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRunStatus")]
    pub async fn indexing_run_status(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_run_status(source_id, run_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorIndexingExecution | null")]
    pub async fn indexing_execution(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_execution(source_id, run_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRunStatus")]
    pub async fn indexing_pause(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_pause(source_id, run_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRunStatus")]
    pub async fn indexing_resume(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_resume(source_id, run_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRunStatus")]
    pub async fn indexing_cancel(
        &self,
        source_id: String,
        run_id: String,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_cancel(source_id, run_id).await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorJobPage")]
    pub async fn indexing_jobs(
        &self,
        source_id: String,
        run_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self
            .inner
            .indexing_jobs(source_id, run_id, from(page)?)
            .await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorAttemptPage")]
    pub async fn indexing_attempts(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self
            .inner
            .indexing_attempts(source_id, run_id, job_id, from(page)?)
            .await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRedrivePage")]
    pub async fn indexing_redrives(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorPage")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self
            .inner
            .indexing_redrives(source_id, run_id, job_id, from(page)?)
            .await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorRedrive")]
    pub async fn indexing_redrive(
        &self,
        source_id: String,
        run_id: String,
        job_id: String,
        #[wasm_bindgen(unchecked_param_type = "OperatorRedriveRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self
            .inner
            .indexing_redrive(source_id, run_id, job_id, from(input)?)
            .await)
    }
    #[wasm_bindgen(unchecked_return_type = "OperatorQueueDiagnostics")]
    pub async fn indexing_queue(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OperatorDiagnosticsRequest")] input: JsValue,
    ) -> Result<JsValue, JsValue> {
        to(self.inner.indexing_queue(from(input)?).await)
    }
}

fn get(object: &JsValue, field: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &field.into())
}
fn invoke(object: &JsValue, method: &str, args: &Array) -> Result<JsValue, JsValue> {
    get(object, method)?
        .dyn_into::<Function>()?
        .apply(object, args)
}
async fn promise(value: JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(value.dyn_into::<Promise>()?).await
}
struct Abort(JsValue);
impl Drop for Abort {
    fn drop(&mut self) {
        let _ = invoke(&self.0, "abort", &Array::new());
    }
}

pub(crate) async fn fetch(
    url: &str,
    body: Option<Vec<u8>>,
    ambiguous: bool,
) -> Result<(u16, Vec<u8>), OperatorError> {
    let result: Result<_, JsValue> = async {
        let global = js_sys::global();
        let constructor = get(&global, "AbortController")?.dyn_into::<Function>()?;
        let abort = Abort(Reflect::construct(&constructor, &Array::new())?);
        let init = Object::new();
        for (name, value) in [
            ("redirect", "error".into()),
            ("mode", "same-origin".into()),
            ("credentials", "omit".into()),
            ("cache", "no-store".into()),
            ("signal", get(&abort.0, "signal")?),
            ("method", if body.is_some() { "POST" } else { "GET" }.into()),
        ] {
            Reflect::set(&init, &name.into(), &value)?;
        }
        let headers = Object::new();
        Reflect::set(&headers, &"accept".into(), &"application/json".into())?;
        if let Some(body) = body {
            Reflect::set(&headers, &"content-type".into(), &"application/json".into())?;
            Reflect::set(&init, &"body".into(), &Uint8Array::from(body.as_slice()))?;
        }
        Reflect::set(&init, &"headers".into(), &headers)?;
        let args = Array::new();
        args.push(&url.into());
        args.push(&init);
        let response = promise(invoke(&global, "fetch", &args)?).await?;
        let status = get(&response, "status")?.as_f64().unwrap_or(0.0) as u16;
        let body = get(&response, "body")?;
        if body.is_null() || body.is_undefined() {
            return Ok(Ok((status, Vec::new())));
        }
        let reader = invoke(&body, "getReader", &Array::new())?;
        let mut bytes = Vec::new();
        loop {
            let chunk = promise(invoke(&reader, "read", &Array::new())?).await?;
            if get(&chunk, "done")?.as_bool() == Some(true) {
                break;
            }
            let value = Uint8Array::new(&get(&chunk, "value")?);
            if value.length() as usize > crate::operator::RESPONSE_LIMIT - bytes.len() {
                return Ok(Err(OperatorError::InvalidResponse { ambiguous }));
            }
            bytes.extend(value.to_vec());
        }
        Ok(Ok((status, bytes)))
    }
    .await;
    result.map_err(|_| OperatorError::Unavailable { ambiguous })?
}
