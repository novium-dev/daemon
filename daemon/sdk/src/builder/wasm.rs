#[cfg(feature = "user")]
use crate::builder::user::*;
use crate::builder::{
    DaemonSDKBuilder, DaemonSDKBuilderBuildable, DaemonSDKBuilderRequiresBaseURL,
    DaemonSDKBuilderRequiresIdentifier, DaemonSDKBuilderRequiresToken,
};
use crate::*;
use crate::{DaemonSDK, DaemonSDKError, DefaultDaemonSDK};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
#[cfg(feature = "user")]
use ed25519_dalek::pkcs8::{DecodePrivateKey, KeypairBytes};
#[cfg(feature = "user")]
use ed25519_dalek::SigningKey;
use reqwest::Url;

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
        DaemonSDKBuilderRequiresToken { identifier }
    }
}

#[wasm_bindgen]
impl DaemonSDKBuilderRequiresToken {
    #[wasm_bindgen(js_name = with_token)]
    pub fn wasm_with_token(self, token: String) -> DaemonSDKBuilderRequiresBaseURL {
        DaemonSDKBuilderRequiresBaseURL {
            identifier: self.identifier,
            token,
        }
    }
}

#[wasm_bindgen]
impl DaemonSDKBuilderRequiresBaseURL {
    #[wasm_bindgen(js_name = with_base_url)]
    pub fn wasm_with_base_url(
        self,
        base_url: String,
    ) -> Result<DaemonSDKBuilderBuildable, DaemonSDKError> {
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

#[cfg(feature = "user")]
#[wasm_bindgen]
impl DaemonSDKBuilderBuildable {
    #[wasm_bindgen(js_name = with_user_key_from_pem)]
    pub fn wasm_with_user_key_from_pem(
        self,
        key: String,
    ) -> Result<DaemonSDKBuilderWithUserKey, DaemonSDKError> {
        let bytes =
            KeypairBytes::from_pkcs8_pem(&key).map_err(|_| DaemonSDKError::InvalidWebsocketKey)?;
        self.wasm_with_user_key(bytes.secret_key.to_vec())
    }

    #[wasm_bindgen(js_name = with_user_key_from_base64)]
    pub fn wasm_with_user_key_from_base64(
        self,
        key: String,
    ) -> Result<DaemonSDKBuilderWithUserKey, DaemonSDKError> {
        let mut vec = Vec::with_capacity(32);
        BASE64_STANDARD
            .decode_vec(key, &mut vec)
            .map_err(|_| DaemonSDKError::InvalidWebsocketKey)?;
        self.wasm_with_user_key(vec)
    }

    #[wasm_bindgen(js_name = with_user_key)]
    pub fn wasm_with_user_key(
        self,
        key: Vec<u8>,
    ) -> Result<DaemonSDKBuilderWithUserKey, DaemonSDKError> {
        Ok(DaemonSDKBuilderWithUserKey {
            identifier: self.identifier,
            token: self.token,
            base_url: self.base_url,
            user_key: SigningKey::from_bytes(
                &key.try_into()
                    .map_err(|_| DaemonSDKError::InvalidWebsocketKey)?,
            ),
        })
    }
}

#[cfg(feature = "user")]
#[wasm_bindgen]
impl DaemonSDKBuilderWithUserKey {
    #[wasm_bindgen(js_name = build)]
    pub fn wasm_build(self) -> DaemonSDKWithUserKey {
        DaemonSDKBuilder::<WithUserKey>(self).build().0
    }
}
