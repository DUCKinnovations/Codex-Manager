"use client";

export interface ScriptLogResult {
  total?: number;
  created?: number;
  updated?: number;
  failed?: number;
  sourceFile?: string;
  scriptFile?: string;
  scriptPython?: string;
  scriptLog?: string;
  scriptStderr?: string;
  scriptElapsedMs?: number;
  scriptTotalCount?: number;
  scriptSuccessCount?: number;
  scriptFailedCount?: number;
  canceled?: boolean;
}

export interface ScriptLogHistoryItem {
  id: string;
  createdAt: number;
  result: ScriptLogResult;
}

export const SCRIPT_LOG_HISTORY_KEY = "codexmanager.accounts.script-log-history";
export const SCRIPT_LOG_HISTORY_LIMIT = 50;

export function readScriptLogHistory(): ScriptLogHistoryItem[] {
  try {
    const raw = window.localStorage.getItem(SCRIPT_LOG_HISTORY_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as ScriptLogHistoryItem[];
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((item) => item && typeof item === "object" && item.result)
      .slice(0, SCRIPT_LOG_HISTORY_LIMIT);
  } catch {
    return [];
  }
}

export function writeScriptLogHistory(items: ScriptLogHistoryItem[]) {
  try {
    window.localStorage.setItem(
      SCRIPT_LOG_HISTORY_KEY,
      JSON.stringify(items.slice(0, SCRIPT_LOG_HISTORY_LIMIT)),
    );
    window.dispatchEvent(new CustomEvent("codexmanager:import-log-updated"));
  } catch {
    // ignore local storage write errors
  }
}

export function appendScriptLogHistory(result: ScriptLogResult): ScriptLogHistoryItem[] {
  const next: ScriptLogHistoryItem = {
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    createdAt: Date.now(),
    result,
  };
  const items = [next, ...readScriptLogHistory()].slice(0, SCRIPT_LOG_HISTORY_LIMIT);
  writeScriptLogHistory(items);
  return items;
}

export function clearScriptLogHistory() {
  writeScriptLogHistory([]);
}
