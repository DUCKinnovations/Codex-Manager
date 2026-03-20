use rfd::FileDialog;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use crate::app_storage::{
    read_account_import_contents_from_directory, read_account_import_contents_from_files,
};
use crate::commands::shared::rpc_call_in_background;
use crate::rpc_client::rpc_call;

const LANU_RESULTS_FILE_NAME: &str = "lanu_results.json";
const REGISTER_SCRIPT_FILE_NAME: &str = "http_register_v2.py";
const ACCOUNT_REGISTER_DIR_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_DIR";
const ACCOUNT_REGISTER_FILE_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_FILE";
const ACCOUNT_REGISTER_SCRIPT_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_SCRIPT";
const ACCOUNT_REGISTER_PYTHON_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_PYTHON";

fn normalize_path_candidate(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(path)
}

fn collect_account_register_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_DIR_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            dirs.push(path);
        }
    }
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_FILE_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            if let Some(parent) = path.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_SCRIPT_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            if let Some(parent) = path.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent().and_then(|value| value.parent()) {
        dirs.push(repo_root.join("scripts").join("account-register"));
        dirs.push(repo_root.join("scripts").join("账号注册"));
        if let Some(parent_dir) = repo_root.parent() {
            dirs.push(parent_dir.join("账号注册"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        dirs.push(current_dir.join("scripts").join("account-register"));
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        let key = dir.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(dir);
        }
    }
    deduped
}

fn collect_result_file_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_FILE_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            candidates.push(path);
        }
    }
    for dir in collect_account_register_base_dirs() {
        candidates.push(dir.join(LANU_RESULTS_FILE_NAME));
    }
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        let key = candidate.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn collect_script_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_SCRIPT_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            candidates.push(path);
        }
    }
    for dir in collect_account_register_base_dirs() {
        candidates.push(dir.join(REGISTER_SCRIPT_FILE_NAME));
    }
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        let key = candidate.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn resolve_lanu_results_path() -> Result<PathBuf, String> {
    let deduped = collect_result_file_candidates();

    for candidate in &deduped {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }

    let checked_paths = deduped
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" | ");

    Err(format!(
        "未找到 {LANU_RESULTS_FILE_NAME}，请先执行 http_register_v2.py 生成该文件。已检查: {checked_paths}"
    ))
}

fn resolve_register_script_path() -> Result<PathBuf, String> {
    let candidates = collect_script_candidates();
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    let checked_paths = candidates
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" | ");
    Err(format!(
        "未找到 {REGISTER_SCRIPT_FILE_NAME}。请通过 {ACCOUNT_REGISTER_SCRIPT_ENV} 指定脚本路径，或设置 {ACCOUNT_REGISTER_DIR_ENV} 指向脚本目录。已检查: {checked_paths}"
    ))
}

fn output_tail(bytes: &[u8], limit: usize) -> String {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.chars().count() <= limit {
        return text;
    }
    let tail = text
        .chars()
        .rev()
        .take(limit)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("...(tail)\n{tail}")
}

fn run_register_script(script_path: &Path) -> Result<String, String> {
    let started_at = Instant::now();
    let python_override = std::env::var(ACCOUNT_REGISTER_PYTHON_ENV)
        .ok()
        .and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    let python_candidates = if let Some(value) = python_override {
        vec![value]
    } else {
        vec!["python3".to_string(), "python".to_string()]
    };
    let mut launch_errors = Vec::new();
    for python in python_candidates {
        let output = Command::new(&python)
            .arg(script_path)
            .current_dir(script_path.parent().unwrap_or_else(|| Path::new(".")))
            .output();
        match output {
            Ok(value) => {
                if value.status.success() {
                    log::info!(
                        "account register script finished: python={} script={} elapsed_ms={}",
                        python,
                        script_path.display(),
                        started_at.elapsed().as_millis()
                    );
                    return Ok(python);
                }
                let stderr = output_tail(&value.stderr, 600);
                let stdout = output_tail(&value.stdout, 300);
                return Err(format!(
                    "注册脚本执行失败 (python={python}, exit={:?}): {}\n{}",
                    value.status.code(),
                    stderr,
                    if stdout.is_empty() {
                        String::new()
                    } else {
                        format!("stdout: {stdout}")
                    }
                ));
            }
            Err(err) => {
                launch_errors.push(format!("{python}: {err}"));
            }
        }
    }
    Err(format!(
        "无法启动 Python 执行脚本 {}，请设置 {}。错误: {}",
        script_path.display(),
        ACCOUNT_REGISTER_PYTHON_ENV,
        launch_errors.join(" | ")
    ))
}

#[tauri::command]
pub async fn service_account_import(
    addr: Option<String>,
    contents: Option<Vec<String>>,
    content: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut payload_contents = contents.unwrap_or_default();
    if let Some(single) = content {
        if !single.trim().is_empty() {
            payload_contents.push(single);
        }
    }
    let params = serde_json::json!({ "contents": payload_contents });
    rpc_call_in_background("account/import", addr, Some(params)).await
}

#[tauri::command]
pub async fn service_account_import_by_directory(
    _addr: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let selected_dir = FileDialog::new()
            .set_title("选择账号导入目录")
            .pick_folder();
        let Some(dir_path) = selected_dir else {
            return Ok(serde_json::json!({
              "result": {
                "ok": true,
                "canceled": true
              }
            }));
        };

        let (json_files, contents) = read_account_import_contents_from_directory(&dir_path)?;
        Ok(serde_json::json!({
          "result": {
            "ok": true,
            "canceled": false,
            "directoryPath": dir_path.to_string_lossy().to_string(),
            "fileCount": json_files.len(),
            "contents": contents
          }
        }))
    })
    .await
    .map_err(|err| format!("service_account_import_by_directory task failed: {err}"))?
}

#[tauri::command]
pub async fn service_account_import_by_file(
    _addr: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let selected_files = FileDialog::new()
            .set_title("选择账号导入文件")
            .add_filter("账号文件", &["json", "txt"])
            .pick_files();
        let Some(file_paths) = selected_files else {
            return Ok(serde_json::json!({
              "result": {
                "ok": true,
                "canceled": true
              }
            }));
        };

        let contents = read_account_import_contents_from_files(&file_paths)?;
        Ok(serde_json::json!({
          "result": {
            "ok": true,
            "canceled": false,
            "filePaths": file_paths
              .iter()
              .map(|path| path.to_string_lossy().to_string())
              .collect::<Vec<_>>(),
            "fileCount": file_paths.len(),
            "contents": contents
          }
        }))
    })
    .await
    .map_err(|err| format!("service_account_import_by_file task failed: {err}"))?
}

#[tauri::command]
pub async fn service_account_import_lanu_results(
    addr: Option<String>,
) -> Result<serde_json::Value, String> {
    let (script_file, script_python, source_file, contents) =
        tauri::async_runtime::spawn_blocking(move || {
            let script_file = resolve_register_script_path()?;
            let script_python = run_register_script(&script_file)?;
            let source_file = resolve_lanu_results_path()?;
            let contents =
                read_account_import_contents_from_files(std::slice::from_ref(&source_file))?;
            if contents.is_empty() {
                return Err(format!("导入文件为空: {}", source_file.display()));
            }
            Ok((script_file, script_python, source_file, contents))
        })
        .await
        .map_err(|err| format!("service_account_import_lanu_results task failed: {err}"))??;

    let params = serde_json::json!({ "contents": contents });
    let mut import_result = rpc_call_in_background("account/import", addr, Some(params)).await?;
    if let Some(result) = import_result
        .get_mut("result")
        .and_then(|value| value.as_object_mut())
    {
        result.insert(
            "sourceFile".to_string(),
            serde_json::json!(source_file.to_string_lossy().to_string()),
        );
        result.insert(
            "scriptFile".to_string(),
            serde_json::json!(script_file.to_string_lossy().to_string()),
        );
        result.insert("scriptPython".to_string(), serde_json::json!(script_python));
        result.insert("fileCount".to_string(), serde_json::json!(1));
    }
    Ok(import_result)
}

#[tauri::command]
pub async fn service_account_export_by_account_files(
    addr: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let selected_dir = FileDialog::new()
            .set_title("选择账号导出目录")
            .pick_folder();
        let Some(dir_path) = selected_dir else {
            return Ok(serde_json::json!({
              "result": {
                "ok": true,
                "canceled": true
              }
            }));
        };
        let params = serde_json::json!({
          "outputDir": dir_path.to_string_lossy().to_string()
        });
        rpc_call("account/export", addr, Some(params))
    })
    .await
    .map_err(|err| format!("service_account_export_by_account_files task failed: {err}"))?
}
