"use client";

import { useEffect, useRef } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Terminal, Clock, FileText, CheckCircle2, XCircle } from "lucide-react";

interface ScriptLogModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: {
    total?: number;
    created?: number;
    updated?: number;
    failed?: number;
    sourceFile?: string;
    accountTxtFile?: string;
    scriptFile?: string;
    scriptPython?: string;
    scriptLog?: string;
    scriptStderr?: string;
    scriptElapsedMs?: number;
    scriptFailedCount?: number;
  } | null;
}

function formatElapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainSeconds = seconds % 60;
  return `${minutes}m ${remainSeconds}s`;
}

export function ScriptLogModal({
  open,
  onOpenChange,
  result,
}: ScriptLogModalProps) {
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (open && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [open, result?.scriptLog]);

  if (!result) return null;

  const total = result.total ?? 0;
  const created = result.created ?? 0;
  const updated = result.updated ?? 0;
  const failed = result.failed ?? 0;
  const scriptFailed = result.scriptFailedCount ?? 0;
  const mergedFailed = Math.max(failed, scriptFailed);
  const hasLogFields = "scriptLog" in result || "scriptStderr" in result;
  const logText = [result.scriptLog, result.scriptStderr]
    .filter(Boolean)
    .join("\n")
    .trim();
  const hasLog = logText.length > 0;
  const sourceFileName = result.sourceFile?.split(/[\\/]/).pop() || "";
  const accountTxtFileName = result.accountTxtFile?.split(/[\\/]/).pop() || "";
  const scriptFileName = result.scriptFile?.split(/[\\/]/).pop() || "";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="glass-card border-none sm:max-w-[720px]">
        <DialogHeader>
          <div className="flex items-center gap-3 mb-2">
            <div className="p-2 rounded-full bg-primary/10">
              <Terminal className="h-5 w-5 text-primary" />
            </div>
            <DialogTitle>注册脚本执行日志</DialogTitle>
          </div>
          <DialogDescription>
            自动化注册脚本的执行结果与输出日志
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* stats */}
          <div className="grid grid-cols-4 gap-3">
            <div className="rounded-xl bg-accent/20 p-3 text-center">
              <p className="text-[11px] text-muted-foreground">总计</p>
              <p className="mt-1 text-xl font-semibold">{total}</p>
            </div>
            <div className="rounded-xl bg-green-500/10 p-3 text-center">
              <p className="text-[11px] text-muted-foreground">新增</p>
              <p className="mt-1 text-xl font-semibold text-green-600">
                {created}
              </p>
            </div>
            <div className="rounded-xl bg-blue-500/10 p-3 text-center">
              <p className="text-[11px] text-muted-foreground">更新</p>
              <p className="mt-1 text-xl font-semibold text-blue-600">
                {updated}
              </p>
            </div>
            <div className="rounded-xl bg-red-500/10 p-3 text-center">
              <p className="text-[11px] text-muted-foreground">失败</p>
              <p className="mt-1 text-xl font-semibold text-red-600">
                {failed}
              </p>
            </div>
          </div>

          {/* meta */}
          <div className="flex flex-wrap gap-2 text-[11px]">
            {result.scriptPython && (
              <Badge
                variant="outline"
                className="gap-1 bg-accent/20 font-normal"
              >
                <Terminal className="h-3 w-3" />
                {result.scriptPython}
              </Badge>
            )}
            {scriptFileName && (
              <Badge
                variant="outline"
                className="gap-1 bg-accent/20 font-normal"
              >
                <FileText className="h-3 w-3" />
                {scriptFileName}
              </Badge>
            )}
            {result.scriptElapsedMs != null && (
              <Badge
                variant="outline"
                className="gap-1 bg-accent/20 font-normal"
              >
                <Clock className="h-3 w-3" />
                {formatElapsed(result.scriptElapsedMs)}
              </Badge>
            )}
            {sourceFileName && (
              <Badge
                variant="outline"
                className="gap-1 bg-accent/20 font-normal"
              >
                <FileText className="h-3 w-3" />
                {sourceFileName}
              </Badge>
            )}
            {accountTxtFileName && (
              <Badge
                variant="outline"
                className="gap-1 bg-accent/20 font-normal"
              >
                <FileText className="h-3 w-3" />
                {accountTxtFileName}
              </Badge>
            )}
            {mergedFailed === 0 ? (
              <Badge
                variant="outline"
                className="gap-1 bg-green-500/10 font-normal text-green-600"
              >
                <CheckCircle2 className="h-3 w-3" />
                全部成功
              </Badge>
            ) : (
              <Badge
                variant="outline"
                className="gap-1 bg-red-500/10 font-normal text-red-600"
              >
                <XCircle className="h-3 w-3" />
                {mergedFailed} 个失败
              </Badge>
            )}
          </div>

          {/* log output */}
          <div className="rounded-xl border border-border/50 bg-black/80 backdrop-blur-md">
            <div className="flex items-center gap-2 border-b border-border/30 px-4 py-2">
              <div className="flex gap-1.5">
                <span className="h-3 w-3 rounded-full bg-red-500/80" />
                <span className="h-3 w-3 rounded-full bg-yellow-500/80" />
                <span className="h-3 w-3 rounded-full bg-green-500/80" />
              </div>
              <span className="text-[11px] text-zinc-500">
                脚本输出
              </span>
            </div>
            <pre
              ref={logRef}
              className="max-h-[320px] overflow-auto p-4 font-mono text-xs leading-relaxed text-green-400 whitespace-pre-wrap break-all"
            >
              {hasLog
                ? logText
                : hasLogFields
                  ? "（脚本无输出）"
                  : "（当前服务进程未返回脚本日志字段，请重启到最新版本后重试）"}
            </pre>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
