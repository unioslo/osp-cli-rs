use serde::{Deserialize, Serialize};

pub const PLUGIN_PROTOCOL_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeV1 {
    pub protocol_version: u32,
    pub plugin_id: String,
    pub plugin_version: String,
    pub min_osp_version: Option<String>,
    pub commands: Vec<DescribeCommandV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeCommandV1 {
    pub name: String,
    pub about: String,
    #[serde(default)]
    pub subcommands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseV1 {
    pub protocol_version: u32,
    pub ok: bool,
    pub data: serde_json::Value,
    pub error: Option<ResponseErrorV1>,
    pub meta: ResponseMetaV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseErrorV1 {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseMetaV1 {
    pub format_hint: Option<String>,
    pub columns: Option<Vec<String>>,
}

impl DescribeV1 {
    pub fn validate_v1(&self) -> Result<(), String> {
        if self.protocol_version != PLUGIN_PROTOCOL_V1 {
            return Err(format!(
                "unsupported describe protocol version: {}",
                self.protocol_version
            ));
        }
        if self.plugin_id.trim().is_empty() {
            return Err("plugin_id must not be empty".to_string());
        }
        Ok(())
    }
}

impl ResponseV1 {
    pub fn validate_v1(&self) -> Result<(), String> {
        if self.protocol_version != PLUGIN_PROTOCOL_V1 {
            return Err(format!(
                "unsupported response protocol version: {}",
                self.protocol_version
            ));
        }
        if self.ok && self.error.is_some() {
            return Err("ok=true requires error=null".to_string());
        }
        if !self.ok && self.error.is_none() {
            return Err("ok=false requires error payload".to_string());
        }
        Ok(())
    }
}
