use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

const LANU_RESULTS_FILE_NAME: &str = "lanu_results.json";
const REGISTER_SCRIPT_FILE_NAME: &str = "http_register_v2.py";
const ACCOUNT_REGISTER_DIR_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_DIR";
const ACCOUNT_REGISTER_FILE_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_FILE";
const ACCOUNT_REGISTER_SCRIPT_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_SCRIPT";
const ACCOUNT_REGISTER_PYTHON_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_PYTHON";
const ACCOUNT_REGISTER_TXT_FILE_ENV: &str = "CODEXMANAGER_ACCOUNT_REGISTER_TXT_FILE";
const DEFAULT_ACCOUNT_TXT_FILE_NAME: &str = "accounts-mailtb.txt";

#[derive(Debug, Default, Clone)]
struct LanuRunState {
    running: bool,
    script_file: String,
    account_txt_file: String,
    python: String,
    log: String,
    stderr: String,
    updated_at_ms: u128,
}

static LANU_RUN_STATE: OnceLock<Mutex<LanuRunState>> = OnceLock::new();

fn run_state() -> &'static Mutex<LanuRunState> {
    LANU_RUN_STATE.get_or_init(|| Mutex::new(LanuRunState::default()))
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn with_run_state<F>(f: F)
where
    F: FnOnce(&mut LanuRunState),
{
    let mutex = run_state();
    let mut guard = match mutex.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    f(&mut guard);
}

fn start_run_state(script_file: &Path, account_txt_file: Option<&Path>) {
    with_run_state(|state| {
        state.running = true;
        state.script_file = script_file.to_string_lossy().to_string();
        state.account_txt_file = account_txt_file
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_default();
        state.python.clear();
        state.log.clear();
        state.stderr.clear();
        state.updated_at_ms = now_millis();
    });
}

fn finish_run_state() {
    with_run_state(|state| {
        state.running = false;
        state.updated_at_ms = now_millis();
    });
}

fn append_run_log(is_stderr: bool, chunk: &str) {
    with_run_state(|state| {
        if is_stderr {
            state.stderr.push_str(chunk);
        } else {
            state.log.push_str(chunk);
        }
        state.updated_at_ms = now_millis();
    });
}

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

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(path);
        }
    }
    deduped
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

    if let Ok(current_dir) = std::env::current_dir() {
        dirs.push(current_dir.join("scripts").join("account-register"));
        dirs.push(current_dir.join("scripts").join("账号注册"));
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent().and_then(|value| value.parent()) {
        dirs.push(repo_root.join("scripts").join("account-register"));
        dirs.push(repo_root.join("scripts").join("账号注册"));
    }

    let exe_dir = crate::process_env::exe_dir();
    dirs.push(exe_dir.join("scripts").join("account-register"));
    dirs.push(exe_dir.join("scripts").join("账号注册"));
    if let Some(parent) = exe_dir.parent() {
        dirs.push(parent.join("scripts").join("account-register"));
        dirs.push(parent.join("scripts").join("账号注册"));
    }

    dedupe_paths(dirs)
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
    dedupe_paths(candidates)
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
    dedupe_paths(candidates)
}

fn collect_account_txt_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(raw) = std::env::var(ACCOUNT_REGISTER_TXT_FILE_ENV) {
        if let Some(path) = normalize_path_candidate(&raw) {
            candidates.push(path);
        }
    }
    for dir in collect_account_register_base_dirs() {
        candidates.push(dir.join(DEFAULT_ACCOUNT_TXT_FILE_NAME));
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let is_txt = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("txt"))
                    .unwrap_or(false);
                if !is_txt {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if name.contains("mailtb") {
                    candidates.push(path);
                }
            }
        }
    }
    dedupe_paths(candidates)
}

fn resolve_account_txt_path_for_read() -> Result<PathBuf, String> {
    let candidates = collect_account_txt_candidates();
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
        "未找到账号 txt 文件（mailtb*.txt）。可通过 {ACCOUNT_REGISTER_TXT_FILE_ENV} 指定路径。已检查: {checked_paths}"
    ))
}

fn resolve_account_txt_path_for_write(path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(raw) = path {
        if let Some(candidate) = normalize_path_candidate(raw) {
            if candidate.is_absolute() {
                return Ok(candidate);
            }
            if let Some(base_dir) = collect_account_register_base_dirs().into_iter().next() {
                return Ok(base_dir.join(candidate));
            }
            return std::env::current_dir()
                .map(|dir| dir.join(candidate))
                .map_err(|err| format!("获取当前目录失败: {err}"));
        }
    }
    if let Ok(found) = resolve_account_txt_path_for_read() {
        return Ok(found);
    }
    if let Some(base_dir) = collect_account_register_base_dirs().into_iter().next() {
        return Ok(base_dir.join(DEFAULT_ACCOUNT_TXT_FILE_NAME));
    }
    Err("无法确定账号 txt 保存路径，请设置 CODEXMANAGER_ACCOUNT_REGISTER_DIR".to_string())
}

pub(crate) fn get_lanu_account_txt_config() -> Result<serde_json::Value, String> {
    let file_path = resolve_account_txt_path_for_read()?;
    let content = std::fs::read_to_string(&file_path)
        .map_err(|err| format!("读取账号 txt 失败 ({}): {err}", file_path.display()))?;
    let line_count = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .count();
    Ok(serde_json::json!({
        "filePath": file_path.to_string_lossy().to_string(),
        "fileName": file_path.file_name().and_then(|v| v.to_str()).unwrap_or(""),
        "content": content,
        "lineCount": line_count,
    }))
}

pub(crate) fn save_lanu_account_txt_config(
    content: &str,
    file_path: Option<&str>,
) -> Result<serde_json::Value, String> {
    let target = resolve_account_txt_path_for_write(file_path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("创建目录失败 ({}): {err}", parent.display()))?;
    }
    std::fs::write(&target, content.as_bytes())
        .map_err(|err| format!("写入账号 txt 失败 ({}): {err}", target.display()))?;
    let line_count = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .count();
    Ok(serde_json::json!({
        "ok": true,
        "filePath": target.to_string_lossy().to_string(),
        "fileName": target.file_name().and_then(|v| v.to_str()).unwrap_or(""),
        "lineCount": line_count,
        "bytes": content.len(),
    }))
}

fn resolve_lanu_results_path() -> Result<PathBuf, String> {
    let candidates = collect_result_file_candidates();

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
        "未找到 {LANU_RESULTS_FILE_NAME}，请先执行 {REGISTER_SCRIPT_FILE_NAME} 生成该文件。已检查: {checked_paths}"
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

fn capture_output(bytes: &[u8], limit: usize) -> String {
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

/// Result of running the registration script, including captured output.
pub(crate) struct ScriptRunResult {
    pub python: String,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u128,
}

fn run_register_script(script_path: &Path, account_txt_file: Option<&Path>) -> Result<ScriptRunResult, String> {
    let started_at = Instant::now();
    start_run_state(script_path, account_txt_file);
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
        with_run_state(|state| {
            state.python = python.clone();
        });
        let mut cmd = Command::new(&python);
        cmd.arg("-u");
        cmd.arg(script_path);
        if let Some(file) = account_txt_file {
            cmd.arg("--file").arg(file);
        }
        let spawn = cmd
            .current_dir(script_path.parent().unwrap_or_else(|| Path::new(".")))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match spawn {
            Ok(mut child) => {
                let stdout_pipe = child.stdout.take();
                let stderr_pipe = child.stderr.take();
                let stdout_buffer = Arc::new(Mutex::new(String::new()));
                let stderr_buffer = Arc::new(Mutex::new(String::new()));

                let out_join = stdout_pipe.map({
                    let stdout_buffer = Arc::clone(&stdout_buffer);
                    move |pipe| {
                        thread::spawn(move || {
                            let mut reader = BufReader::new(pipe);
                            let mut line = String::new();
                            loop {
                                line.clear();
                                match reader.read_line(&mut line) {
                                    Ok(0) => break,
                                    Ok(_) => {
                                        {
                                            let mut out = match stdout_buffer.lock() {
                                                Ok(g) => g,
                                                Err(p) => p.into_inner(),
                                            };
                                            out.push_str(&line);
                                        }
                                        append_run_log(false, &line);
                                    }
                                    Err(_) => break,
                                }
                            }
                        })
                    }
                });

                let err_join = stderr_pipe.map({
                    let stderr_buffer = Arc::clone(&stderr_buffer);
                    move |pipe| {
                        thread::spawn(move || {
                            let mut reader = BufReader::new(pipe);
                            let mut line = String::new();
                            loop {
                                line.clear();
                                match reader.read_line(&mut line) {
                                    Ok(0) => break,
                                    Ok(_) => {
                                        {
                                            let mut out = match stderr_buffer.lock() {
                                                Ok(g) => g,
                                                Err(p) => p.into_inner(),
                                            };
                                            out.push_str(&line);
                                        }
                                        append_run_log(true, &line);
                                    }
                                    Err(_) => break,
                                }
                            }
                        })
                    }
                });

                let status = child
                    .wait()
                    .map_err(|err| format!("等待注册脚本进程失败: {err}"))?;
                if let Some(join) = out_join {
                    let _ = join.join();
                }
                if let Some(join) = err_join {
                    let _ = join.join();
                }

                let elapsed_ms = started_at.elapsed().as_millis();
                let stdout_text = {
                    let out = match stdout_buffer.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    out.clone()
                };
                let stderr_text = {
                    let out = match stderr_buffer.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    out.clone()
                };

                if status.success() {
                    log::info!(
                        "account register script finished: python={} script={} elapsed_ms={}",
                        python,
                        script_path.display(),
                        elapsed_ms
                    );
                    finish_run_state();
                    return Ok(ScriptRunResult {
                        python,
                        stdout: stdout_text,
                        stderr: stderr_text,
                        elapsed_ms,
                    });
                }
                let stderr_tail = capture_output(stderr_text.as_bytes(), 600);
                let stdout_tail = capture_output(stdout_text.as_bytes(), 300);
                finish_run_state();
                return Err(format!(
                    "注册脚本执行失败 (python={python}, exit={:?}): {}\n{}",
                    status.code(),
                    stderr_tail,
                    if stdout_tail.is_empty() {
                        String::new()
                    } else {
                        format!("stdout: {stdout_tail}")
                    }
                ));
            }
            Err(err) => {
                launch_errors.push(format!("{python}: {err}"));
            }
        }
    }

    finish_run_state();
    Err(format!(
        "无法启动 Python 执行脚本 {}，请设置 {}。错误: {}",
        script_path.display(),
        ACCOUNT_REGISTER_PYTHON_ENV,
        launch_errors.join(" | ")
    ))
}

fn read_import_content(file_path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(file_path)
        .map_err(|err| format!("读取导入文件失败 ({}): {err}", file_path.display()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("导入文件为空: {}", file_path.display()));
    }
    Ok(trimmed.to_string())
}

#[derive(Default)]
struct LanuFilterStats {
    source_total: usize,
    eligible_total: usize,
    skipped_unsuccessful: usize,
    skipped_missing_tokens: usize,
}

fn has_non_empty_string_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> bool {
    obj.get(key)
        .and_then(|value| value.as_str())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn filter_lanu_content(raw_content: &str) -> Result<(String, LanuFilterStats), String> {
    let parsed = serde_json::from_str::<serde_json::Value>(raw_content)
        .map_err(|err| format!("解析 {LANU_RESULTS_FILE_NAME} 失败: {err}"))?;
    let items = parsed
        .as_array()
        .ok_or_else(|| format!("{LANU_RESULTS_FILE_NAME} 必须是 JSON 数组"))?;

    let mut stats = LanuFilterStats {
        source_total: items.len(),
        ..LanuFilterStats::default()
    };
    let mut filtered = Vec::with_capacity(items.len());

    for item in items {
        let success = item
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !success {
            stats.skipped_unsuccessful += 1;
            continue;
        }

        let Some(tokens) = item.get("tokens").and_then(|value| value.as_object()) else {
            stats.skipped_missing_tokens += 1;
            continue;
        };
        let has_required_tokens = has_non_empty_string_field(tokens, "access_token")
            && has_non_empty_string_field(tokens, "id_token")
            && has_non_empty_string_field(tokens, "refresh_token");
        if !has_required_tokens {
            stats.skipped_missing_tokens += 1;
            continue;
        }

        filtered.push(item.clone());
    }

    stats.eligible_total = filtered.len();
    let content = serde_json::to_string(&filtered)
        .map_err(|err| format!("序列化筛选后的导入内容失败: {err}"))?;
    Ok((content, stats))
}

fn extract_u64_after(line: &str, marker: &str) -> Option<u64> {
    let start = line.find(marker)? + marker.len();
    let digits = line[start..]
        .chars()
        .skip_while(|ch| ch.is_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

fn parse_script_summary(stdout: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    for line in stdout.lines().rev() {
        if !line.contains("完成!") || !line.contains("成功:") || !line.contains("失败:") {
            continue;
        }
        let success = extract_u64_after(line, "成功:");
        let failed = extract_u64_after(line, "失败:");
        let total = extract_u64_after(line, "总计:");
        return (total, success, failed);
    }
    (None, None, None)
}

pub(crate) fn import_lanu_results() -> Result<serde_json::Value, String> {
    let script_file = resolve_register_script_path()?;
    let account_txt_file = resolve_account_txt_path_for_read().ok();
    let run_result = run_register_script(&script_file, account_txt_file.as_deref())?;
    let source_file = resolve_lanu_results_path().map_err(|err| {
        let stdout_tail = capture_output(run_result.stdout.as_bytes(), 500);
        let stderr_tail = capture_output(run_result.stderr.as_bytes(), 300);
        let mut detail = err;
        if !stdout_tail.is_empty() {
            detail.push_str("\n脚本输出:\n");
            detail.push_str(&stdout_tail);
        }
        if !stderr_tail.is_empty() {
            detail.push_str("\n脚本错误:\n");
            detail.push_str(&stderr_tail);
        }
        detail
    })?;
    let raw_content = read_import_content(&source_file)?;
    let (content, filter_stats) = filter_lanu_content(&raw_content)?;
    let import_result = crate::account_import::import_account_auth_json(vec![content])?;

    let mut payload =
        serde_json::to_value(import_result).map_err(|err| format!("序列化导入结果失败: {err}"))?;
    let Some(result) = payload.as_object_mut() else {
        return Err("导入结果格式异常".to_string());
    };
    result.insert(
        "sourceFile".to_string(),
        serde_json::json!(source_file.to_string_lossy().to_string()),
    );
    result.insert(
        "scriptFile".to_string(),
        serde_json::json!(script_file.to_string_lossy().to_string()),
    );
    result.insert("scriptPython".to_string(), serde_json::json!(&run_result.python));
    if let Some(txt_file) = account_txt_file {
        result.insert(
            "accountTxtFile".to_string(),
            serde_json::json!(txt_file.to_string_lossy().to_string()),
        );
    }
    result.insert("fileCount".to_string(), serde_json::json!(1));
    result.insert("scriptLog".to_string(), serde_json::json!(&run_result.stdout));
    result.insert("scriptStderr".to_string(), serde_json::json!(&run_result.stderr));
    result.insert("scriptElapsedMs".to_string(), serde_json::json!(run_result.elapsed_ms as u64));
    let (script_total, script_success, script_failed) = parse_script_summary(&run_result.stdout);
    if let Some(value) = script_total {
        result.insert("scriptTotalCount".to_string(), serde_json::json!(value));
    }
    if let Some(value) = script_success {
        result.insert("scriptSuccessCount".to_string(), serde_json::json!(value));
    }
    if let Some(value) = script_failed {
        result.insert("scriptFailedCount".to_string(), serde_json::json!(value));
    }
    result.insert(
        "sourceTotal".to_string(),
        serde_json::json!(filter_stats.source_total as u64),
    );
    result.insert(
        "eligibleTotal".to_string(),
        serde_json::json!(filter_stats.eligible_total as u64),
    );
    result.insert(
        "skippedUnsuccessful".to_string(),
        serde_json::json!(filter_stats.skipped_unsuccessful as u64),
    );
    result.insert(
        "skippedMissingTokens".to_string(),
        serde_json::json!(filter_stats.skipped_missing_tokens as u64),
    );
    Ok(payload)
}

pub(crate) fn get_lanu_run_status() -> Result<serde_json::Value, String> {
    let snapshot = {
        let mutex = run_state();
        let guard = match mutex.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.clone()
    };
    Ok(serde_json::json!({
        "running": snapshot.running,
        "scriptFile": snapshot.script_file,
        "accountTxtFile": snapshot.account_txt_file,
        "scriptPython": snapshot.python,
        "scriptLog": snapshot.log,
        "scriptStderr": snapshot.stderr,
        "updatedAtMs": snapshot.updated_at_ms as u64,
    }))
}
