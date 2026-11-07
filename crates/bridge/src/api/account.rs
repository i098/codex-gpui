//! Sign-in and Bedrock requests: `account/*` plus the config reads the sign-in policy needs.

use anyhow::Result;
use serde_json::{Value, json};

use crate::Bridge;

impl Bridge {
    pub async fn config_requirements(&self) -> Result<Value> {
        self.request("configRequirements/read", json!({})).await
    }

    /// `config/read` of the merged config without layers, for the default cwd.
    pub async fn config_read_merged(&self) -> Result<Value> {
        self.request("config/read", json!({"includeLayers": false, "cwd": null}))
            .await
    }

    pub async fn gateway_oauth_read(&self) -> Result<Value> {
        self.request("account/gatewayOAuth/read", Option::<()>::None)
            .await
    }

    pub async fn gateway_oauth_login(&self) -> Result<Value> {
        self.request("account/gatewayOAuth/login", Option::<()>::None)
            .await
    }

    pub async fn gateway_oauth_cancel(&self) -> Result<Value> {
        self.request("account/gatewayOAuth/cancel", Option::<()>::None)
            .await
    }

    /// Browser sign-in; the response carries `loginId` and `authUrl`.
    pub async fn login_start_chatgpt(&self) -> Result<Value> {
        self.login_start(json!({
            "type": "chatgpt", "appBrand": "chatgpt", "useHostedLoginSuccessPage": true,
        }))
        .await
    }

    /// One-time-code sign-in; the response carries `loginId`, `verificationUrl`, `userCode`.
    pub async fn login_start_device_code(&self) -> Result<Value> {
        self.login_start(json!({"type": "chatgptDeviceCode"})).await
    }

    pub async fn login_start_api_key(&self, key: &str) -> Result<Value> {
        self.login_start(json!({"type": "apiKey", "apiKey": key}))
            .await
    }

    pub async fn login_start_bedrock_api_key(&self, api_key: &str, region: &str) -> Result<Value> {
        self.login_start(json!({"type": "amazonBedrock", "apiKey": api_key, "region": region}))
            .await
    }

    pub async fn login_start_bedrock_access_keys(
        &self,
        access_key_id: &str,
        secret_access_key: &str,
        session_token: Option<&str>,
        region: &str,
    ) -> Result<Value> {
        self.login_start(json!({
            "type": "amazonBedrockAccessKeys",
            "accessKeyId": access_key_id,
            "secretAccessKey": secret_access_key,
            "sessionToken": session_token,
            "region": region,
        }))
        .await
    }

    async fn login_start(&self, params: Value) -> Result<Value> {
        self.request("account/login/start", params).await
    }

    pub async fn login_cancel(&self, login_id: &str) -> Result<Value> {
        self.request("account/login/cancel", json!({"loginId": login_id}))
            .await
    }

    pub async fn bedrock_discover(&self) -> Result<Value> {
        self.request("account/bedrock/discover", json!({})).await
    }

    pub async fn bedrock_setup_profile(&self, profile: &str, region: &str) -> Result<Value> {
        self.request(
            "account/bedrock/setup",
            json!({"type": "profile", "profile": profile, "region": region}),
        )
        .await
    }

    pub async fn bedrock_setup_environment(&self, region: &str) -> Result<Value> {
        self.request(
            "account/bedrock/setup",
            json!({"type": "environment", "region": region}),
        )
        .await
    }

    pub async fn bedrock_check_govcloud(&self) -> Result<Value> {
        self.request("account/bedrock/checkGovCloudRequirements", json!({}))
            .await
    }
}
