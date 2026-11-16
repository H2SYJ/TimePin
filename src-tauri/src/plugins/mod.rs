//! 插件系统 - 为 M2 预留接口
//!
//! M1 阶段不实现具体插件，仅定义模块结构

pub mod manifest;
pub mod protocol;

/// 插件管理器（M2 实现）
pub struct PluginManager;

impl PluginManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}