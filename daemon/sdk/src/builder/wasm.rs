use crate::*;
use reqwest::Url;
use crate::builder::{DaemonSDKBuilder, DaemonSDKBuilderBuildable, DaemonSDKBuilderRequiresBaseURL, DaemonSDKBuilderRequiresIdentifier, DaemonSDKBuilderRequiresToken};
use crate::{DaemonSDK, DaemonSDKError, DefaultDaemonSDK};

#[wasm_bindgen]
impl DefaultDaemonSDK {

    #[wasm_bindgen(js_name = builder)]
    pub fn wasm_builder() -> DaemonSDKBuilderRequiresIdentifier {
        DaemonSDK::<()>::builder().0
    }
}

#[wasm_bindgen]
impl DaemonSDKBuilderRequiresIdentifier {

    #[wasm_bindgen(js_name = with_identifier)]
    pub fn wasm_with_identifier(self, identifier: String) -> DaemonSDKBuilderRequiresToken {
        DaemonSDKBuilderRequiresToken {
            identifier
        }
    }

}

#[wasm_bindgen]
impl DaemonSDKBuilderRequiresToken {

    #[wasm_bindgen(js_name = with_token)]
    pub fn wasm_with_token(self, token: String) -> DaemonSDKBuilderRequiresBaseURL {
        DaemonSDKBuilderRequiresBaseURL {
            identifier: self.identifier,
            token
        }
    }

}

#[wasm_bindgen]
impl DaemonSDKBuilderRequiresBaseURL {

    #[wasm_bindgen(js_name = with_base_url)]
    pub fn wasm_with_base_url(self, base_url: String) -> Result<DaemonSDKBuilderBuildable, DaemonSDKError> {
        Ok(DaemonSDKBuilderBuildable {
            identifier: self.identifier,
            token: self.token,
            base_url: Url::parse(&base_url).map_err(DaemonSDKError::URLParseError)?,
        })
    }

}

#[wasm_bindgen]
impl DaemonSDKBuilderBuildable {

    #[wasm_bindgen(js_name = build)]
    pub fn wasm_build(self) -> DefaultDaemonSDK {
        DaemonSDKBuilder::<()>(self).build().0
    }

}