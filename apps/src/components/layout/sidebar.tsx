"use client";

import { usePathname, useRouter } from "next/navigation";
import { 
  LayoutDashboard, 
  Users, 
  Key, 
  FileText, 
  ListChecks,
  Settings, 
  Compass,
  ChevronLeft, 
  ChevronRight
} from "lucide-react";
import { cn } from "@/lib/utils";
import { normalizeRoutePath } from "@/lib/utils/static-routes";
import { Button } from "@/components/ui/button";
import { isTauriRuntime } from "@/lib/api/transport";
import { useAppStore } from "@/lib/store/useAppStore";
import {
  memo,
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  type MouseEvent,
} from "react";

const NAV_ITEMS = [
  { name: "仪表盘", hint: "总览", href: "/", icon: LayoutDashboard },
  { name: "账号管理", hint: "账号池", href: "/accounts/", icon: Users },
  { name: "平台密钥", hint: "网关令牌", href: "/apikeys/", icon: Key },
  { name: "请求日志", hint: "调用追踪", href: "/logs/", icon: FileText },
  { name: "导入配置", hint: "日志与邮箱", href: "/import-config/", icon: ListChecks },
  { name: "设置", hint: "系统参数", href: "/settings/", icon: Settings },
];
const DESKTOP_NAVIGATION_FALLBACK_MS = 500;

const NavItem = memo(({
  item,
  isActive,
  isSidebarOpen,
  onNavigate,
}: {
  item: (typeof NAV_ITEMS)[number],
  isActive: boolean,
  isSidebarOpen: boolean,
  onNavigate: (href: string, event: MouseEvent<HTMLAnchorElement>) => void,
}) => (
  <a
    href={item.href}
    onClick={(event) => onNavigate(item.href, event)}
    title={isSidebarOpen ? undefined : `${item.name} · ${item.hint}`}
    className={cn(
      "nav-item relative flex items-center gap-3 rounded-xl px-3 py-2.5",
      isActive ? "nav-item-active text-primary" : "text-muted-foreground"
    )}
  >
    {isActive ? (
      <span className="absolute left-0 top-1/2 h-6 w-1 -translate-y-1/2 rounded-r-full bg-primary" />
    ) : null}
    <item.icon className="h-4 w-4 shrink-0" />
    {isSidebarOpen ? (
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm font-medium">{item.name}</span>
        <span className="truncate text-[11px] text-muted-foreground">{item.hint}</span>
      </div>
    ) : null}
  </a>
));

NavItem.displayName = "NavItem";

export function Sidebar() {
  const pathname = usePathname();
  const router = useRouter();
  const { isSidebarOpen, toggleSidebar } = useAppStore();
  const normalizedPathname = normalizeRoutePath(pathname);
  const isDesktopStaticRuntime = isTauriRuntime();
  const desktopNavigationFallbackTimerRef = useRef<number | null>(null);

  const handleNavigate = useCallback(
    (href: string, event: MouseEvent<HTMLAnchorElement>) => {
      const nextPath = normalizeRoutePath(href);
      if (nextPath === normalizedPathname) {
        event.preventDefault();
        return;
      }

      event.preventDefault();
      if (isDesktopStaticRuntime) {
        const currentPath = normalizeRoutePath(window.location.pathname);
        if (desktopNavigationFallbackTimerRef.current !== null) {
          window.clearTimeout(desktopNavigationFallbackTimerRef.current);
        }

        startTransition(() => {
          router.push(href);
        });

        desktopNavigationFallbackTimerRef.current = window.setTimeout(() => {
          desktopNavigationFallbackTimerRef.current = null;
          if (normalizeRoutePath(window.location.pathname) === currentPath) {
            window.location.assign(href);
          }
        }, DESKTOP_NAVIGATION_FALLBACK_MS);
        return;
      }

      router.push(href);
    },
    [isDesktopStaticRuntime, normalizedPathname, router],
  );

  useEffect(() => {
    if (desktopNavigationFallbackTimerRef.current !== null) {
      window.clearTimeout(desktopNavigationFallbackTimerRef.current);
      desktopNavigationFallbackTimerRef.current = null;
    }
  }, [normalizedPathname]);

  useEffect(() => {
    return () => {
      if (desktopNavigationFallbackTimerRef.current !== null) {
        window.clearTimeout(desktopNavigationFallbackTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (isDesktopStaticRuntime) {
      return;
    }

    const runtime = globalThis as typeof globalThis & {
      requestIdleCallback?: (
        callback: IdleRequestCallback,
        options?: IdleRequestOptions,
      ) => number;
      cancelIdleCallback?: (handle: number) => void;
    };

    const prefetchRoutes = () => {
      for (const item of NAV_ITEMS) {
        if (normalizeRoutePath(item.href) !== normalizedPathname) {
          router.prefetch(item.href);
        }
      }
    };

    if (runtime.requestIdleCallback) {
      const idleId = runtime.requestIdleCallback(() => prefetchRoutes(), {
        timeout: 1200,
      });
      return () => runtime.cancelIdleCallback?.(idleId);
    }

    const timer = globalThis.setTimeout(prefetchRoutes, 120);
    return () => globalThis.clearTimeout(timer);
  }, [isDesktopStaticRuntime, normalizedPathname, router]);

  const renderedItems = useMemo(() => 
    NAV_ITEMS.map((item) => (
      <NavItem 
        key={item.href} 
        item={item} 
        isActive={normalizeRoutePath(item.href) === normalizedPathname} 
        isSidebarOpen={isSidebarOpen}
        onNavigate={handleNavigate}
      />
    )),
    [handleNavigate, normalizedPathname, isSidebarOpen]
  );

  return (
    <div
      className={cn(
        "relative z-20 flex shrink-0 flex-col glass-sidebar transition-[width] duration-300 ease-in-out",
        isSidebarOpen ? "w-60 xl:w-64" : "w-14 md:w-16"
      )}
    >
      <div className="flex h-16 items-center border-b px-3 md:px-4 shrink-0">
        <div className="flex items-center gap-2 overflow-hidden">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-lg shadow-primary/20">
            <span className="text-sm font-bold">CM</span>
          </div>
          {isSidebarOpen && (
            <div className="flex flex-col overflow-hidden animate-in fade-in duration-300">
              <span className="truncate text-sm font-bold">CodexManager</span>
              <span className="truncate text-[11px] text-muted-foreground opacity-80">账号池 · 用量管理</span>
            </div>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto py-4">
        <div className="mb-2 px-2">
          {isSidebarOpen ? (
            <div className="flex items-center gap-1.5 px-2 py-1 text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
              <Compass className="h-3 w-3" />
              导航
            </div>
          ) : null}
        </div>
        <nav className="grid gap-1.5 px-2">
          {renderedItems}
        </nav>
      </div>

      <div className="border-t p-2 shrink-0">
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "h-10 w-full gap-3 rounded-xl px-3 text-muted-foreground hover:text-foreground",
            isSidebarOpen ? "justify-start" : "justify-center"
          )}
          onClick={toggleSidebar}
        >
          {isSidebarOpen ? (
            <>
              <ChevronLeft className="h-4 w-4 shrink-0" />
              <span className="text-sm">收起侧边栏</span>
            </>
          ) : (
            <ChevronRight className="h-4 w-4 shrink-0" />
          )}
        </Button>
      </div>
    </div>
  );
}
