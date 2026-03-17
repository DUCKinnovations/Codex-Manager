"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  BarChart3,
  Download,
  ExternalLink,
  FileUp,
  FolderOpen,
  MoreVertical,
  Pin,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";
import { AddAccountModal } from "@/components/modals/add-account-modal";
import { ConfirmDialog } from "@/components/modals/confirm-dialog";
import UsageModal from "@/components/modals/usage-modal";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAccounts } from "@/hooks/useAccounts";
import { cn } from "@/lib/utils";
import {
  formatTsFromSeconds,
  isPrimaryWindowOnlyUsage,
  isSecondaryWindowOnlyUsage,
} from "@/lib/utils/usage";
import { Account } from "@/types";

type StatusFilter = "all" | "available" | "low_quota";

interface QuotaProgressProps {
  label: string;
  remainPercent: number | null;
  icon: LucideIcon;
  tone: "green" | "blue";
  emptyText?: string;
}

function QuotaProgress({
  label,
  remainPercent,
  icon: Icon,
  tone,
  emptyText = "--",
}: QuotaProgressProps) {
  const value = remainPercent ?? 0;
  const trackClassName = tone === "blue" ? "bg-blue-500/20" : "bg-green-500/20";
  const indicatorClassName = tone === "blue" ? "bg-blue-500" : "bg-green-500";

  return (
    <div className="flex min-w-[120px] flex-col gap-1">
      <div className="flex items-center justify-between text-[10px]">
        <div className="flex items-center gap-1 text-muted-foreground">
          <Icon className="h-3 w-3" />
          <span>{label}</span>
        </div>
        <span className="font-medium">{remainPercent == null ? emptyText : `${value}%`}</span>
      </div>
      <Progress
        value={value}
        trackClassName={trackClassName}
        indicatorClassName={indicatorClassName}
      />
    </div>
  );
}

export default function AccountsPage() {
  const router = useRouter();
  const {
    accounts,
    groups,
    isLoading,
    refreshAccount,
    refreshAllAccounts,
    deleteAccount,
    deleteManyAccounts,
    deleteUnavailableFree,
    importByFile,
    importByDirectory,
    exportAccounts,
    isRefreshingAccountId,
    isRefreshingAllAccounts,
    isExporting,
    isDeletingMany,
    manualPreferredAccountId,
    setPreferredAccount,
    clearPreferredAccount,
    isUpdatingPreferred,
  } = useAccounts();

  const [search, setSearch] = useState("");
  const [groupFilter, setGroupFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [pageSize, setPageSize] = useState("20");
  const [page, setPage] = useState(1);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [addAccountModalOpen, setAddAccountModalOpen] = useState(false);
  const [usageModalOpen, setUsageModalOpen] = useState(false);
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [deleteDialogState, setDeleteDialogState] = useState<
    | { kind: "single"; account: Account }
    | { kind: "selected"; ids: string[]; count: number }
    | null
  >(null);

  const filteredAccounts = useMemo(() => {
    return accounts.filter((account) => {
      const matchSearch =
        !search ||
        account.name.toLowerCase().includes(search.toLowerCase()) ||
        account.id.toLowerCase().includes(search.toLowerCase());
      const matchGroup = groupFilter === "all" || (account.group || "默认") === groupFilter;
      const matchStatus =
        statusFilter === "all" ||
        (statusFilter === "available" && account.isAvailable) ||
        (statusFilter === "low_quota" && account.isLowQuota);
      return matchSearch && matchGroup && matchStatus;
    });
  }, [accounts, groupFilter, search, statusFilter]);

  const pageSizeNumber = Number(pageSize) || 20;
  const totalPages = Math.max(1, Math.ceil(filteredAccounts.length / pageSizeNumber));
  const safePage = Math.min(page, totalPages);
  const accountIdSet = useMemo(() => new Set(accounts.map((account) => account.id)), [accounts]);
  const effectiveSelectedIds = useMemo(
    () => selectedIds.filter((id) => accountIdSet.has(id)),
    [accountIdSet, selectedIds]
  );

  const visibleAccounts = useMemo(() => {
    const offset = (safePage - 1) * pageSizeNumber;
    return filteredAccounts.slice(offset, offset + pageSizeNumber);
  }, [filteredAccounts, pageSizeNumber, safePage]);

  const selectedAccount = useMemo(
    () => accounts.find((account) => account.id === selectedAccountId) ?? null,
    [accounts, selectedAccountId]
  );

  const handleSearchChange = (value: string) => {
    setSearch(value);
    setPage(1);
  };

  const handleGroupFilterChange = (value: string | null) => {
    setGroupFilter(value || "all");
    setPage(1);
  };

  const handleStatusFilterChange = (value: StatusFilter) => {
    setStatusFilter(value);
    setPage(1);
  };

  const handlePageSizeChange = (value: string | null) => {
    setPageSize(value || "20");
    setPage(1);
  };

  const toggleSelect = (id: string) => {
    setSelectedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id]
    );
  };

  const toggleSelectAllVisible = () => {
    const visibleIds = visibleAccounts.map((account) => account.id);
    const allSelected = visibleIds.every((id) => effectiveSelectedIds.includes(id));
    setSelectedIds((current) => {
      if (allSelected) {
        return current.filter((id) => !visibleIds.includes(id));
      }
      return Array.from(new Set([...current, ...visibleIds]));
    });
  };

  const openUsage = (account: Account) => {
    setSelectedAccountId(account.id);
    setUsageModalOpen(true);
  };

  const handleDeleteSelected = () => {
    if (!effectiveSelectedIds.length) {
      toast.error("请先选择要删除的账号");
      return;
    }
    setDeleteDialogState({
      kind: "selected",
      ids: [...effectiveSelectedIds],
      count: effectiveSelectedIds.length,
    });
  };

  const handleDeleteSingle = (account: Account) => {
    setDeleteDialogState({ kind: "single", account });
  };

  const handleConfirmDelete = () => {
    if (!deleteDialogState) return;
    if (deleteDialogState.kind === "single") {
      deleteAccount(deleteDialogState.account.id);
      return;
    }
    deleteManyAccounts(deleteDialogState.ids);
    setSelectedIds((current) => current.filter((id) => !deleteDialogState.ids.includes(id)));
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-start gap-4 xl:flex-nowrap">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3">
            <div className="relative w-full sm:w-64">
              <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder="搜索账号名 / 编号..."
                className="h-10 bg-card/50 pl-9"
                value={search}
                onChange={(event) => handleSearchChange(event.target.value)}
              />
            </div>
            <Select value={groupFilter} onValueChange={handleGroupFilterChange}>
              <SelectTrigger className="h-10 w-[160px] shrink-0 bg-card/50">
                <SelectValue placeholder="全部分组" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">全部分组 ({accounts.length})</SelectItem>
                {groups.map((group) => (
                  <SelectItem key={group.label} value={group.label}>
                    {group.label} ({group.count})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex shrink-0 items-center rounded-lg border bg-muted/30 p-1">
              {[
                { id: "all", label: "全部" },
                { id: "available", label: "可用" },
                { id: "low_quota", label: "低配额" },
              ].map((filter) => (
                <button
                  key={filter.id}
                  onClick={() => handleStatusFilterChange(filter.id as StatusFilter)}
                  className={cn(
                    "rounded-md px-4 py-1.5 text-xs font-medium transition-all",
                    statusFilter === filter.id
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                >
                  {filter.label}
                </button>
              ))}
            </div>
          </div>

          <div className="ml-auto flex items-center gap-2 self-start">
            <DropdownMenu>
              <DropdownMenuTrigger>
                <Button
                  variant="outline"
                  className="h-10 min-w-[132px] justify-between gap-2 rounded-xl border-border/70 bg-card/70 px-3 shadow-sm backdrop-blur-sm"
                  render={<span />}
                  nativeButton={false}
                >
                  <span className="flex items-center gap-2">
                    <span className="text-sm font-medium">账号操作</span>
                    {effectiveSelectedIds.length > 0 ? (
                      <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-semibold text-primary">
                        {effectiveSelectedIds.length}
                      </span>
                    ) : null}
                  </span>
                  <MoreVertical className="h-4 w-4 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-64 rounded-xl border border-border/70 bg-popover/95 p-2 shadow-xl backdrop-blur-md">
                <DropdownMenuGroup>
                  <DropdownMenuLabel className="px-2 py-1 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/80">
                    账号管理
                  </DropdownMenuLabel>
                  <DropdownMenuItem className="h-9 rounded-lg px-2" onClick={() => setAddAccountModalOpen(true)}>
                    <Plus className="mr-2 h-4 w-4" /> 添加账号
                  </DropdownMenuItem>
                  <DropdownMenuItem className="h-9 rounded-lg px-2" onClick={() => importByFile()}>
                    <FileUp className="mr-2 h-4 w-4" /> 按文件导入
                    <DropdownMenuShortcut>FILE</DropdownMenuShortcut>
                  </DropdownMenuItem>
                  <DropdownMenuItem className="h-9 rounded-lg px-2" onClick={() => importByDirectory()}>
                    <FolderOpen className="mr-2 h-4 w-4" /> 按文件夹导入
                    <DropdownMenuShortcut>DIR</DropdownMenuShortcut>
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    className="h-9 rounded-lg px-2"
                    disabled={isExporting}
                    onClick={() => exportAccounts()}
                  >
                    <Download className="mr-2 h-4 w-4" />
                    导出账号
                    <DropdownMenuShortcut>
                      {isExporting ? "..." : "ZIP"}
                    </DropdownMenuShortcut>
                  </DropdownMenuItem>
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuGroup>
                  <DropdownMenuLabel className="px-2 py-1 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/80">
                    批量操作
                  </DropdownMenuLabel>
                  <DropdownMenuItem
                    className="h-9 rounded-lg px-2"
                    disabled={isRefreshingAllAccounts}
                    onClick={() => refreshAllAccounts()}
                  >
                    <RefreshCw className="mr-2 h-4 w-4" /> 刷新所有账号
                    <DropdownMenuShortcut>
                      {isRefreshingAllAccounts ? "..." : "ALL"}
                    </DropdownMenuShortcut>
                  </DropdownMenuItem>
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuGroup>
                  <DropdownMenuLabel className="px-2 py-1 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/80">
                    清理
                  </DropdownMenuLabel>
                  <DropdownMenuItem
                    disabled={!effectiveSelectedIds.length || isDeletingMany}
                    variant="destructive"
                    className="h-9 rounded-lg px-2"
                    onClick={handleDeleteSelected}
                  >
                    <Trash2 className="mr-2 h-4 w-4" /> 删除选中账号
                    <DropdownMenuShortcut>{effectiveSelectedIds.length || "-"}</DropdownMenuShortcut>
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    variant="destructive"
                    className="h-9 rounded-lg px-2"
                    onClick={() => deleteUnavailableFree()}
                  >
                    <Trash2 className="mr-2 h-4 w-4" /> 一键清理不可用免费
                  </DropdownMenuItem>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
            <Button
              className="h-10 gap-2 shadow-lg shadow-primary/20"
              onClick={() => refreshAllAccounts()}
              disabled={isRefreshingAllAccounts}
            >
              <RefreshCw
                className={cn("h-4 w-4", isRefreshingAllAccounts && "animate-spin")}
              />
              刷新所有
            </Button>
          </div>
        </div>
      </div>

      <Card className="overflow-hidden border-none bg-card/50 shadow-xl backdrop-blur-md">
        <CardContent className="p-0">
          <Table>
            <TableHeader className="bg-muted/30">
              <TableRow>
                <TableHead className="w-12 text-center">
                  <Checkbox
                    checked={
                      visibleAccounts.length > 0 &&
                      visibleAccounts.every((account) => effectiveSelectedIds.includes(account.id))
                    }
                    onCheckedChange={toggleSelectAllVisible}
                  />
                </TableHead>
                <TableHead className="max-w-[220px]">账号信息</TableHead>
                <TableHead>5h 额度</TableHead>
                <TableHead>7d 额度</TableHead>
                <TableHead className="w-20">顺序</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="text-center">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                Array.from({ length: 5 }).map((_, index) => (
                  <TableRow key={index}>
                    <TableCell><Skeleton className="mx-auto h-4 w-4" /></TableCell>
                    <TableCell><Skeleton className="h-4 w-32" /></TableCell>
                    <TableCell><Skeleton className="h-4 w-24" /></TableCell>
                    <TableCell><Skeleton className="h-4 w-24" /></TableCell>
                    <TableCell><Skeleton className="h-4 w-10" /></TableCell>
                    <TableCell><Skeleton className="h-6 w-16 rounded-full" /></TableCell>
                    <TableCell><Skeleton className="mx-auto h-8 w-24" /></TableCell>
                  </TableRow>
                ))
              ) : visibleAccounts.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="h-48 text-center">
                    <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground">
                      <Search className="h-8 w-8 opacity-20" />
                      <p>未找到符合条件的账号</p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                visibleAccounts.map((account) => {
                  const primaryWindowOnly = isPrimaryWindowOnlyUsage(account.usage);
                  const secondaryWindowOnly = isSecondaryWindowOnlyUsage(account.usage);
                  return (
                    <TableRow key={account.id} className="group transition-colors hover:bg-muted/30">
                    <TableCell className="text-center">
                      <Checkbox
                        checked={effectiveSelectedIds.includes(account.id)}
                        onCheckedChange={() => toggleSelect(account.id)}
                      />
                    </TableCell>
                    <TableCell className="max-w-[220px]">
                      <div className="flex flex-col overflow-hidden">
                        <div className="flex items-center gap-2 overflow-hidden">
                          <span className="truncate text-sm font-semibold">{account.name}</span>
                          <Badge
                            variant="secondary"
                            className="h-4 shrink-0 bg-accent/50 px-1.5 text-[9px]"
                          >
                            {account.group || "默认"}
                          </Badge>
                          {manualPreferredAccountId === account.id ? (
                            <Badge
                              variant="secondary"
                              className="h-4 shrink-0 bg-amber-500/15 px-1.5 text-[9px] text-amber-700 dark:text-amber-300"
                            >
                              优先
                            </Badge>
                          ) : null}
                        </div>
                        <span className="truncate font-mono text-[10px] uppercase text-muted-foreground opacity-60">
                          {account.id.slice(0, 16)}...
                        </span>
                        <span className="mt-1 text-[10px] text-muted-foreground">
                          最近刷新: {formatTsFromSeconds(account.lastRefreshAt, "从未刷新")}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <QuotaProgress
                        label="5小时"
                        remainPercent={account.primaryRemainPercent}
                        icon={RefreshCw}
                        tone="green"
                        emptyText={secondaryWindowOnly ? "未提供" : "--"}
                      />
                    </TableCell>
                    <TableCell>
                      <QuotaProgress
                        label="7天"
                        remainPercent={account.secondaryRemainPercent}
                        icon={RefreshCw}
                        tone="blue"
                        emptyText={primaryWindowOnly ? "未提供" : "--"}
                      />
                    </TableCell>
                    <TableCell>
                      <span className="rounded bg-muted/50 px-2 py-0.5 font-mono text-xs">
                        {account.priority}
                      </span>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1.5">
                        <div
                          className={cn(
                            "h-1.5 w-1.5 rounded-full",
                            account.isAvailable ? "bg-green-500" : "bg-red-500"
                          )}
                        />
                        <span
                          className={cn(
                            "text-[11px] font-medium",
                            account.isAvailable
                              ? "text-green-600 dark:text-green-400"
                              : "text-red-600 dark:text-red-400"
                          )}
                        >
                          {account.availabilityText}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="table-action-cell gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-muted-foreground transition-colors hover:text-primary"
                          onClick={() => openUsage(account)}
                          title="用量详情"
                        >
                          <BarChart3 className="h-4 w-4" />
                        </Button>
                        <DropdownMenu>
                          <DropdownMenuTrigger>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8"
                              render={<span />}
                              nativeButton={false}
                            >
                              <MoreVertical className="h-4 w-4" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem
                              className="gap-2"
                              disabled={isUpdatingPreferred}
                              onClick={() =>
                                manualPreferredAccountId === account.id
                                  ? clearPreferredAccount()
                                  : setPreferredAccount(account.id)
                              }
                            >
                              <Pin className="h-4 w-4" />
                              {manualPreferredAccountId === account.id ? "取消优先" : "设为优先"}
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              className="gap-2"
                              onClick={() =>
                                router.push(`/logs?query=${encodeURIComponent(account.id)}`)
                              }
                            >
                              <ExternalLink className="h-4 w-4" /> 详情与日志
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              className="gap-2 text-red-500"
                              onClick={() => handleDeleteSingle(account)}
                            >
                              <Trash2 className="h-4 w-4" /> 删除
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <div className="flex items-center justify-between px-2">
        <div className="text-xs text-muted-foreground">
          共 {filteredAccounts.length} 个账号
          {effectiveSelectedIds.length > 0 ? (
            <span className="ml-1 text-primary">(已选择 {effectiveSelectedIds.length} 个)</span>
          ) : null}
        </div>
        <div className="flex items-center gap-6">
          <div className="flex items-center gap-2">
            <span className="whitespace-nowrap text-xs text-muted-foreground">每页显示</span>
            <Select value={pageSize} onValueChange={handlePageSizeChange}>
              <SelectTrigger className="h-8 w-[70px] text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {["5", "10", "20", "50", "100", "500"].map((value) => (
                  <SelectItem key={value} value={value}>
                    {value}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              className="h-8 px-3 text-xs"
              disabled={safePage <= 1}
              onClick={() => setPage((current) => Math.max(1, current - 1))}
            >
              上一页
            </Button>
            <div className="min-w-[60px] text-center text-xs font-medium">
              第 {safePage} / {totalPages} 页
            </div>
            <Button
              variant="outline"
              size="sm"
              className="h-8 px-3 text-xs"
              disabled={safePage >= totalPages}
              onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
            >
              下一页
            </Button>
          </div>
        </div>
      </div>

      {addAccountModalOpen ? (
        <AddAccountModal open={addAccountModalOpen} onOpenChange={setAddAccountModalOpen} />
      ) : null}
      <UsageModal
        account={selectedAccount}
        open={usageModalOpen}
        onOpenChange={(open) => {
          setUsageModalOpen(open);
          if (!open) {
            setSelectedAccountId("");
          }
        }}
        onRefresh={refreshAccount}
        isRefreshing={
          isRefreshingAllAccounts ||
          (!!selectedAccount && isRefreshingAccountId === selectedAccount.id)
        }
      />
      <ConfirmDialog
        open={Boolean(deleteDialogState)}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteDialogState(null);
          }
        }}
        title={
          deleteDialogState?.kind === "single"
            ? "删除账号"
            : "批量删除账号"
        }
        description={
          deleteDialogState?.kind === "single"
            ? `确定删除账号 ${deleteDialogState.account.name} 吗？删除后不可恢复。`
            : `确定删除选中的 ${deleteDialogState?.count || 0} 个账号吗？删除后不可恢复。`
        }
        confirmText="删除"
        confirmVariant="destructive"
        onConfirm={handleConfirmDelete}
      />
    </div>
  );
}
