use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use super::protocol::PROTOCOL_VERSION;

/// 插件清单（版本 1）
///
/// 每个插件目录下的 `plugin.json`（或 `manifest.json`）反序列化为此结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// 插件实现的协议版本（必须与主程序 major 兼容）
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// 入口文件：runtime=python/node 时为脚本路径，binary/rust 时为可执行文件
    pub entry: String,
    pub runtime: PluginRuntime,
    pub capabilities: Vec<PluginCapability>,
    /// 参数 JSON Schema（properties 部分），UI 据此渲染参数表单
    #[serde(default)]
    pub parameters_schema: HashMap<String, serde_json::Value>,
    /// 结果 JSON Schema（文档用途为主，运行时宽松校验）
    #[serde(default)]
    pub output_schema: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntime {
    Python,
    Rust,
    Node,
    Binary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum PluginCapability {
    VideoFrames,
    AudioSamples,
    Metadata,
    Gpu,
    Network,
    FileWrite,
}

impl PluginManifest {
    /// 校验清单字段；返回错误列表（不短路，便于一次性报全）。
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.protocol_version.trim().is_empty() {
            errors.push("protocolVersion 不能为空".to_string());
        } else if !super::runtime::versions_compatible(PROTOCOL_VERSION, &self.protocol_version) {
            errors.push(format!(
                "协议版本不兼容：主程序 {PROTOCOL_VERSION}，插件 {}",
                self.protocol_version
            ));
        }
        if self.name.trim().is_empty() {
            errors.push("name 不能为空".to_string());
        } else if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            errors.push("name 只允许 ASCII 字母数字、`-`、`_`".to_string());
        }
        if self.version.trim().is_empty() {
            errors.push("version 不能为空".to_string());
        }
        if self.entry.trim().is_empty() {
            errors.push("entry 不能为空".to_string());
        } else {
            let entry = Path::new(&self.entry);
            if entry.is_absolute() || self.entry.contains("..") {
                errors.push("entry 必须是插件目录内的相对路径，不允许绝对路径或 `..`".to_string());
            }
        }
        errors
    }

    /// 从插件目录加载清单（查找 `plugin.json` 或 `manifest.json`）。
    pub fn load(plugin_dir: &Path) -> Result<Self, String> {
        for name in ["plugin.json", "manifest.json"] {
            let path = plugin_dir.join(name);
            if path.is_file() {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
                let manifest: Self = serde_json::from_str(&content)
                    .map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
                return Ok(manifest);
            }
        }
        Err(format!(
            "{} 中未找到 plugin.json / manifest.json",
            plugin_dir.display()
        ))
    }
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> PluginManifest {
        PluginManifest {
            name: "demo-plugin".to_string(),
            entry: "main.py".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn valid_manifest_passes() {
        assert!(valid().validate().is_empty());
    }

    #[test]
    fn rejects_bad_entry_paths() {
        let mut m = valid();
        m.entry = "../escape.py".to_string();
        assert!(!m.validate().is_empty());
        m.entry = "C:/abs/evil.py".to_string();
        assert!(!m.validate().is_empty());
    }

    #[test]
    fn rejects_incompatible_protocol() {
        let mut m = valid();
        m.protocol_version = "2.0.0".to_string();
        assert!(m.validate().iter().any(|e| e.contains("不兼容")));
    }

    #[test]
    fn rejects_bad_name() {
        let mut m = valid();
        m.name = "bad name!".to_string();
        assert!(!m.validate().is_empty());
    }
}
