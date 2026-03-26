"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { Clock3, ListChecks, Save, Settings2, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { ScriptLogModal } from "@/components/modals/script-log-modal";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { accountClient } from "@/lib/api/account-client";
import {
  clearScriptLogHistory,
  readScriptLogHistory,
  type ScriptLogHistoryItem,
  type ScriptLogResult,
} from "@/lib/script-log-history";
import { cn } from "@/lib/utils";

function formatScriptLogTime(ts: number): string {
  return new Date(ts).toLocaleString("zh-CN", { hour12: false });
}

export default function ImportConfigPage() {
  const [activeTab, setActiveTab] = useState("logs");
  const [logs, setLogs] = useState<ScriptLogHistoryItem[]>([]);
  const [selected, setSelected] = useState<ScriptLogResult | null>(null);
  const [open, setOpen] = useState(false);

  const [txtFilePath, setTxtFilePath] = useState("");
  const [txtDraft, setTxtDraft] = useState("");
  const [txtLineCount, setTxtLineCount] = useState(0);
  const [isTxtLoading, setIsTxtLoading] = useState(false);
  const [isTxtSaving, setIsTxtSaving] = useState(false);
  const realtimeLogRef = useRef<HTMLPreElement>(null);
  const previousRunningRef = useRef(false);
  const [autoFollowRealtime, setAutoFollowRealtime] = useState(true);
  const [runStatus, setRunStatus] = useState<{
    running: boolean;
    scriptLog: string;
    scriptStderr: string;
    updatedAtMs: number;
  }>({
    running: false,
    scriptLog: "",
    scriptStderr: "",
    updatedAtMs: 0,
  });

  const reloadLogs = () => {
    setLogs(readScriptLogHistory());
  };

  const loadTxtConfig = async () => {
    setIsTxtLoading(true);
    try {
      const result = await accountClient.getLanuConfig();
      setTxtFilePath(String(result.filePath || ""));
      setTxtDraft(String(result.content || ""));
      setTxtLineCount(Number(result.lineCount || 0));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error(`读取 TXT 配置失败: ${message}`);
    } finally {
      setIsTxtLoading(false);
    }
  };

  useEffect(() => {
    reloadLogs();
    void loadTxtConfig();
  }, []);

  useEffect(() => {
    const handleUpdated = () => reloadLogs();
    window.addEventListener("codexmanager:import-log-updated", handleUpdated as EventListener);
    return () => {
      window.removeEventListener(
        "codexmanager:import-log-updated",
        handleUpdated as EventListener,
      );
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const result = await accountClient.getLanuRunStatus();
        if (cancelled) return;
        setRunStatus({
          running: Boolean(result.running),
          scriptLog: String(result.scriptLog || ""),
          scriptStderr: String(result.scriptStderr || ""),
          updatedAtMs: Number(result.updatedAtMs || 0),
        });
        const currentRunning = Boolean(result.running);
        if (previousRunningRef.current && !currentRunning) {
          reloadLogs();
        }
        previousRunningRef.current = currentRunning;
      } catch {
        // ignore polling error
      }
    };
    void poll();
    const id = window.setInterval(() => {
      void poll();
    }, 1000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const summary = useMemo(() => {
    let total = 0;
    let failed = 0;
    for (const item of logs) {
      total += item.result.total ?? 0;
      failed += Math.max(
        item.result.failed ?? 0,
        item.result.scriptFailedCount ?? 0,
      );
    }
    return { total, failed };
  }, [logs]);

  const draftLineCount = useMemo(
    () =>
      txtDraft
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line.length > 0).length,
    [txtDraft],
  );

  const realtimeText = useMemo(() => {
    const merged = `${runStatus.scriptLog}${runStatus.scriptStderr ? `\n${runStatus.scriptStderr}` : ""}`;
    return merged.trim();
  }, [runStatus.scriptLog, runStatus.scriptStderr]);

  useEffect(() => {
    if (!autoFollowRealtime) return;
    const el = realtimeLogRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [realtimeText, autoFollowRealtime]);

  const saveTxtConfig = async () => {
    setIsTxtSaving(true);
    try {
      const result = await accountClient.setLanuConfig(txtDraft, txtFilePath || undefined);
      setTxtFilePath(String(result.filePath || txtFilePath));
      setTxtLineCount(Number(result.lineCount || draftLineCount));
      toast.success("TXT 邮箱配置已保存");
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error(`保存 TXT 配置失败: ${message}`);
    } finally {
      setIsTxtSaving(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="metric-chip rounded-2xl p-3">
          <p className="text-[11px] text-muted-foreground">日志条数</p>
          <p className="mt-1 text-2xl font-semibold">{logs.length}</p>
        </div>
        <div className="metric-chip rounded-2xl p-3">
          <p className="text-[11px] text-muted-foreground">累计导入总计</p>
          <p className="mt-1 text-2xl font-semibold">{summary.total}</p>
        </div>
        <div className="metric-chip rounded-2xl p-3">
          <p className="text-[11px] text-muted-foreground">TXT 邮箱行数</p>
          <p className="mt-1 text-2xl font-semibold">{txtLineCount || draftLineCount}</p>
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab} className="w-full">
        <TabsList className="glass-card mb-3 h-11 w-full grid grid-cols-2 rounded-xl border-none p-1">
          <TabsTrigger value="logs" className="gap-2">
            <ListChecks className="h-4 w-4" />
            日志查看
          </TabsTrigger>
          <TabsTrigger value="txt" className="gap-2">
            <Settings2 className="h-4 w-4" />
            TXT 邮箱配置
          </TabsTrigger>
        </TabsList>

        <TabsContent value="logs" className="mt-0">
          <Card className="glass-card border-none shadow-xl backdrop-blur-md">
            <CardContent className="space-y-3 p-4">
              <div className="rounded-xl border border-border/60 bg-black/80 p-3">
                <div className="mb-2 flex items-center justify-between text-xs">
                  <span className="text-zinc-300">
                    实时运行日志
                  </span>
                  <span className={cn(runStatus.running ? "text-emerald-400" : "text-zinc-400")}>
                    {runStatus.running ? "运行中" : "空闲"}
                  </span>
                </div>
                <div className="mb-2 text-[11px] text-zinc-500">
                  可拖拽右下角拉伸日志框高度
                </div>
                <pre
                  ref={realtimeLogRef}
                  onScroll={(event) => {
                    const target = event.currentTarget;
                    const nearBottom =
                      target.scrollTop + target.clientHeight >= target.scrollHeight - 8;
                    setAutoFollowRealtime(nearBottom);
                  }}
                  style={{ height: "640px" }}
                  className="min-h-[140px] resize-y overflow-auto whitespace-pre-wrap break-all font-mono text-xs leading-relaxed text-green-400"
                >
                  {realtimeText || "（暂无实时输出）"}
                </pre>
              </div>

              <div className="flex items-center justify-between">
                <div className="text-sm font-medium">导入日志列表</div>
                <div className="flex items-center gap-2">
                  <Button variant="outline" size="sm" onClick={reloadLogs}>
                    刷新
                  </Button>
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={logs.length === 0}
                    onClick={() => {
                      clearScriptLogHistory();
                      reloadLogs();
                    }}
                  >
                    <Trash2 className="mr-1 h-3.5 w-3.5" />
                    清空
                  </Button>
                </div>
              </div>

              {logs.length === 0 ? (
                <div className="rounded-xl border border-dashed border-border/60 p-4 text-sm text-muted-foreground">
                  暂无导入日志。请先到“账号管理”执行一次“一键注册并导入”。
                </div>
              ) : (
                <div className="grid gap-2">
                  {logs.map((item) => {
                    const total = item.result.total ?? 0;
                    const created = item.result.created ?? 0;
                    const updated = item.result.updated ?? 0;
                    const failed = Math.max(
                      item.result.failed ?? 0,
                      item.result.scriptFailedCount ?? 0,
                    );
                    const success = Math.max(
                      item.result.scriptSuccessCount ?? total - failed,
                      0,
                    );
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => {
                          setSelected(item.result);
                          setOpen(true);
                        }}
                        className="w-full rounded-xl border border-border/60 bg-card/40 px-4 py-3 text-left transition-colors hover:bg-card/70"
                      >
                        <div className="flex items-center justify-between">
                          <div className="text-sm font-medium">总计 {total}</div>
                          <span
                            className={cn(
                              "rounded px-2 py-0.5 text-xs",
                              failed > 0
                                ? "bg-red-500/10 text-red-600"
                                : "bg-green-500/10 text-green-600",
                            )}
                          >
                            {failed > 0
                              ? `成功 ${success} / 失败 ${failed}`
                              : `成功 ${success}`}
                          </span>
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">
                          新增 {created} · 更新 {updated}
                        </div>
                        <div className="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground">
                          <Clock3 className="h-3 w-3" />
                          {formatScriptLogTime(item.createdAt)}
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="txt" className="mt-0">
          <Card className="glass-card border-none shadow-xl backdrop-blur-md">
            <CardContent className="space-y-3 p-4">
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="text-sm font-medium">邮箱账号 TXT</div>
                  <div className="truncate text-xs text-muted-foreground">
                    {txtFilePath || "未定位到文件，保存时将自动创建默认文件"}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={isTxtLoading}
                    onClick={() => void loadTxtConfig()}
                  >
                    刷新
                  </Button>
                  <Button
                    size="sm"
                    disabled={isTxtSaving}
                    onClick={() => void saveTxtConfig()}
                  >
                    <Save className="mr-1 h-3.5 w-3.5" />
                    保存
                  </Button>
                </div>
              </div>

              <Textarea
                value={txtDraft}
                onChange={(event) => setTxtDraft(event.target.value)}
                disabled={isTxtLoading || isTxtSaving}
                placeholder="每行一个账号，格式示例：email@example.com----password----mail_url"
                className="min-h-[420px] font-mono text-xs"
              />

              <div className="text-xs text-muted-foreground">
                当前非空行: {draftLineCount}
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <ScriptLogModal open={open} onOpenChange={setOpen} result={selected} />
    </div>
  );
}
