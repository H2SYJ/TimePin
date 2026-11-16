use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 插件清单（版本 1）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub entry: String,
    pub runtime: PluginRuntime,
    pub capabilities: Vec<PluginCapability>,
    pub parameters_schema: HashMap<String, serde_json::Value>,
    pub output_schema: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntime {
    Python,
    Rust,
    Node,
    Binary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginCapability {
    VideoFrames,
    AudioSamples,
    Metadata,
    Gpu,
    Network,
    FileWrite,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            protocol_version: "1.0.0".to_string(),
            name: String::new(),
            version: "0.1.0".to_string(),
            description: String::new(),
            author: String::new(),
            entry: "main.py".to_string(),
            runtime: PluginRuntime::Python,
            capabilities: Vec::new(),
            parameters_schema: HashMap::new(),
            output_schema: HashMap::new(),
        }
    }
}