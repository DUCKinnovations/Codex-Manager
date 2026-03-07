use codexmanager_core::auth::{DEFAULT_CLIENT_ID, DEFAULT_ISSUER, DEFAULT_ORIGINATOR};
use codexmanager_core::rpc::types::{JsonRpcRequest, JsonRpcResponse};
use codexmanager_core::storage::{now_ts, Storage};
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

#[path = "account/account_availability.rs"]
mod account_availability;
#[path = "account/account_cleanup.rs"]
mod account_cleanup;
#[path = "account/account_delete.rs"]
mod account_delete;
#[path = "account/account_delete_many.rs"]
mod account_delete_many;
#[path = "account/account_export.rs"]
mod account_export;
#[path = "account/account_import.rs"]
mod account_import;
#[path = "account/account_list.rs"]
mod account_list;
#[path = "account/account_status.rs"]
mod account_status;
#[path = "account/account_update.rs"]
mod account_update;
#[path = "apikey/apikey_create.rs"]
mod apikey_create;
#[path = "apikey/apikey_delete.rs"]
mod apikey_delete;
#[path = "apikey/apikey_disable.rs"]
mod apikey_disable;
#[path = "apikey/apikey_enable.rs"]
mod apikey_enable;
#[path = "apikey/apikey_list.rs"]
mod apikey_list;
#[path = "apikey/apikey_models.rs"]
mod apikey_models;
#[path = "apikey/apikey_profile.rs"]
mod apikey_profile;
#[path = "apikey/apikey_read_secret.rs"]
mod apikey_read_secret;
#[path = "apikey/apikey_update_model.rs"]
mod apikey_update_model;
#[path = "auth/auth_callback.rs"]
mod auth_callback;
#[path = "auth/auth_login.rs"]
mod auth_login;
#[path = "auth/auth_tokens.rs"]
mod auth_tokens;
mod gateway;
mod http;
mod lock_utils;
pub mod process_env;
mod reasoning_effort;
#[path = "requestlog/requestlog_clear.rs"]
mod requestlog_clear;
#[path = "requestlog/requestlog_list.rs"]
mod requestlog_list;
#[path = "requestlog/requestlog_today_summary.rs"]
mod requestlog_today_summary;
mod rpc_dispatch;
#[path = "storage/storage_helpers.rs"]
mod storage_helpers;
#[path = "usage/usage_account_meta.rs"]
mod usage_account_meta;
#[path = "usage/usage_http.rs"]
mod usage_http;
#[path = "usage/usage_keepalive.rs"]
mod usage_keepalive;
#[path = "usage/usage_list.rs"]
mod usage_list;
#[path = "usage/usage_read.rs"]
mod usage_read;
#[path = "usage/usage_refresh.rs"]
mod usage_refresh;
#[path = "usage/usage_scheduler.rs"]
mod usage_scheduler;
#[path = "usage/usage_snapshot_store.rs"]
mod usage_snapshot_store;
#[path = "usage/usage_token_refresh.rs"]
mod usage_token_refresh;

pub const DEFAULT_ADDR: &str = "localhost:48760";
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:48760";

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static RPC_AUTH_TOKEN: OnceLock<String> = OnceLock::new();
static ENV_OVERRIDE_BASELINE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

pub mod portable {
    // 中文注释：service/web 发行物使用“同目录可选 env 文件 + 默认 DB + token 文件”机制，做到解压即用。
    pub fn bootstrap_current_process() {
        crate::process_env::load_env_from_exe_dir();
        crate::process_env::ensure_default_db_path();
        // 提前生成并落库 token，便于 web 进程/外部工具复用同一 token。
        let _ = crate::rpc_auth_token();
    }
}

pub const SERVICE_BIND_MODE_SETTING_KEY: &str = "service.bind_mode";
pub const SERVICE_BIND_MODE_LOOPBACK: &str = "loopback";
pub const SERVICE_BIND_MODE_ALL_INTERFACES: &str = "all_interfaces";

fn normalize_service_bind_mode(raw: Option<&str>) -> &'static str {
    let Some(value) = raw else {
        return SERVICE_BIND_MODE_LOOPBACK;
    };
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "all_interfaces" | "all-interfaces" | "all" | "0.0.0.0" => SERVICE_BIND_MODE_ALL_INTERFACES,
        _ => SERVICE_BIND_MODE_LOOPBACK,
    }
}

fn open_app_settings_storage() -> Option<Storage> {
    crate::process_env::ensure_default_db_path();
    let path = std::env::var("CODEXMANAGER_DB_PATH").ok()?;
    let storage = Storage::open(&path).ok()?;
    let _ = storage.init();
    Some(storage)
}

pub fn current_service_bind_mode() -> String {
    get_persisted_app_setting(SERVICE_BIND_MODE_SETTING_KEY)
        .map(|value| normalize_service_bind_mode(Some(&value)).to_string())
        .or_else(current_env_service_bind_mode)
        .unwrap_or_else(|| SERVICE_BIND_MODE_LOOPBACK.to_string())
}

pub fn set_service_bind_mode(mode: &str) -> Result<String, String> {
    let normalized = normalize_service_bind_mode(Some(mode)).to_string();
    let storage = open_app_settings_storage().ok_or_else(|| "storage unavailable".to_string())?;
    storage
        .set_app_setting(SERVICE_BIND_MODE_SETTING_KEY, &normalized, now_ts())
        .map_err(|err| format!("save service bind mode failed: {err}"))?;
    Ok(normalized)
}

pub fn bind_all_interfaces_enabled() -> bool {
    current_service_bind_mode() == SERVICE_BIND_MODE_ALL_INTERFACES
}

pub fn default_listener_bind_addr() -> String {
    if bind_all_interfaces_enabled() {
        DEFAULT_BIND_ADDR.to_string()
    } else {
        DEFAULT_ADDR.to_string()
    }
}

// 中文注释：客户端本地探活/调用继续走 localhost；真正监听地址是否放开到 0.0.0.0 由配置控制。
pub fn listener_bind_addr(addr: &str) -> String {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return default_listener_bind_addr();
    }

    let addr = trimmed.strip_prefix("http://").unwrap_or(trimmed);
    let addr = addr.strip_prefix("https://").unwrap_or(addr);
    let addr = addr.split('/').next().unwrap_or(addr);
    let bind_all = bind_all_interfaces_enabled();

    if !addr.contains(':') {
        return if bind_all {
            format!("0.0.0.0:{addr}")
        } else {
            format!("localhost:{addr}")
        };
    }

    let Some((host, port)) = addr.rsplit_once(':') else {
        return addr.to_string();
    };
    if host == "0.0.0.0" {
        return format!("0.0.0.0:{port}");
    }
    if host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
    {
        return if bind_all {
            format!("0.0.0.0:{port}")
        } else {
            format!("localhost:{port}")
        };
    }

    addr.to_string()
}

pub const APP_SETTING_UPDATE_AUTO_CHECK_KEY: &str = "app.update.auto_check";
pub const APP_SETTING_CLOSE_TO_TRAY_ON_CLOSE_KEY: &str = "app.close_to_tray_on_close";
pub const APP_SETTING_UI_LOW_TRANSPARENCY_KEY: &str = "ui.low_transparency";
pub const APP_SETTING_UI_THEME_KEY: &str = "ui.theme";
pub const APP_SETTING_SERVICE_ADDR_KEY: &str = "app.service_addr";
pub const APP_SETTING_GATEWAY_ROUTE_STRATEGY_KEY: &str = "gateway.route_strategy";
pub const APP_SETTING_GATEWAY_CPA_NO_COOKIE_HEADER_MODE_KEY: &str =
    "gateway.cpa_no_cookie_header_mode";
pub const APP_SETTING_GATEWAY_UPSTREAM_PROXY_URL_KEY: &str = "gateway.upstream_proxy_url";
pub const APP_SETTING_GATEWAY_BACKGROUND_TASKS_KEY: &str = "gateway.background_tasks";
pub const APP_SETTING_ENV_OVERRIDES_KEY: &str = "app.env_overrides";
pub const APP_SETTING_WEB_ACCESS_PASSWORD_HASH_KEY: &str = "web.auth.password_hash";
pub const WEB_ACCESS_SESSION_COOKIE_NAME: &str = "codexmanager_web_auth";

const ENV_OVERRIDE_SCOPE_SERVICE: &str = "service";
const ENV_OVERRIDE_SCOPE_DESKTOP: &str = "desktop";
const ENV_OVERRIDE_SCOPE_WEB: &str = "web";
const ENV_OVERRIDE_APPLY_MODE_RUNTIME: &str = "runtime";
const ENV_OVERRIDE_APPLY_MODE_RESTART: &str = "restart";

const APP_SETTINGS_ENV_UNSUPPORTED_KEYS: &[&str] = &[
    "CODEXMANAGER_DB_PATH",
    "CODEXMANAGER_RPC_TOKEN",
    "CODEXMANAGER_RPC_TOKEN_FILE",
];

const APP_SETTINGS_ENV_RESERVED_KEYS: &[&str] = &[
    "CODEXMANAGER_SERVICE_ADDR",
    "CODEXMANAGER_ROUTE_STRATEGY",
    "CODEXMANAGER_CPA_NO_COOKIE_HEADER_MODE",
    "CODEXMANAGER_UPSTREAM_PROXY_URL",
    "CODEXMANAGER_DISABLE_POLLING",
    "CODEXMANAGER_USAGE_POLLING_ENABLED",
    "CODEXMANAGER_USAGE_POLL_INTERVAL_SECS",
    "CODEXMANAGER_GATEWAY_KEEPALIVE_ENABLED",
    "CODEXMANAGER_GATEWAY_KEEPALIVE_INTERVAL_SECS",
    "CODEXMANAGER_TOKEN_REFRESH_POLLING_ENABLED",
    "CODEXMANAGER_TOKEN_REFRESH_POLL_INTERVAL_SECS",
    "CODEXMANAGER_USAGE_REFRESH_WORKERS",
    "CODEXMANAGER_HTTP_WORKER_FACTOR",
    "CODEXMANAGER_HTTP_WORKER_MIN",
    "CODEXMANAGER_HTTP_STREAM_WORKER_FACTOR",
    "CODEXMANAGER_HTTP_STREAM_WORKER_MIN",
];

#[derive(Clone, Copy)]
struct EnvOverrideCatalogItem {
    key: &'static str,
    label: &'static str,
    scope: &'static str,
    apply_mode: &'static str,
    default_value: &'static str,
}

impl EnvOverrideCatalogItem {
    const fn new(
        key: &'static str,
        label: &'static str,
        scope: &'static str,
        apply_mode: &'static str,
        default_value: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            scope,
            apply_mode,
            default_value,
        }
    }
}

const ENV_OVERRIDE_CATALOG: &[EnvOverrideCatalogItem] = &[
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ACCOUNT_IMPORT_BATCH_SIZE",
        "账号导入批大小",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "200",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ACCOUNT_MAX_INFLIGHT",
        "单账号最大并发",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "0",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ALLOW_NON_LOOPBACK_LOGIN_ADDR",
        "允许非回环登录回调",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "0",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_CANDIDATE_CACHE_TTL_MS",
        "候选缓存 TTL（毫秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "500",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_CLIENT_ID",
        "OpenAI Client ID",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        DEFAULT_CLIENT_ID,
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_FRONT_PROXY_MAX_BODY_BYTES",
        "前置代理最大请求体（字节）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "16777216",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_GATEWAY_KEEPALIVE_FAILURE_BACKOFF_MAX_SECS",
        "保活失败退避上限（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "900",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_GATEWAY_KEEPALIVE_JITTER_SECS",
        "保活抖动（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "5",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_GITHUB_TOKEN",
        "GitHub 访问令牌",
        ENV_OVERRIDE_SCOPE_DESKTOP,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_HTTP_BRIDGE_OUTPUT_TEXT_LIMIT_BYTES",
        "HTTP 桥输出截断上限（字节）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "131072",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_HTTP_QUEUE_FACTOR",
        "普通请求队列因子",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "4",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_HTTP_QUEUE_MIN",
        "普通请求最小队列",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "32",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_HTTP_STREAM_QUEUE_FACTOR",
        "流式请求队列因子",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "2",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_HTTP_STREAM_QUEUE_MIN",
        "流式请求最小队列",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "16",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ISSUER",
        "OpenAI Issuer",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        DEFAULT_ISSUER,
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_LOGIN_ADDR",
        "登录回调监听地址",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "localhost:1455",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_NO_SERVICE",
        "桌面端不启动 Service",
        ENV_OVERRIDE_SCOPE_DESKTOP,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ORIGINATOR",
        "登录 Originator",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        DEFAULT_ORIGINATOR,
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_POLL_FAILURE_BACKOFF_MAX_SECS",
        "通用轮询失败退避上限（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "1800",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_POLL_JITTER_SECS",
        "通用轮询抖动（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "5",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_PROMPT_CACHE_CAPACITY",
        "Prompt 缓存容量",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "4096",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_PROMPT_CACHE_CLEANUP_INTERVAL_SECS",
        "Prompt 缓存清理间隔（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "60",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_PROMPT_CACHE_TTL_SECS",
        "Prompt 缓存 TTL（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "3600",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_PROXY_LIST",
        "上游代理池列表",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_REDIRECT_URI",
        "登录回调 URI",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "http://localhost:1455/auth/callback",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_REQUEST_GATE_WAIT_TIMEOUT_MS",
        "请求闸门等待超时（毫秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "300",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ROUTE_HEALTH_P2C_BALANCED_WINDOW",
        "均衡模式 P2C 窗口",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "6",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ROUTE_HEALTH_P2C_ENABLED",
        "启用路由 P2C 健康选择",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "1",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ROUTE_HEALTH_P2C_ORDERED_WINDOW",
        "有序模式 P2C 窗口",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "3",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ROUTE_STATE_CAPACITY",
        "路由状态容量",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "4096",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_ROUTE_STATE_TTL_SECS",
        "路由状态 TTL（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "21600",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_STRICT_REQUEST_PARAM_ALLOWLIST",
        "严格请求参数白名单",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "1",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_TRACE_BODY_PREVIEW_MAX_BYTES",
        "Trace Body 预览上限（字节）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "0",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_TRACE_QUEUE_CAPACITY",
        "Trace 队列容量",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "2048",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPDATE_PRERELEASE",
        "更新包含预发布",
        ENV_OVERRIDE_SCOPE_DESKTOP,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPDATE_REPO",
        "更新仓库",
        ENV_OVERRIDE_SCOPE_DESKTOP,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "qxcnm/Codex-Manager",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_BASE_URL",
        "上游基础地址",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "https://chatgpt.com/backend-api/codex",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_CONNECT_TIMEOUT_SECS",
        "上游连接超时（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "15",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_COOKIE",
        "上游 Cookie",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_FALLBACK_BASE_URL",
        "上游回退地址",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_STREAM_TIMEOUT_MS",
        "上游流式超时（毫秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "300000",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS",
        "上游总超时（毫秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "120000",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_USAGE_BASE_URL",
        "用量接口基础地址",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "https://chatgpt.com",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_USAGE_POLL_FAILURE_BACKOFF_MAX_SECS",
        "用量轮询失败退避上限（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "1800",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_USAGE_POLL_JITTER_SECS",
        "用量轮询抖动（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "5",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_USAGE_REFRESH_FAILURE_EVENT_WINDOW_SECS",
        "用量失败事件去重窗口（秒）",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "60",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_USAGE_SNAPSHOTS_RETAIN_PER_ACCOUNT",
        "每账号保留用量快照数",
        ENV_OVERRIDE_SCOPE_SERVICE,
        ENV_OVERRIDE_APPLY_MODE_RUNTIME,
        "200",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_WEB_ADDR",
        "Web 监听地址",
        ENV_OVERRIDE_SCOPE_WEB,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "localhost:48761",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_WEB_NO_OPEN",
        "Web 启动后不自动打开",
        ENV_OVERRIDE_SCOPE_WEB,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_WEB_NO_SPAWN_SERVICE",
        "Web 不自动拉起 Service",
        ENV_OVERRIDE_SCOPE_WEB,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
    EnvOverrideCatalogItem::new(
        "CODEXMANAGER_WEB_ROOT",
        "Web 静态资源目录",
        ENV_OVERRIDE_SCOPE_WEB,
        ENV_OVERRIDE_APPLY_MODE_RESTART,
        "",
    ),
];

const DEFAULT_UI_THEME: &str = "tech";
const VALID_UI_THEMES: &[&str] = &[
    "tech", "dark", "business", "mint", "sunset", "grape", "ocean", "forest", "rose", "slate",
    "aurora",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTasksInput {
    pub usage_polling_enabled: Option<bool>,
    pub usage_poll_interval_secs: Option<u64>,
    pub gateway_keepalive_enabled: Option<bool>,
    pub gateway_keepalive_interval_secs: Option<u64>,
    pub token_refresh_polling_enabled: Option<bool>,
    pub token_refresh_poll_interval_secs: Option<u64>,
    pub usage_refresh_workers: Option<usize>,
    pub http_worker_factor: Option<usize>,
    pub http_worker_min: Option<usize>,
    pub http_stream_worker_factor: Option<usize>,
    pub http_stream_worker_min: Option<usize>,
}

impl BackgroundTasksInput {
    fn into_patch(self) -> usage_refresh::BackgroundTasksSettingsPatch {
        usage_refresh::BackgroundTasksSettingsPatch {
            usage_polling_enabled: self.usage_polling_enabled,
            usage_poll_interval_secs: self.usage_poll_interval_secs,
            gateway_keepalive_enabled: self.gateway_keepalive_enabled,
            gateway_keepalive_interval_secs: self.gateway_keepalive_interval_secs,
            token_refresh_polling_enabled: self.token_refresh_polling_enabled,
            token_refresh_poll_interval_secs: self.token_refresh_poll_interval_secs,
            usage_refresh_workers: self.usage_refresh_workers,
            http_worker_factor: self.http_worker_factor,
            http_worker_min: self.http_worker_min,
            http_stream_worker_factor: self.http_stream_worker_factor,
            http_stream_worker_min: self.http_stream_worker_min,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsPatch {
    update_auto_check: Option<bool>,
    close_to_tray_on_close: Option<bool>,
    low_transparency: Option<bool>,
    theme: Option<String>,
    service_addr: Option<String>,
    service_listen_mode: Option<String>,
    route_strategy: Option<String>,
    cpa_no_cookie_header_mode_enabled: Option<bool>,
    upstream_proxy_url: Option<String>,
    background_tasks: Option<BackgroundTasksInput>,
    env_overrides: Option<HashMap<String, String>>,
    web_access_password: Option<String>,
}

fn env_override_catalog_value() -> Vec<Value> {
    ENV_OVERRIDE_CATALOG
        .iter()
        .map(|item| {
            serde_json::json!({
                "key": item.key,
                "label": item.label,
                "scope": item.scope,
                "applyMode": item.apply_mode,
                "defaultValue": env_override_default_value(item.key),
            })
        })
        .collect()
}

fn env_override_baseline() -> &'static Mutex<HashMap<String, Option<String>>> {
    ENV_OVERRIDE_BASELINE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn env_override_catalog_item(key: &str) -> Option<&'static EnvOverrideCatalogItem> {
    ENV_OVERRIDE_CATALOG
        .iter()
        .find(|item| item.key.eq_ignore_ascii_case(key))
}

fn is_env_override_catalog_key(key: &str) -> bool {
    env_override_catalog_item(key).is_some()
}

fn is_env_override_unsupported_key(key: &str) -> bool {
    APP_SETTINGS_ENV_UNSUPPORTED_KEYS
        .iter()
        .any(|item| item.eq_ignore_ascii_case(key))
}

fn is_env_override_reserved_key(key: &str) -> bool {
    APP_SETTINGS_ENV_RESERVED_KEYS
        .iter()
        .any(|item| item.eq_ignore_ascii_case(key))
}

fn normalize_env_override_key(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().to_ascii_uppercase();
    if normalized.is_empty() {
        return Err("environment variable key is empty".to_string());
    }
    if !normalized.starts_with("CODEXMANAGER_") {
        return Err(format!("{normalized} must start with CODEXMANAGER_"));
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(format!("{normalized} contains unsupported characters"));
    }
    if is_env_override_unsupported_key(&normalized) {
        return Err(format!(
            "{normalized} must stay in process/.env because it is required before app_settings can be loaded"
        ));
    }
    if is_env_override_reserved_key(&normalized) {
        return Err(format!(
            "{normalized} is already managed by an existing settings card; update it there instead"
        ));
    }
    Ok(normalized)
}

fn normalize_env_override_patch_value(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_saved_env_override_text(raw: &str) -> String {
    raw.trim().to_string()
}

fn normalize_env_overrides_patch(
    overrides: HashMap<String, String>,
) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut normalized = BTreeMap::new();
    for (raw_key, raw_value) in overrides {
        let key = normalize_env_override_key(&raw_key)?;
        normalized.insert(key, normalize_env_override_patch_value(Some(&raw_value)));
    }
    Ok(normalized)
}

fn parse_saved_env_override_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(normalize_saved_env_override_text(text)),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_string()),
        Value::Null => None,
        _ => None,
    }
}

fn env_override_original_process_value(key: &str) -> Option<String> {
    let baseline =
        crate::lock_utils::lock_recover(env_override_baseline(), "env_override_baseline");
    if let Some(value) = baseline.get(key) {
        return value.clone();
    }
    drop(baseline);
    normalize_optional_text(std::env::var(key).ok().as_deref())
}

fn env_override_default_value(key: &str) -> String {
    env_override_original_process_value(key).unwrap_or_else(|| {
        env_override_catalog_item(key)
            .map(|item| item.default_value.to_string())
            .unwrap_or_default()
    })
}

fn env_override_default_snapshot() -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    for item in ENV_OVERRIDE_CATALOG {
        snapshot.insert(item.key.to_string(), env_override_default_value(item.key));
    }
    snapshot
}

pub fn current_env_overrides() -> BTreeMap<String, String> {
    let mut normalized = env_override_default_snapshot();
    let Some(raw) = get_persisted_app_setting(APP_SETTING_ENV_OVERRIDES_KEY) else {
        return normalized;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Map<String, Value>>(&raw) else {
        log::warn!("parse persisted env overrides failed: invalid json");
        return normalized;
    };

    for (raw_key, raw_value) in parsed {
        let Ok(key) = normalize_env_override_key(&raw_key) else {
            log::warn!(
                "skip persisted env override: key={} invalid",
                raw_key.trim()
            );
            continue;
        };
        if let Some(value) = parse_saved_env_override_value(&raw_value) {
            if is_env_override_catalog_key(&key) || !value.is_empty() {
                normalized.insert(key, value);
            } else {
                normalized.remove(&key);
            }
        }
    }
    normalized
}

fn save_env_overrides_value(overrides: &BTreeMap<String, String>) -> Result<(), String> {
    let raw = serde_json::to_string(overrides)
        .map_err(|err| format!("serialize env overrides failed: {err}"))?;
    save_persisted_app_setting(APP_SETTING_ENV_OVERRIDES_KEY, Some(&raw))
}

fn apply_env_overrides_to_process(
    previous: &BTreeMap<String, String>,
    next: &BTreeMap<String, String>,
) {
    let mut all_keys = BTreeSet::new();
    all_keys.extend(previous.keys().cloned());
    all_keys.extend(next.keys().cloned());
    if all_keys.is_empty() {
        return;
    }

    let mut baseline =
        crate::lock_utils::lock_recover(env_override_baseline(), "env_override_baseline");
    for key in &all_keys {
        baseline
            .entry(key.clone())
            .or_insert_with(|| normalize_optional_text(std::env::var(key).ok().as_deref()));
    }

    for key in all_keys {
        if let Some(value) = next.get(&key) {
            if value.trim().is_empty() {
                if let Some(original) = baseline.get(&key).and_then(|value| value.clone()) {
                    std::env::set_var(&key, original);
                } else {
                    std::env::remove_var(&key);
                }
            } else {
                std::env::set_var(&key, value);
            }
            continue;
        }
        if let Some(original) = baseline.get(&key).and_then(|value| value.clone()) {
            std::env::set_var(&key, original);
        } else {
            std::env::remove_var(&key);
        }
    }
}

fn reload_runtime_after_env_override_apply() {
    gateway::reload_runtime_config_from_env();
    usage_refresh::reload_background_tasks_runtime_from_env();
    usage_http::reload_usage_http_client_from_env();
}

pub fn set_env_overrides(
    overrides: HashMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let previous = current_env_overrides();
    let patch = normalize_env_overrides_patch(overrides)?;
    let mut next = if patch.is_empty() {
        env_override_default_snapshot()
    } else {
        previous.clone()
    };

    for (key, value) in patch {
        if let Some(value) = value {
            next.insert(key, value);
        } else if is_env_override_catalog_key(&key) {
            next.insert(key.clone(), env_override_default_value(&key));
        } else {
            next.remove(&key);
        }
    }

    for item in ENV_OVERRIDE_CATALOG {
        next.entry(item.key.to_string())
            .or_insert_with(|| env_override_default_value(item.key));
    }

    save_env_overrides_value(&next)?;
    apply_env_overrides_to_process(&previous, &next);
    reload_runtime_after_env_override_apply();
    Ok(next)
}

fn list_app_settings_map() -> HashMap<String, String> {
    open_app_settings_storage()
        .and_then(|storage| storage.list_app_settings().ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn get_persisted_app_setting(key: &str) -> Option<String> {
    open_app_settings_storage()
        .and_then(|storage| storage.get_app_setting(key).ok().flatten())
        .and_then(|value| normalize_optional_text(Some(&value)))
}

fn save_persisted_app_setting(key: &str, value: Option<&str>) -> Result<(), String> {
    let storage = open_app_settings_storage().ok_or_else(|| "storage unavailable".to_string())?;
    let text = normalize_optional_text(value).unwrap_or_default();
    storage
        .set_app_setting(key, &text, now_ts())
        .map_err(|err| format!("save {key} failed: {err}"))?;
    Ok(())
}

fn save_persisted_bool_setting(key: &str, value: bool) -> Result<(), String> {
    save_persisted_app_setting(key, Some(if value { "1" } else { "0" }))
}

fn parse_bool_with_default(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn normalize_optional_text(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_ui_theme(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or(DEFAULT_UI_THEME).trim().to_ascii_lowercase();
    if VALID_UI_THEMES.iter().any(|theme| *theme == candidate) {
        candidate
    } else {
        DEFAULT_UI_THEME.to_string()
    }
}

fn normalize_saved_service_addr(raw: Option<&str>) -> Result<String, String> {
    let Some(value) = normalize_optional_text(raw) else {
        return Ok(DEFAULT_ADDR.to_string());
    };
    let value = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(&value);
    let value = value.split('/').next().unwrap_or(value).trim();
    if value.is_empty() {
        return Err("service address is empty".to_string());
    }
    if value.contains(':') {
        return Ok(value.to_string());
    }
    Ok(format!("localhost:{value}"))
}

fn current_env_service_addr() -> Option<String> {
    let raw = std::env::var("CODEXMANAGER_SERVICE_ADDR").ok()?;
    let normalized = normalize_saved_service_addr(Some(&raw)).ok()?;
    let Some((host, port)) = normalized.rsplit_once(':') else {
        return Some(normalized);
    };
    match host {
        "0.0.0.0" | "::" | "[::]" => Some(format!("localhost:{port}")),
        _ => Some(normalized),
    }
}

fn current_env_service_bind_mode() -> Option<String> {
    let raw = std::env::var("CODEXMANAGER_SERVICE_ADDR").ok()?;
    let normalized = normalize_saved_service_addr(Some(&raw)).ok()?;
    let host = normalized
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(normalized.as_str());
    let mode = match host {
        "0.0.0.0" | "::" | "[::]" => SERVICE_BIND_MODE_ALL_INTERFACES,
        _ => SERVICE_BIND_MODE_LOOPBACK,
    };
    Some(mode.to_string())
}

pub fn current_saved_service_addr() -> String {
    get_persisted_app_setting(APP_SETTING_SERVICE_ADDR_KEY)
        .and_then(|value| normalize_saved_service_addr(Some(&value)).ok())
        .or_else(current_env_service_addr)
        .unwrap_or_else(|| DEFAULT_ADDR.to_string())
}

pub fn set_saved_service_addr(addr: Option<&str>) -> Result<String, String> {
    let normalized = normalize_saved_service_addr(addr)?;
    save_persisted_app_setting(APP_SETTING_SERVICE_ADDR_KEY, Some(&normalized))?;
    Ok(normalized)
}

pub fn current_update_auto_check_enabled() -> bool {
    get_persisted_app_setting(APP_SETTING_UPDATE_AUTO_CHECK_KEY)
        .map(|value| parse_bool_with_default(&value, true))
        .unwrap_or(true)
}

pub fn set_update_auto_check_enabled(enabled: bool) -> Result<bool, String> {
    save_persisted_bool_setting(APP_SETTING_UPDATE_AUTO_CHECK_KEY, enabled)?;
    Ok(enabled)
}

pub fn current_close_to_tray_on_close_setting() -> bool {
    get_persisted_app_setting(APP_SETTING_CLOSE_TO_TRAY_ON_CLOSE_KEY)
        .map(|value| parse_bool_with_default(&value, false))
        .unwrap_or(false)
}

pub fn set_close_to_tray_on_close_setting(enabled: bool) -> Result<bool, String> {
    save_persisted_bool_setting(APP_SETTING_CLOSE_TO_TRAY_ON_CLOSE_KEY, enabled)?;
    Ok(enabled)
}

pub fn current_ui_low_transparency_enabled() -> bool {
    get_persisted_app_setting(APP_SETTING_UI_LOW_TRANSPARENCY_KEY)
        .map(|value| parse_bool_with_default(&value, false))
        .unwrap_or(false)
}

pub fn set_ui_low_transparency_enabled(enabled: bool) -> Result<bool, String> {
    save_persisted_bool_setting(APP_SETTING_UI_LOW_TRANSPARENCY_KEY, enabled)?;
    Ok(enabled)
}

pub fn current_ui_theme() -> String {
    normalize_ui_theme(get_persisted_app_setting(APP_SETTING_UI_THEME_KEY).as_deref())
}

pub fn set_ui_theme(theme: Option<&str>) -> Result<String, String> {
    let normalized = normalize_ui_theme(theme);
    save_persisted_app_setting(APP_SETTING_UI_THEME_KEY, Some(&normalized))?;
    Ok(normalized)
}

pub fn set_gateway_route_strategy(strategy: &str) -> Result<String, String> {
    let applied = gateway::set_route_strategy(strategy)?.to_string();
    save_persisted_app_setting(APP_SETTING_GATEWAY_ROUTE_STRATEGY_KEY, Some(&applied))?;
    Ok(applied)
}

pub fn set_gateway_cpa_no_cookie_header_mode(enabled: bool) -> Result<bool, String> {
    let applied = gateway::set_cpa_no_cookie_header_mode(enabled);
    save_persisted_bool_setting(APP_SETTING_GATEWAY_CPA_NO_COOKIE_HEADER_MODE_KEY, applied)?;
    Ok(applied)
}

pub fn set_gateway_upstream_proxy_url(proxy_url: Option<&str>) -> Result<Option<String>, String> {
    let normalized = normalize_optional_text(proxy_url);
    let applied = gateway::set_upstream_proxy_url(normalized.as_deref())?;
    save_persisted_app_setting(
        APP_SETTING_GATEWAY_UPSTREAM_PROXY_URL_KEY,
        applied.as_deref(),
    )?;
    Ok(applied)
}

pub fn set_gateway_background_tasks(
    input: BackgroundTasksInput,
) -> Result<serde_json::Value, String> {
    let applied = usage_refresh::set_background_tasks_settings(input.into_patch());
    let raw = serde_json::to_string(&applied)
        .map_err(|err| format!("serialize background tasks failed: {err}"))?;
    save_persisted_app_setting(APP_SETTING_GATEWAY_BACKGROUND_TASKS_KEY, Some(&raw))?;
    serde_json::to_value(applied).map_err(|err| err.to_string())
}

fn current_background_tasks_snapshot_value() -> Result<serde_json::Value, String> {
    serde_json::to_value(usage_refresh::background_tasks_settings()).map_err(|err| err.to_string())
}

pub fn current_web_access_password_hash() -> Option<String> {
    get_persisted_app_setting(APP_SETTING_WEB_ACCESS_PASSWORD_HASH_KEY)
}

pub fn web_access_password_configured() -> bool {
    current_web_access_password_hash().is_some()
}

pub fn set_web_access_password(password: Option<&str>) -> Result<bool, String> {
    match normalize_optional_text(password) {
        Some(value) => {
            let hashed = hash_web_access_password(&value);
            save_persisted_app_setting(APP_SETTING_WEB_ACCESS_PASSWORD_HASH_KEY, Some(&hashed))?;
            Ok(true)
        }
        None => {
            save_persisted_app_setting(APP_SETTING_WEB_ACCESS_PASSWORD_HASH_KEY, Some(""))?;
            Ok(false)
        }
    }
}

pub fn web_auth_status_value() -> Result<Value, String> {
    Ok(serde_json::json!({
        "passwordConfigured": web_access_password_configured(),
    }))
}

pub fn verify_web_access_password(password: &str) -> bool {
    let Some(stored_hash) = current_web_access_password_hash() else {
        return true;
    };
    verify_password_hash(password, &stored_hash)
}

pub fn build_web_access_session_token(password_hash: &str, rpc_token: &str) -> String {
    hex_sha256(format!("codexmanager-web-auth-session:{password_hash}:{rpc_token}").as_bytes())
}

fn hash_web_access_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let salt_hex = hex_encode(&salt);
    let digest = hex_sha256(format!("{salt_hex}:{password}").as_bytes());
    format!("sha256${salt_hex}${digest}")
}

fn verify_password_hash(password: &str, stored_hash: &str) -> bool {
    let mut parts = stored_hash.split('$');
    let Some(kind) = parts.next() else {
        return false;
    };
    let Some(salt_hex) = parts.next() else {
        return false;
    };
    let Some(expected_hash) = parts.next() else {
        return false;
    };
    if kind != "sha256" || parts.next().is_some() {
        return false;
    }
    constant_time_eq(
        hex_sha256(format!("{salt_hex}:{password}").as_bytes()).as_bytes(),
        expected_hash.as_bytes(),
    )
}

fn hex_sha256(bytes: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_ref());
    let digest = hasher.finalize();
    hex_encode(digest.as_slice())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn sync_runtime_settings_from_storage() {
    let settings = list_app_settings_map();
    let env_overrides = current_env_overrides();
    apply_env_overrides_to_process(&env_overrides, &env_overrides);
    reload_runtime_after_env_override_apply();

    if let Some(mode) = settings.get(SERVICE_BIND_MODE_SETTING_KEY) {
        let _ = set_service_bind_mode(mode);
    }
    if let Some(strategy) = settings.get(APP_SETTING_GATEWAY_ROUTE_STRATEGY_KEY) {
        if let Some(strategy) = normalize_optional_text(Some(strategy)) {
            if let Err(err) = gateway::set_route_strategy(&strategy) {
                log::warn!("sync persisted route strategy failed: {err}");
            }
        }
    }
    if let Some(raw) = settings.get(APP_SETTING_GATEWAY_CPA_NO_COOKIE_HEADER_MODE_KEY) {
        gateway::set_cpa_no_cookie_header_mode(parse_bool_with_default(raw, false));
    }
    if let Some(proxy_url) = settings.get(APP_SETTING_GATEWAY_UPSTREAM_PROXY_URL_KEY) {
        let normalized = normalize_optional_text(Some(proxy_url));
        if let Err(err) = gateway::set_upstream_proxy_url(normalized.as_deref()) {
            log::warn!("sync persisted upstream proxy failed: {err}");
        }
    }
    if let Some(raw) = settings.get(APP_SETTING_GATEWAY_BACKGROUND_TASKS_KEY) {
        match serde_json::from_str::<BackgroundTasksInput>(raw) {
            Ok(input) => {
                usage_refresh::set_background_tasks_settings(input.into_patch());
            }
            Err(err) => {
                log::warn!("parse persisted background tasks failed: {err}");
            }
        }
    }
}

pub fn app_settings_get() -> Result<Value, String> {
    app_settings_get_with_overrides(None, None)
}

pub fn app_settings_get_with_overrides(
    close_to_tray_on_close: Option<bool>,
    close_to_tray_supported: Option<bool>,
) -> Result<Value, String> {
    initialize_storage_if_needed()?;
    sync_runtime_settings_from_storage();
    let background_tasks = current_background_tasks_snapshot_value()?;
    let update_auto_check = current_update_auto_check_enabled();
    let persisted_close_to_tray = current_close_to_tray_on_close_setting();
    let close_to_tray = close_to_tray_on_close.unwrap_or(persisted_close_to_tray);
    let low_transparency = current_ui_low_transparency_enabled();
    let theme = current_ui_theme();
    let service_addr = current_saved_service_addr();
    let service_listen_mode = current_service_bind_mode();
    let route_strategy = gateway::current_route_strategy().to_string();
    let cpa_no_cookie_header_mode_enabled = gateway::cpa_no_cookie_header_mode_enabled();
    let upstream_proxy_url = gateway::current_upstream_proxy_url();
    let background_tasks_raw = serde_json::to_string(&background_tasks)
        .map_err(|err| format!("serialize background tasks failed: {err}"))?;
    let env_overrides = current_env_overrides();

    let _ = save_persisted_bool_setting(APP_SETTING_UPDATE_AUTO_CHECK_KEY, update_auto_check);
    let _ = save_persisted_bool_setting(
        APP_SETTING_CLOSE_TO_TRAY_ON_CLOSE_KEY,
        persisted_close_to_tray,
    );
    let _ = save_persisted_bool_setting(APP_SETTING_UI_LOW_TRANSPARENCY_KEY, low_transparency);
    let _ = save_persisted_app_setting(APP_SETTING_UI_THEME_KEY, Some(&theme));
    let _ = save_persisted_app_setting(APP_SETTING_SERVICE_ADDR_KEY, Some(&service_addr));
    let _ = save_persisted_app_setting(SERVICE_BIND_MODE_SETTING_KEY, Some(&service_listen_mode));
    let _ = save_persisted_app_setting(
        APP_SETTING_GATEWAY_ROUTE_STRATEGY_KEY,
        Some(&route_strategy),
    );
    let _ = save_persisted_bool_setting(
        APP_SETTING_GATEWAY_CPA_NO_COOKIE_HEADER_MODE_KEY,
        cpa_no_cookie_header_mode_enabled,
    );
    let _ = save_persisted_app_setting(
        APP_SETTING_GATEWAY_UPSTREAM_PROXY_URL_KEY,
        upstream_proxy_url.as_deref(),
    );
    let _ = save_persisted_app_setting(
        APP_SETTING_GATEWAY_BACKGROUND_TASKS_KEY,
        Some(&background_tasks_raw),
    );
    let _ = save_env_overrides_value(&env_overrides);

    Ok(serde_json::json!({
        "updateAutoCheck": update_auto_check,
        "closeToTrayOnClose": close_to_tray,
        "closeToTraySupported": close_to_tray_supported,
        "lowTransparency": low_transparency,
        "theme": theme,
        "serviceAddr": service_addr,
        "serviceListenMode": service_listen_mode,
        "serviceListenModeOptions": [
            SERVICE_BIND_MODE_LOOPBACK,
            SERVICE_BIND_MODE_ALL_INTERFACES
        ],
        "routeStrategy": route_strategy,
        "routeStrategyOptions": ["ordered", "balanced"],
        "cpaNoCookieHeaderModeEnabled": cpa_no_cookie_header_mode_enabled,
        "upstreamProxyUrl": upstream_proxy_url.unwrap_or_default(),
        "backgroundTasks": background_tasks,
        "envOverrides": env_overrides,
        "envOverrideCatalog": env_override_catalog_value(),
        "envOverrideReservedKeys": APP_SETTINGS_ENV_RESERVED_KEYS,
        "envOverrideUnsupportedKeys": APP_SETTINGS_ENV_UNSUPPORTED_KEYS,
        "webAccessPasswordConfigured": web_access_password_configured(),
    }))
}

pub fn app_settings_set(params: Option<&Value>) -> Result<Value, String> {
    initialize_storage_if_needed()?;
    let patch = match params {
        Some(value) => serde_json::from_value::<AppSettingsPatch>(value.clone())
            .map_err(|err| format!("invalid app settings payload: {err}"))?,
        None => AppSettingsPatch::default(),
    };

    if let Some(enabled) = patch.update_auto_check {
        set_update_auto_check_enabled(enabled)?;
    }
    if let Some(enabled) = patch.close_to_tray_on_close {
        set_close_to_tray_on_close_setting(enabled)?;
    }
    if let Some(enabled) = patch.low_transparency {
        set_ui_low_transparency_enabled(enabled)?;
    }
    if let Some(theme) = patch.theme {
        let _ = set_ui_theme(Some(&theme))?;
    }
    if let Some(service_addr) = patch.service_addr {
        let _ = set_saved_service_addr(Some(&service_addr))?;
    }
    if let Some(mode) = patch.service_listen_mode {
        let _ = set_service_bind_mode(&mode)?;
    }
    if let Some(strategy) = patch.route_strategy {
        let _ = set_gateway_route_strategy(&strategy)?;
    }
    if let Some(enabled) = patch.cpa_no_cookie_header_mode_enabled {
        let _ = set_gateway_cpa_no_cookie_header_mode(enabled)?;
    }
    if let Some(proxy_url) = patch.upstream_proxy_url {
        let _ = set_gateway_upstream_proxy_url(Some(&proxy_url))?;
    }
    if let Some(background_tasks) = patch.background_tasks {
        let _ = set_gateway_background_tasks(background_tasks)?;
    }
    if let Some(env_overrides) = patch.env_overrides {
        let _ = set_env_overrides(env_overrides)?;
    }
    if let Some(password) = patch.web_access_password {
        let _ = set_web_access_password(Some(&password))?;
    }

    app_settings_get()
}

pub struct ServerHandle {
    pub addr: String,
    join: thread::JoinHandle<()>,
}

impl ServerHandle {
    pub fn join(self) {
        let _ = self.join.join();
    }
}

pub fn start_one_shot_server() -> std::io::Result<ServerHandle> {
    portable::bootstrap_current_process();
    gateway::reload_runtime_config_from_env();
    // 中文注释：one-shot 入口也先尝试建表，避免未初始化数据库在首个 RPC 就触发读写失败。
    if let Err(err) = storage_helpers::initialize_storage() {
        log::warn!("storage startup init skipped: {}", err);
    }
    sync_runtime_settings_from_storage();
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    let addr = server
        .server_addr()
        .to_ip()
        .map(|a| a.to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "server addr missing"))?;
    let join = thread::spawn(move || {
        if let Some(request) = server.incoming_requests().next() {
            crate::http::backend_router::handle_backend_request(request);
        }
    });
    Ok(ServerHandle { addr, join })
}

pub fn start_server(addr: &str) -> std::io::Result<()> {
    portable::bootstrap_current_process();
    gateway::reload_runtime_config_from_env();
    // 中文注释：启动阶段先做一次显式初始化；不放在每次 open_storage 里是为避免高频 RPC 重复执行迁移检查。
    if let Err(err) = storage_helpers::initialize_storage() {
        log::warn!("storage startup init skipped: {}", err);
    }
    sync_runtime_settings_from_storage();
    usage_refresh::ensure_usage_polling();
    usage_refresh::ensure_gateway_keepalive();
    usage_refresh::ensure_token_refresh_polling();
    http::server::start_http(addr)
}

pub fn initialize_storage_if_needed() -> Result<(), String> {
    storage_helpers::initialize_storage()
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

pub fn clear_shutdown_flag() {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
}

fn build_rpc_auth_token() -> String {
    if let Some(token) = process_env::read_rpc_token_from_env_or_file() {
        std::env::set_var(process_env::ENV_RPC_TOKEN, &token);
        return token;
    }

    let generated = process_env::generate_rpc_token_hex_32bytes();
    std::env::set_var(process_env::ENV_RPC_TOKEN, &generated);

    // 中文注释：多进程启动（例如 docker compose）时，避免两个进程同时生成不同 token 并互相覆盖。
    if let Some(existing) = process_env::persist_rpc_token_if_missing(&generated) {
        std::env::set_var(process_env::ENV_RPC_TOKEN, &existing);
        return existing;
    }

    generated
}

pub fn rpc_auth_token() -> &'static str {
    RPC_AUTH_TOKEN.get_or_init(build_rpc_auth_token).as_str()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn rpc_auth_token_matches(candidate: &str) -> bool {
    let expected = rpc_auth_token();
    constant_time_eq(expected.as_bytes(), candidate.trim().as_bytes())
}

pub fn request_shutdown(addr: &str) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    // Best-effort wakeups for both IPv4 and IPv6 loopback so whichever listener is active exits.
    let _ = send_shutdown_request(addr);
    let addr_trimmed = addr.trim();
    if addr_trimmed.len() > "localhost:".len()
        && addr_trimmed[..("localhost:".len())].eq_ignore_ascii_case("localhost:")
    {
        let port = &addr_trimmed["localhost:".len()..];
        let _ = send_shutdown_request(&format!("127.0.0.1:{port}"));
        let _ = send_shutdown_request(&format!("[::1]:{port}"));
    }
}

fn send_shutdown_request(addr: &str) -> std::io::Result<()> {
    let addr = addr.trim();
    if addr.is_empty() {
        return Ok(());
    }
    let addr = addr.strip_prefix("http://").unwrap_or(addr);
    let addr = addr.strip_prefix("https://").unwrap_or(addr);
    let addr = addr.split('/').next().unwrap_or(addr);
    let mut stream = TcpStream::connect(addr)?;
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let request = format!("GET /__shutdown HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    Ok(())
}

pub(crate) fn handle_request(req: JsonRpcRequest) -> JsonRpcResponse {
    rpc_dispatch::handle_request(req)
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;
