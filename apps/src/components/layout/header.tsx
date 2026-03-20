"use client";

import { useEffect, useState } from "react";
import { usePathname } from "next/navigation";
import {
  FileText,
  KeyRound,
  LayoutDashboard,
  Settings as SettingsIcon,
  SlidersHorizontal,
  Users,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";
import { useAppStore } from "@/lib/store/useAppStore";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DisclaimerTicker } from "@/components/layout/disclaimer-ticker";
import { WebPasswordModal } from "../modals/web-password-modal";
import { serviceClient } from "@/lib/api/service-client";
import { appClient } from "@/lib/api/app-client";
import { isTauriRuntime } from "@/lib/api/transport";
import {
  formatServiceError,
  isExpectedInitializeResult,
  normalizeServiceAddr,
} from "@/lib/utils/service";

const DEFAULT_SERVICE_ADDR = "localhost:48760";

const PAGE_META: Record<
  string,
  { title: string; subtitle: string; icon: LucideIcon }
> = {
  "/": {
    title: "仪表盘",
    subtitle: "系统状态与账号池总览",
    icon: LayoutDashboard,
  },
  "/accounts": {
    title: "账号管理",
    subtitle: "账号筛选、调度与配额维护",
    icon: Users,
  },
  "/apikeys": {
    title: "平台密钥",
    subtitle: "网关密钥与模型绑定策略",
    icon: KeyRound,
  },
  "/logs": {
    title: "请求日志",
    subtitle: "链路追踪与调用结果排查",
    icon: FileText,
  },
  "/settings": {
    title: "应用设置",
    subtitle: "主题、网关与运行参数",
    icon: SlidersHorizontal,
  },
};

export function Header() {
  const { serviceStatus, setServiceStatus, setAppSettings } = useAppStore();
  const pathname = usePathname();
  const [webPasswordModalOpen, setWebPasswordModalOpen] = useState(false);
  const [isDesktop, setIsDesktop] = useState(false);
  const [isToggling, setIsToggling] = useState(false);
  const [portInput, setPortInput] = useState("48760");

  useEffect(() => {
    setIsDesktop(isTauriRuntime());
  }, []);

  useEffect(() => {
    const current = String(serviceStatus.addr || DEFAULT_SERVICE_ADDR);
    const [, port = current] = current.split(":");
    setPortInput(port || "48760");
  }, [serviceStatus.addr]);

  const normalizedPathname =
    pathname === "/" ? pathname : pathname.replace(/\/+$/, "");
  const pageMeta = PAGE_META[normalizedPathname] || {
    title: "CodexManager",
    subtitle: "账号池与网关管理",
    icon: SlidersHorizontal,
  };
  const PageIcon = pageMeta.icon;

  const persistServiceAddr = async (nextAddr: string) => {
    const normalized = normalizeServiceAddr(nextAddr);
    const settings = await appClient.setSettings({ serviceAddr: normalized });
    setAppSettings(settings);
    setServiceStatus({ addr: normalized });
    return normalized;
  };

  const handleToggleService = async (enabled: boolean) => {
    setIsToggling(true);
    try {
      const nextAddr = await persistServiceAddr(serviceStatus.addr || `localhost:${portInput}`);
      if (enabled) {
        await serviceClient.start(nextAddr);
        const initResult = await serviceClient.initialize(nextAddr);
        if (!isExpectedInitializeResult(initResult)) {
          throw new Error("Port is in use or unexpected service responded (missing server_name)");
        }
        setServiceStatus({
          connected: true,
          version: initResult.version,
          addr: nextAddr,
        });
        toast.success("服务已启动");
      } else {
        await serviceClient.stop();
        setServiceStatus({ connected: false, version: "" });
        toast.info("服务已停止");
      }
    } catch (error: unknown) {
      toast.error(`操作失败: ${formatServiceError(error)}`);
    } finally {
      setIsToggling(false);
    }
  };

  const handlePortBlur = async () => {
    try {
      const nextAddr = await persistServiceAddr(`localhost:${portInput}`);
      setServiceStatus({ addr: nextAddr });
    } catch (error: unknown) {
      toast.error(`地址保存失败: ${formatServiceError(error)}`);
    }
  };

  return (
    <>
      <header className="glass-header sticky top-0 z-30 h-16 px-4 md:px-6">
        <div className="shell-main flex h-full items-center gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/15 text-primary ring-1 ring-primary/20">
              <PageIcon className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <h1 className="truncate text-base font-semibold md:text-lg">{pageMeta.title}</h1>
              <p className="truncate text-[11px] text-muted-foreground">{pageMeta.subtitle}</p>
            </div>
            <Badge
              variant={serviceStatus.connected ? "default" : "secondary"}
              className="ml-2 hidden h-6 items-center gap-1.5 rounded-full px-2.5 md:inline-flex"
            >
              <span
                className={
                  serviceStatus.connected
                    ? "h-1.5 w-1.5 rounded-full bg-emerald-400"
                    : "h-1.5 w-1.5 rounded-full bg-amber-400"
                }
              />
              {serviceStatus.connected ? "服务在线" : "服务离线"}
            </Badge>
            {serviceStatus.version ? (
              <span className="hidden text-xs text-muted-foreground lg:inline">
                v{serviceStatus.version}
              </span>
            ) : null}
          </div>

          <div className="hidden min-w-0 flex-1 justify-center lg:flex">
            <DisclaimerTicker />
          </div>

          <div className="flex shrink-0 items-center gap-2 md:gap-3">
            {isDesktop ? (
              <div className="panel-elevated flex items-center gap-2 rounded-xl px-3 py-1.5 shadow-sm">
                <span className="hidden text-xs font-medium text-muted-foreground sm:inline">
                  监听端口
                </span>
                <Input
                  className="h-7 w-16 border-none bg-transparent p-0 text-xs font-mono focus-visible:ring-0"
                  placeholder="48760"
                  value={portInput}
                  onChange={(event) => {
                    const nextPort = event.target.value.replace(/[^\d]/g, "");
                    setPortInput(nextPort);
                    if (nextPort) {
                      setServiceStatus({ addr: `localhost:${nextPort}` });
                    }
                  }}
                  onBlur={() => void handlePortBlur()}
                />
                <div className="mx-0.5 h-4 w-px bg-border" />
                <Switch
                  checked={serviceStatus.connected}
                  disabled={isToggling}
                  onCheckedChange={handleToggleService}
                  className="scale-90"
                />
              </div>
            ) : null}

            <Button
              variant="outline"
              size="sm"
              className="h-9 gap-2 rounded-xl px-3"
              onClick={() => setWebPasswordModalOpen(true)}
            >
              <SettingsIcon className="h-3.5 w-3.5" />
              <span className="text-xs">密码</span>
            </Button>
          </div>
        </div>
      </header>

      <WebPasswordModal
        open={webPasswordModalOpen}
        onOpenChange={setWebPasswordModalOpen}
      />
    </>
  );
}
