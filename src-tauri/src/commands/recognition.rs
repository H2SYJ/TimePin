use crate::plugins::manifest::PluginManifest;
use crate::plugins::recognition::{
    ConfigureParams, DetectParams, OcrParams, PreviewParams, RecognitionProcess, RoiListResponse,
    RoiLoadParams, RoiLoadResponse, RoiSaveParams, RoiSaveResponse, TemplateMatchParams,
    TemplateMatchResponse,
};
use crate::plugins::{manifest::PluginRuntime, python_env};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

/// 识别服务状态
pub struct RecognitionState {
    process: Arc<Mutex<Option<RecognitionProcess>>>,
    service_dir: PathBuf,
}

impl RecognitionState {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            process: Arc::new(Mutex::new(None)),
            service_dir: crate::core::get_plugin_dir(app).join("recognition"),
        }
    }
}

/// 识别服务的插件清单——与 `plugins/recognition/plugin.json` 保持一致。
/// 由于识别服务不走普通插件发现流程（服务目录独立），这里手工构造 manifest，
/// 使其能走同一套 Python 环境解析与 venv 建置。
fn recognition_manifest(service_dir: &std::path::Path) -> PluginManifest {
    // 优先从磁盘读 plugin.json（保持字段与实际清单一致），失败则回退默认声明。
    PluginManifest::load(service_dir).unwrap_or_else(|_| PluginManifest {
        protocol_version: crate::plugins::PROTOCOL_VERSION.to_string(),
        name: "timepin-recognition".to_string(),
        version: "0.1.0".to_string(),
        description: crate::plugins::manifest::LocalizedText::Text("内置识别服务".to_string()),
        author: "TimePin".to_string(),
        entry: "main.py".to_string(),
        runtime: PluginRuntime::Python,
        capabilities: Vec::new(),
        parameters_schema: Default::default(),
        output_schema: Default::default(),
        python: None,
        dependencies: Vec::new(),
    })
}

/// 确保识别服务进程已启动；解释器先经共享解析器/venv 建置。
fn ensure_recognition_process(
    app: &AppHandle,
    state: &State<RecognitionState>,
) -> Result<(), String> {
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    if process_guard.is_none() {
        let service_dir = state.service_dir.clone();
        let manifest = recognition_manifest(&service_dir);
        let settings =
            crate::core::settings::load_settings(app).map_err(|e| format!("读取设置失败：{e}"))?;
        let interpreter =
            python_env::ensure_python_environment(&manifest, &service_dir, app, &settings)
                .map_err(|e| format!("识别服务 Python 环境准备失败：{e}"))?;
        let process = RecognitionProcess::spawn(&service_dir, interpreter, None)
            .map_err(|e| format!("启动识别服务失败: {e}"))?;
        *process_guard = Some(process);
    }
    Ok(())
}

#[tauri::command]
pub async fn recognition_template_match(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: TemplateMatchParams,
) -> Result<TemplateMatchResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process
        .template_match(params, None)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_ocr(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: OcrParams,
) -> Result<crate::plugins::recognition::OcrResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.ocr(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_detect(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: DetectParams,
) -> Result<crate::plugins::recognition::DetectResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.detect(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_preview(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: PreviewParams,
) -> Result<crate::plugins::protocol::PreviewFrame, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.preview(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_save(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: RoiSaveParams,
) -> Result<RoiSaveResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_save(params).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_load(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: RoiLoadParams,
) -> Result<RoiLoadResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_load(params).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_list(
    app: AppHandle,
    state: State<'_, RecognitionState>,
) -> Result<RoiListResponse, String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_list().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_configure(
    app: AppHandle,
    state: State<'_, RecognitionState>,
    params: ConfigureParams,
) -> Result<(), String> {
    ensure_recognition_process(&app, &state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.configure(params).map_err(|e| e.to_string())
}
