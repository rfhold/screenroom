use super::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const DOMAIN_TYPES: &str = include_str!("wasm-models.ts");

#[wasm_bindgen(typescript_custom_section)]
const ERROR_TYPES: &str = "export interface ClientFailure extends Error { code: 'invalid_argument' | 'not_found' | 'unavailable' | 'deadline_exceeded' | 'cancelled' | 'resource_exhausted' | 'rpc' | 'invalid_response'; grpc_code?: number; }";

fn js_error(error: ClientError) -> JsValue {
    let value = js_sys::Error::new(&error.to_string());
    if let ClientError::Rpc { code, .. } = &error {
        let _ = js_sys::Reflect::set(&value, &"grpc_code".into(), &JsValue::from(*code));
    }
    let code = match error {
        ClientError::InvalidArgument { .. } => "invalid_argument",
        ClientError::NotFound { .. } => "not_found",
        ClientError::Unavailable { .. } => "unavailable",
        ClientError::DeadlineExceeded => "deadline_exceeded",
        ClientError::Cancelled => "cancelled",
        ClientError::ResourceExhausted { .. } => "resource_exhausted",
        ClientError::Rpc { .. } => "rpc",
        ClientError::InvalidResponse { .. } => "invalid_response",
    };
    let _ = js_sys::Reflect::set(&value, &"code".into(), &code.into());
    value.into()
}
fn from_js<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|e| js_error(invalid(e.to_string())))
}
fn to_js<T: Serialize>(value: T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| {
            js_error(ClientError::InvalidResponse {
                reason: e.to_string(),
            })
        })
}

#[wasm_bindgen(js_name=Client)]
pub struct WasmClient {
    inner: Arc<Client>,
}
#[wasm_bindgen(js_class=Client)]
impl WasmClient {
    #[wasm_bindgen(constructor)]
    pub fn new(
        #[wasm_bindgen(unchecked_param_type = "ClientConfig")] config: JsValue,
    ) -> Result<WasmClient, JsValue> {
        Ok(Self {
            inner: Client::new(from_js(config)?).map_err(js_error)?,
        })
    }
    pub fn cancel_all(&self) {
        self.inner.cancel_all();
    }
    pub fn invalidate_reads(&self) {
        self.inner.invalidate_reads();
    }
    #[wasm_bindgen(unchecked_return_type = "HomeRow[]")]
    pub async fn get_home(&self, library_id: String) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .get_home(library_id)
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "MediaLibrary[]")]
    pub async fn list_libraries(&self) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .list_libraries()
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "ItemPage")]
    pub async fn list_library_items(
        &self,
        library_id: String,
        #[wasm_bindgen(unchecked_param_type = "Page")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .list_library_items(library_id, from_js(page)?)
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "Item")]
    pub async fn get_item(&self, id: String) -> Result<JsValue, JsValue> {
        to_js(self.inner.clone().get_item(id).await.map_err(js_error)?)
    }
    #[wasm_bindgen(unchecked_return_type = "CatalogInfo")]
    pub async fn get_catalog_info(&self) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .get_catalog_info()
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "ItemPage")]
    pub async fn list_children(
        &self,
        parent_id: String,
        #[wasm_bindgen(unchecked_param_type = "Page")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .list_children(parent_id, from_js(page)?)
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "CreditPage")]
    pub async fn list_credits(
        &self,
        item_id: String,
        #[wasm_bindgen(unchecked_param_type = "Page")] page: JsValue,
    ) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .list_credits(item_id, from_js(page)?)
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "ItemPage")]
    pub async fn search(
        &self,
        #[wasm_bindgen(unchecked_param_type = "SearchRequest")] request: JsValue,
    ) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .search(from_js(request)?)
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "SearchCapabilities")]
    pub async fn get_search_capabilities(&self) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .get_search_capabilities()
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "Preferences")]
    pub async fn get_preferences(&self) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .get_preferences()
                .await
                .map_err(js_error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "Preferences")]
    pub async fn update_preferences(
        &self,
        #[wasm_bindgen(unchecked_param_type = "Preferences")] preferences: JsValue,
        #[wasm_bindgen(unchecked_param_type = "string[]")] update_mask: JsValue,
    ) -> Result<JsValue, JsValue> {
        to_js(
            self.inner
                .clone()
                .update_preferences(from_js(preferences)?, from_js(update_mask)?)
                .await
                .map_err(js_error)?,
        )
    }
    pub async fn load_artwork(&self, url: String) -> Result<Vec<u8>, JsValue> {
        self.inner.clone().load_artwork(url).await.map_err(js_error)
    }
}
