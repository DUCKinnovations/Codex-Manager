function normalizeEnvKey(value) {
  return String(value || "").trim().toUpperCase();
}

function normalizeEnvTextValue(value) {
  return String(value ?? "").trim();
}

function fallbackEnvLabel(key) {
  return key || "未命名变量";
}

const ENV_OVERRIDE_DESCRIPTION_MAP = {
  CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS: "控制单次上游请求允许持续的最长时间，单位毫秒；超过后会主动结束请求并返回超时错误。",
  CODEXMANAGER_UPSTREAM_STREAM_TIMEOUT_MS: "控制流式上游请求允许持续的最长时间，单位毫秒；适合限制长时间无结束的 SSE/流式连接。",
  CODEXMANAGER_UPSTREAM_CONNECT_TIMEOUT_SECS: "控制连接上游服务器时的超时时间，单位秒；主要影响握手和网络建立阶段。",
  CODEXMANAGER_UPSTREAM_BASE_URL: "控制默认上游地址；修改后，网关会把请求转发到新的目标地址。",
  CODEXMANAGER_UPSTREAM_FALLBACK_BASE_URL: "控制主上游失败时的回退地址；当主上游不可用时会尝试切换到这里。",
  CODEXMANAGER_UPSTREAM_COOKIE: "给上游请求补充 Cookie 凭据，常用于通过挑战页或保持受保护会话。",
  CODEXMANAGER_PROXY_LIST: "配置上游代理池列表；多个代理会按账号稳定分配，用于分流或隔离出口。",
  CODEXMANAGER_WEB_ADDR: "控制 Web 管理页的监听地址和端口；修改后会影响浏览器访问入口。",
  CODEXMANAGER_LOGIN_ADDR: "控制本地登录回调服务的监听地址；用于浏览器登录后回传授权结果。",
  CODEXMANAGER_REDIRECT_URI: "控制登录流程使用的回调 URI；需要与回调监听地址保持一致。",
  CODEXMANAGER_WEB_ROOT: "控制 Web 静态资源目录；适合自定义前端资源位置或部署目录。",
  CODEXMANAGER_UPDATE_REPO: "控制桌面端检查更新时使用的 GitHub 仓库地址。",
  CODEXMANAGER_GITHUB_TOKEN: "给更新检查提供 GitHub 访问令牌，减少 API 限流或访问受限带来的失败。",
};

function genericEnvOverrideDescription(item) {
  const key = String(item?.key || "").trim().toUpperCase();
  const label = String(item?.label || "").trim() || fallbackEnvLabel(key);

  if (ENV_OVERRIDE_DESCRIPTION_MAP[key]) {
    return ENV_OVERRIDE_DESCRIPTION_MAP[key];
  }
  if (key.includes("TIMEOUT")) {
    return `控制${label}，超过设定时长后会按对应模块判定为超时；时间单位以变量名中的 ms / secs 为准。`;
  }
  if (key.includes("INTERVAL")) {
    return `控制${label}的执行间隔；值越小触发越频繁，但会带来更高的后台调度开销。`;
  }
  if (key.includes("TTL")) {
    return `控制${label}的保留时长；时间越长命中率更高，但状态或缓存会保留得更久。`;
  }
  if (key.includes("CAPACITY")) {
    return `控制${label}的容量上限；值越大可容纳的数据更多，但会占用更多内存或存储空间。`;
  }
  if (key.includes("QUEUE")) {
    return `控制${label}相关队列的大小；过小会更早限流，过大则可能增加堆积。`;
  }
  if (key.includes("WORKER") || key.includes("INFLIGHT") || key.includes("BATCH_SIZE")) {
    return `控制${label}相关的并发或批量规模；数值越大通常吞吐更高，但资源占用也会增加。`;
  }
  if (key.includes("JITTER") || key.includes("BACKOFF")) {
    return `控制${label}相关的抖动或退避策略，用于降低固定频率重试造成的尖峰压力。`;
  }
  if (key.endsWith("_ADDR") || key.endsWith("_URI") || key.endsWith("_ROOT")) {
    return `控制${label}对应的监听地址、回调地址或目录位置；修改后通常会影响访问入口或资源路径。`;
  }
  if (key.endsWith("_URL") || key.includes("BASE_URL")) {
    return `控制${label}访问的目标地址；修改后，请求会转发到新的上游位置。`;
  }
  if (key.includes("TOKEN") || key.includes("COOKIE")) {
    return `控制${label}使用的凭据内容；通常用于认证、鉴权或保持会话。`;
  }
  if (key.startsWith("CODEXMANAGER_ALLOW_")
    || key.startsWith("CODEXMANAGER_STRICT_")
    || key.startsWith("CODEXMANAGER_NO_")) {
    return `控制是否启用“${label}”这类行为开关；不同取值会直接改变对应模块的运行方式。`;
  }
  return `${label} 对应 ${key} 环境变量，修改后会按当前作用域与生效方式应用到相关模块。`;
}

export function normalizeStringList(value) {
  const items = Array.isArray(value) ? value : [];
  return [...new Set(items.map((item) => String(item || "").trim()).filter(Boolean))]
    .sort((left, right) => left.localeCompare(right));
}

export function normalizeEnvOverrides(value) {
  const source = value && typeof value === "object" ? value : {};
  const entries = [];
  for (const [rawKey, rawValue] of Object.entries(source)) {
    const key = normalizeEnvKey(rawKey);
    if (!key || !key.startsWith("CODEXMANAGER_")) {
      continue;
    }
    entries.push([key, normalizeEnvTextValue(rawValue)]);
  }
  entries.sort(([left], [right]) => left.localeCompare(right));
  return Object.fromEntries(entries);
}

export function normalizeEnvOverrideCatalog(value) {
  const source = Array.isArray(value) ? value : [];
  const catalog = new Map();
  for (const item of source) {
    if (!item || typeof item !== "object") {
      continue;
    }
    const key = normalizeEnvKey(item.key);
    if (!key || !key.startsWith("CODEXMANAGER_")) {
      continue;
    }
    const scope = String(item.scope || "service").trim().toLowerCase() || "service";
    const applyMode = String(item.applyMode || "runtime").trim().toLowerCase() || "runtime";
    const label = String(item.label || "").trim() || fallbackEnvLabel(key);
    const defaultValue = normalizeEnvTextValue(item.defaultValue);
    catalog.set(key, {
      key,
      label,
      scope,
      applyMode,
      defaultValue,
    });
  }
  return [...catalog.values()].sort((left, right) => left.key.localeCompare(right.key));
}

export function filterEnvOverrideCatalog(catalog, keyword) {
  const items = Array.isArray(catalog) ? catalog : [];
  const query = String(keyword || "").trim().toLowerCase();
  if (!query) {
    return [...items];
  }
  return items.filter((item) => {
    const label = String(item.label || "").toLowerCase();
    const key = String(item.key || "").toLowerCase();
    return label.includes(query) || key.includes(query);
  });
}

export function buildEnvOverrideOptionLabel(item) {
  if (!item || typeof item !== "object") {
    return "";
  }
  return String(item.label || "").trim() || fallbackEnvLabel(item.key);
}

export function formatEnvOverrideDisplayValue(value) {
  const normalized = normalizeEnvTextValue(value);
  return normalized || "空";
}

export function buildEnvOverrideDescription(item) {
  if (!item || typeof item !== "object") {
    return "这里会显示当前变量的作用说明。";
  }
  return genericEnvOverrideDescription(item);
}
