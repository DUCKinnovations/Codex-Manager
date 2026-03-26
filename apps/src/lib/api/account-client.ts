import { invoke, withAddr } from "./transport";
import {
  normalizeAccountList,
  normalizeApiKeyCreateResult,
  normalizeApiKeyList,
  normalizeApiKeyUsageStats,
  normalizeLoginStartResult,
  normalizeModelOptions,
  normalizeUsageAggregateSummary,
  normalizeUsageList,
  normalizeUsageSnapshot,
} from "./normalize";
import {
  AccountListResult,
  AccountUsage,
  ApiKey,
  ApiKeyCreateResult,
  ApiKeyUsageStat,
  ChatgptAuthTokensRefreshResult,
  CurrentAccessTokenAccountReadResult,
  LoginStatusResult,
  LoginStartResult,
  ModelOption,
  UsageAggregateSummary,
} from "../../types";

interface AccountImportResult {
  canceled?: boolean;
  total?: number;
  created?: number;
  updated?: number;
  failed?: number;
  fileCount?: number;
  sourceFile?: string;
  scriptFile?: string;
  scriptPython?: string;
  scriptLog?: string;
  scriptStderr?: string;
  scriptElapsedMs?: number;
  scriptTotalCount?: number;
  scriptSuccessCount?: number;
  scriptFailedCount?: number;
  directoryPath?: string;
  contents?: string[];
}

interface LanuAccountTxtConfigResult {
  filePath?: string;
  fileName?: string;
  content?: string;
  lineCount?: number;
  ok?: boolean;
  bytes?: number;
}

interface LanuRunStatusResult {
  running?: boolean;
  scriptFile?: string;
  accountTxtFile?: string;
  scriptPython?: string;
  scriptLog?: string;
  scriptStderr?: string;
  updatedAtMs?: number;
}

interface AccountExportResult {
  canceled?: boolean;
  exported?: number;
  outputDir?: string;
}

interface DeleteUnavailableFreeResult {
  deleted?: number;
}

interface LoginStartPayload {
  loginType?: string;
  openBrowser?: boolean;
  note?: string | null;
  tags?: string[] | string | null;
  group?: string | null;
  groupName?: string | null;
  workspaceId?: string | null;
}

interface ChatgptAuthTokensLoginPayload {
  accessToken: string;
  refreshToken?: string | null;
  idToken?: string | null;
  chatgptAccountId?: string | null;
  workspaceId?: string | null;
  chatgptPlanType?: string | null;
}

interface ApiKeyPayload {
  name?: string | null;
  modelSlug?: string | null;
  reasoningEffort?: string | null;
  serviceTier?: string | null;
  protocolType?: string | null;
  upstreamBaseUrl?: string | null;
  staticHeadersJson?: string | null;
}

export const accountClient = {
  async list(params?: Record<string, unknown>): Promise<AccountListResult> {
    const result = await invoke<unknown>("service_account_list", withAddr(params));
    return normalizeAccountList(result);
  },
  delete: (accountId: string) =>
    invoke("service_account_delete", withAddr({ accountId })),
  deleteMany: (accountIds: string[]) =>
    invoke("service_account_delete_many", withAddr({ accountIds })),
  deleteUnavailableFree: () =>
    invoke<DeleteUnavailableFreeResult>("service_account_delete_unavailable_free", withAddr()),
  updateSort: (accountId: string, sort: number) =>
    invoke("service_account_update", withAddr({ accountId, sort })),
  disableAccount: (accountId: string) =>
    invoke("service_account_update", withAddr({ accountId, status: "disabled" })),
  enableAccount: (accountId: string) =>
    invoke("service_account_update", withAddr({ accountId, status: "active" })),
  import: (contents: string[]) =>
    invoke<AccountImportResult>("service_account_import", withAddr({ contents })),
  async importByDirectory(): Promise<AccountImportResult> {
    const picked = await invoke<AccountImportResult>(
      "service_account_import_by_directory",
      withAddr()
    );
    if (picked?.canceled || !Array.isArray(picked?.contents) || picked.contents.length === 0) {
      return picked;
    }

    const imported = await invoke<AccountImportResult>(
      "service_account_import",
      withAddr({ contents: picked.contents })
    );
    return {
      ...imported,
      canceled: false,
      directoryPath: picked.directoryPath || "",
      fileCount: picked.fileCount || picked.contents.length,
    };
  },
  async importByFile(): Promise<AccountImportResult> {
    const picked = await invoke<AccountImportResult>(
      "service_account_import_by_file",
      withAddr()
    );
    if (picked?.canceled || !Array.isArray(picked?.contents) || picked.contents.length === 0) {
      return picked;
    }

    const imported = await invoke<AccountImportResult>(
      "service_account_import",
      withAddr({ contents: picked.contents })
    );
    return {
      ...imported,
      canceled: false,
      fileCount: picked.fileCount || picked.contents.length,
    };
  },
  async importLanuResults(): Promise<AccountImportResult> {
    return invoke<AccountImportResult>(
      "service_account_import_lanu_results",
      withAddr(),
      {
        timeoutMs: 0,
        retries: 0,
      }
    );
  },
  async getLanuConfig(): Promise<LanuAccountTxtConfigResult> {
    return invoke<LanuAccountTxtConfigResult>(
      "service_account_lanu_config_get",
      withAddr()
    );
  },
  async setLanuConfig(content: string, filePath?: string): Promise<LanuAccountTxtConfigResult> {
    return invoke<LanuAccountTxtConfigResult>(
      "service_account_lanu_config_set",
      withAddr({ content, filePath: filePath || null })
    );
  },
  async getLanuRunStatus(): Promise<LanuRunStatusResult> {
    return invoke<LanuRunStatusResult>(
      "service_account_lanu_run_status",
      withAddr(),
      {
        timeoutMs: 0,
        retries: 0,
      }
    );
  },
  export: () =>
    invoke<AccountExportResult>("service_account_export_by_account_files", withAddr()),

  async getUsage(accountId: string): Promise<AccountUsage | null> {
    const result = await invoke<unknown>("service_usage_read", withAddr({ accountId }));
    const source =
      result && typeof result === "object" && "snapshot" in result
        ? (result as { snapshot?: unknown }).snapshot
        : result;
    return normalizeUsageSnapshot(source);
  },
  async listUsage(): Promise<AccountUsage[]> {
    const result = await invoke<unknown>("service_usage_list", withAddr());
    return normalizeUsageList(result);
  },
  refreshUsage: (accountId?: string) =>
    invoke(
      "service_usage_refresh",
      withAddr(accountId ? { accountId } : {})
    ),
  async aggregateUsage(): Promise<UsageAggregateSummary> {
    const result = await invoke<unknown>("service_usage_aggregate", withAddr());
    return normalizeUsageAggregateSummary(result);
  },

  async startLogin(params: LoginStartPayload): Promise<LoginStartResult> {
    const result = await invoke<unknown>(
      "service_login_start",
      withAddr({
        loginType: params?.loginType || "chatgpt",
        openBrowser: params?.openBrowser ?? true,
        note: params?.note || null,
        tags: Array.isArray(params?.tags)
          ? params.tags
              .map((item: string) => String(item || "").trim())
              .filter(Boolean)
              .join(",")
          : params?.tags || null,
        groupName: params?.group || params?.groupName || null,
        workspaceId: params?.workspaceId || null,
      })
    );
    return normalizeLoginStartResult(result);
  },
  async getLoginStatus(loginId: string): Promise<LoginStatusResult> {
    const result = await invoke<unknown>("service_login_status", withAddr({ loginId }));
    const source =
      result && typeof result === "object" && !Array.isArray(result)
        ? (result as Record<string, unknown>)
        : {};
    return {
      status: typeof source.status === "string" ? source.status.trim() : "",
      error: typeof source.error === "string" ? source.error.trim() : "",
    };
  },
  completeLogin: (state: string, code: string, redirectUri: string) =>
    invoke("service_login_complete", withAddr({ state, code, redirectUri })),
  loginWithChatgptAuthTokens: (params: ChatgptAuthTokensLoginPayload) =>
    invoke("service_login_chatgpt_auth_tokens", withAddr({
      accessToken: params.accessToken,
      refreshToken: params.refreshToken || null,
      idToken: params.idToken || null,
      chatgptAccountId: params.chatgptAccountId || null,
      workspaceId: params.workspaceId || null,
      chatgptPlanType: params.chatgptPlanType || null,
    })),
  async readCurrentAccessTokenAccount(
    refreshToken = false
  ): Promise<CurrentAccessTokenAccountReadResult> {
    const result = await invoke<unknown>(
      "service_account_read",
      withAddr({ refreshToken })
    );
    const source =
      result && typeof result === "object" && !Array.isArray(result)
        ? (result as Record<string, unknown>)
        : {};
    return {
      account:
        source.account && typeof source.account === "object" && !Array.isArray(source.account)
          ? (source.account as CurrentAccessTokenAccountReadResult["account"])
          : null,
      authMode: typeof source.authMode === "string" ? source.authMode : null,
      requiresOpenaiAuth: Boolean(source.requiresOpenaiAuth),
    };
  },
  logoutCurrentAccessTokenAccount: () =>
    invoke("service_account_logout", withAddr()),
  async refreshChatgptAuthTokens(
    previousAccountId?: string
  ): Promise<ChatgptAuthTokensRefreshResult> {
    const result = await invoke<unknown>(
      "service_chatgpt_auth_tokens_refresh",
      withAddr({ previousAccountId: previousAccountId || null })
    );
    const source =
      result && typeof result === "object" && !Array.isArray(result)
        ? (result as Record<string, unknown>)
        : {};
    return {
      accountId: String(source.accountId || "").trim(),
      accessToken: String(source.accessToken || "").trim(),
      chatgptAccountId: String(source.chatgptAccountId || "").trim(),
      chatgptPlanType:
        typeof source.chatgptPlanType === "string"
          ? source.chatgptPlanType.trim()
          : null,
      chatgptPlanTypeRaw:
        typeof source.chatgptPlanTypeRaw === "string"
          ? source.chatgptPlanTypeRaw.trim()
          : null,
    };
  },

  async listApiKeys(): Promise<ApiKey[]> {
    const result = await invoke<unknown>("service_apikey_list", withAddr());
    return normalizeApiKeyList(result);
  },
  async createApiKey(params: ApiKeyPayload): Promise<ApiKeyCreateResult> {
    const result = await invoke<unknown>(
      "service_apikey_create",
      withAddr({
        name: params.name || null,
        modelSlug: params.modelSlug || null,
        reasoningEffort: params.reasoningEffort || null,
        serviceTier: params.serviceTier || null,
        protocolType: params.protocolType || null,
        upstreamBaseUrl: params.upstreamBaseUrl || null,
        staticHeadersJson: params.staticHeadersJson || null,
      })
    );
    return normalizeApiKeyCreateResult(result);
  },
  async listApiKeyUsageStats(): Promise<ApiKeyUsageStat[]> {
    const result = await invoke<unknown>("service_apikey_usage_stats", withAddr());
    return normalizeApiKeyUsageStats(result);
  },
  deleteApiKey: (keyId: string) =>
    invoke("service_apikey_delete", withAddr({ keyId })),
  updateApiKey: (keyId: string, params: ApiKeyPayload) =>
    invoke(
      "service_apikey_update_model",
      withAddr({
        keyId,
        name: params.name || null,
        modelSlug: params.modelSlug || null,
        reasoningEffort: params.reasoningEffort || null,
        serviceTier: params.serviceTier || null,
        protocolType: params.protocolType || null,
        upstreamBaseUrl: params.upstreamBaseUrl || null,
        staticHeadersJson: params.staticHeadersJson || null,
      })
    ),
  disableApiKey: (keyId: string) =>
    invoke("service_apikey_disable", withAddr({ keyId })),
  enableApiKey: (keyId: string) =>
    invoke("service_apikey_enable", withAddr({ keyId })),
  async listModels(refreshRemote?: boolean): Promise<ModelOption[]> {
    const result = await invoke<unknown>(
      "service_apikey_models",
      withAddr({ refreshRemote })
    );
    return normalizeModelOptions(result);
  },
  async readApiKeySecret(keyId: string): Promise<string> {
    const result = await invoke<{ key?: string }>(
      "service_apikey_read_secret",
      withAddr({ keyId })
    );
    return String(result?.key || "").trim();
  },
};
