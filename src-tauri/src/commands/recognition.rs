use crate::plugins::recognition::{
    ConfigureParams, DetectParams, OcrParams, PreviewParams, RecognitionProcess, RoiListResponse,
    RoiLoadParams, RoiLoadResponse, RoiSaveParams, RoiSaveResponse, TemplateMatchParams,
    TemplateMatchResponse,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

/// 识别服务状态
pub struct RecognitionState {
    process: Arc<Mutex<Option<RecognitionProcess>>>,
    service_dir: PathBuf,
}

impl RecognitionState {
    pub fn new(app: &AppHandle) -> Self {
        // 识别服务目录（开发模式：仓库根 plugins/recognition；生产模式：resource_dir/plugins/recognition）
        let mut service_dir = None;

        // 开发模式
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors().take(4) {
                let candidate = ancestor.join("plugins").join("recognition");
                if candidate.is_dir() {
                    service_dir = Some(candidate);
                    break;
                }
            }
        }

        // 生产模式
        if service_dir.is_none() {
            if let Ok(resource) = app.path().resource_dir() {
                let candidate = resource.join("plugins").join("recognition");
                if candidate.is_dir() {
                    service_dir = Some(candidate);
                }
            }
        }

        Self {
            process: Arc::new(Mutex::new(None)),
            service_dir: service_dir.unwrap_or_else(|| PathBuf::from("plugins/recognition")),
        }
    }
}

/// 确保识别服务进程已启动
fn ensure_recognition_process(state: &State<RecognitionState>) -> Result<(), String> {
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    if process_guard.is_none() {
        let service_dir = state.service_dir.clone();
        let process = RecognitionProcess::spawn(&service_dir, None)
            .map_err(|e| format!("启动识别服务失败: {e}"))?;
        *process_guard = Some(process);
    }
    Ok(())
}

#[tauri::command]
pub async fn recognition_template_match(
    state: State<'_, RecognitionState>,
    params: TemplateMatchParams,
) -> Result<TemplateMatchResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process
        .template_match(params, None)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_ocr(
    state: State<'_, RecognitionState>,
    params: OcrParams,
) -> Result<crate::plugins::recognition::OcrResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.ocr(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_detect(
    state: State<'_, RecognitionState>,
    params: DetectParams,
) -> Result<crate::plugins::recognition::DetectResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.detect(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_preview(
    state: State<'_, RecognitionState>,
    params: PreviewParams,
) -> Result<crate::plugins::protocol::PreviewFrame, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.preview(params, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_save(
    state: State<'_, RecognitionState>,
    params: RoiSaveParams,
) -> Result<RoiSaveResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_save(params).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_load(
    state: State<'_, RecognitionState>,
    params: RoiLoadParams,
) -> Result<RoiLoadResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_load(params).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_roi_list(
    state: State<'_, RecognitionState>,
) -> Result<RoiListResponse, String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.roi_list().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recognition_configure(
    state: State<'_, RecognitionState>,
    params: ConfigureParams,
) -> Result<(), String> {
    ensure_recognition_process(&state)?;
    let mut process_guard = state.process.lock().map_err(|e| e.to_string())?;
    let process = process_guard.as_mut().ok_or("识别服务未启动")?;
    process.configure(params).map_err(|e| e.to_string())
}
