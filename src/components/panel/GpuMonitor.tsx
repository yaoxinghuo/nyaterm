import {
  ChevronRight,
  Fan,
  Fingerprint,
  Gauge,
  RefreshCw,
  Search,
  Thermometer,
  Zap,
} from "lucide-react";
import type { ReactNode } from "react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SiNvidia } from "react-icons/si";
import PanelHeader from "@/components/layout/PanelHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { RemoteGpuOverviewState } from "@/hooks/useRemoteGpuOverview";
import { useVirtualList } from "@/hooks/useVirtualList";
import { cn } from "@/lib/utils";
import type { RemoteGpu, RemoteGpuOverview, RemoteGpuProcess } from "@/types/global";

const GPU_PROCESS_ROW_HEIGHT = 56;
const GPU_PROCESS_LIST_MAX_HEIGHT = 320;

interface GpuMonitorProps {
  activeSessionId: string | null;
  enabled: boolean;
  gpuOverviewState: RemoteGpuOverviewState;
}

export default function GpuMonitor({
  activeSessionId,
  enabled,
  gpuOverviewState,
}: GpuMonitorProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const { error, isManualRefreshing, overview, refresh } = gpuOverviewState;

  const normalizedQuery = query.trim().toLowerCase();
  const processes = useMemo(() => {
    const items = overview?.processes ?? [];
    return items
      .filter((process) => processMatches(process, normalizedQuery))
      .sort(
        (left, right) =>
          right.used_memory_mb - left.used_memory_mb ||
          (left.gpu_index ?? 9999) - (right.gpu_index ?? 9999) ||
          left.pid - right.pid,
      );
  }, [normalizedQuery, overview?.processes]);
  const summary = useMemo(() => buildGpuSummary(overview), [overview]);

  return (
    <div
      className="nyaterm-wallpaper-transparent-surface h-full flex flex-col"
      style={{ backgroundColor: "var(--df-bg-panel)" }}
    >
      <PanelHeader
        title={t("panel.gpuMonitor")}
        meta={
          overview?.available ? (
            <span className="font-mono">
              {t("gpuMonitor.driver")} {overview.driver_version || "-"} · {t("gpuMonitor.cuda")}{" "}
              {overview.cuda_version || "-"}
            </span>
          ) : null
        }
        actions={
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 rounded-md text-muted-foreground hover:text-foreground disabled:opacity-40"
                onClick={refresh}
                disabled={!enabled || !activeSessionId || isManualRefreshing}
                aria-label={t("common.refresh")}
              >
                <RefreshCw className={cn("h-4 w-4", isManualRefreshing && "animate-spin")} />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">{t("common.refresh")}</TooltipContent>
          </Tooltip>
        }
      />

      <div className="flex-1 min-h-0 overflow-y-auto terminal-scroll p-2.5">
        {gpuOverviewState.paused && (
          <output className="block mb-2 text-xs text-muted-foreground">
            {t("gpuMonitor.paused")}
          </output>
        )}
        {error && overview && (
          <p role="alert" className="mb-2 text-xs text-destructive">
            {t("gpuMonitor.error")}
          </p>
        )}
        {!activeSessionId ? (
          <EmptyState icon={<SiNvidia />} text={t("gpuMonitor.noSession")} />
        ) : !enabled ? (
          <EmptyState icon={<SiNvidia />} text={t("gpuMonitor.disabled")} />
        ) : error && !overview ? (
          <EmptyState icon={<SiNvidia />} text={t("gpuMonitor.error")} />
        ) : !overview ? (
          <LoadingState label={t("common.loading")} />
        ) : !overview.available ? (
          <EmptyState icon={<SiNvidia />} text={t("gpuMonitor.unavailable")} />
        ) : overview.gpus.length === 0 ? (
          <EmptyState icon={<SiNvidia />} text={t("gpuMonitor.noGpus")} />
        ) : (
          <div className="space-y-2.5">
            <SummaryGrid summary={summary} />

            <div className="space-y-2">
              {overview.gpus.map((gpu) => (
                <GpuCard key={gpu.uuid || gpu.index} gpu={gpu} />
              ))}
            </div>

            <div className="space-y-2">
              <div className="relative">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder={t("gpuMonitor.search")}
                  className="h-8 pl-8 text-xs"
                />
              </div>
              {processes.length > 0 ? (
                <VirtualGpuProcessList processes={processes} />
              ) : (
                <div>
                  <div className="rounded-md border border-dashed p-4 text-center text-xs text-muted-foreground">
                    {normalizedQuery ? t("gpuMonitor.noMatches") : t("gpuMonitor.noProcesses")}
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryGrid({ summary }: { summary: GpuSummary }) {
  const { t } = useTranslation();
  return (
    <div className="grid grid-cols-2 gap-1.5">
      <SummaryItem label={t("gpuMonitor.gpus")} value={summary.gpuCount.toString()} />
      <SummaryItem label={t("gpuMonitor.maxUtilization")} value={formatPercent(summary.maxGpu)} />
      <SummaryItem
        label={t("gpuMonitor.memory")}
        value={`${formatMemoryMb(summary.memoryUsedMb)} / ${formatMemoryMb(summary.memoryTotalMb)}`}
      />
      <SummaryItem
        label={t("gpuMonitor.maxTemperature")}
        value={summary.maxTemperature == null ? "-" : `${Math.round(summary.maxTemperature)} C`}
      />
    </div>
  );
}

function SummaryItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border bg-background/35 px-2.5 py-2">
      <div className="text-[0.625rem] uppercase text-muted-foreground">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-semibold">{value}</div>
    </div>
  );
}

function GpuCard({ gpu }: { gpu: RemoteGpu }) {
  const { t } = useTranslation();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const memoryPercent =
    gpu.memory_total_mb > 0 ? (gpu.memory_used_mb / gpu.memory_total_mb) * 100 : 0;
  const gpuUtilization = gpu.utilization_gpu_percent ?? 0;

  return (
    <Collapsible open={detailsOpen} onOpenChange={setDetailsOpen}>
      <div
        className={cn(
          "rounded-md border border-l-2 bg-background/35 px-3 py-2.5",
          gpuUtilization >= 90 || memoryPercent >= 90
            ? "border-l-red-500"
            : gpuUtilization >= 70 || memoryPercent >= 70
              ? "border-l-amber-400"
              : "border-l-emerald-500",
        )}
      >
        <div className="flex min-w-0 items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <Badge
                variant="secondary"
                className="shrink-0 border border-primary/30 bg-primary/10 px-1.5 font-mono text-[0.6875rem] font-semibold text-primary"
              >
                GPU #{gpu.index}
              </Badge>
              <div className="truncate text-sm font-semibold leading-5" title={gpu.name}>
                {gpu.name || t("gpuMonitor.unknownGpu")}
              </div>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <Badge variant="outline" className="px-1.5 text-[0.625rem]">
              {gpu.pstate || "-"}
            </Badge>
            <Tooltip>
              <CollapsibleTrigger asChild>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 rounded-md text-muted-foreground hover:text-foreground"
                    aria-label={t("gpuMonitor.details")}
                  >
                    <ChevronRight
                      className={cn("h-3.5 w-3.5 transition-transform", detailsOpen && "rotate-90")}
                    />
                  </Button>
                </TooltipTrigger>
              </CollapsibleTrigger>
              <TooltipContent side="top">{t("gpuMonitor.details")}</TooltipContent>
            </Tooltip>
          </div>
        </div>

        <div className="mt-2.5 space-y-2">
          <MetricBar label={t("gpuMonitor.gpuUtilization")} value={gpuUtilization} />
          <MetricBar
            label={t("gpuMonitor.memoryUtilization")}
            detail={`${formatMemoryMb(gpu.memory_used_mb)} / ${formatMemoryMb(gpu.memory_total_mb)}`}
            value={memoryPercent}
          />
        </div>

        <CollapsibleContent className="mt-1 grid grid-cols-2 gap-1.5 overflow-hidden text-[0.6875rem] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0">
          <GpuMetric
            icon={<Fingerprint className="h-3.5 w-3.5" />}
            label={t("gpuMonitor.uuid")}
            value={gpu.uuid || "-"}
            className="col-span-2"
            valueTitle={gpu.uuid}
          />
          <GpuMetric
            icon={<Thermometer className="h-3.5 w-3.5" />}
            label={t("gpuMonitor.temperature")}
            value={gpu.temperature_c == null ? "-" : `${Math.round(gpu.temperature_c)} C`}
          />
          <GpuMetric
            icon={<Zap className="h-3.5 w-3.5" />}
            label={t("gpuMonitor.power")}
            value={formatPower(gpu.power_draw_w, gpu.power_limit_w)}
          />
          <GpuMetric
            icon={<Fan className="h-3.5 w-3.5" />}
            label={t("gpuMonitor.fan")}
            value={gpu.fan_speed_percent == null ? "-" : formatPercent(gpu.fan_speed_percent)}
          />
          <GpuMetric
            icon={<Gauge className="h-3.5 w-3.5" />}
            label={t("gpuMonitor.memoryFree")}
            value={formatMemoryMb(gpu.memory_free_mb)}
          />
        </CollapsibleContent>
      </div>
    </Collapsible>
  );
}

function MetricBar({ detail, label, value }: { detail?: string; label: string; value: number }) {
  const safeValue = clampPercent(value);
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2 text-[0.6875rem]">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono text-muted-foreground">
          {detail ?? formatPercent(safeValue)}
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-700",
            safeValue >= 90 ? "bg-red-500" : safeValue >= 70 ? "bg-amber-400" : "bg-emerald-500",
          )}
          style={{ width: `${safeValue}%` }}
        />
      </div>
    </div>
  );
}

function GpuMetric({
  className,
  icon,
  label,
  value,
  valueTitle,
}: {
  className?: string;
  icon: ReactNode;
  label: string;
  value: string;
  valueTitle?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-1.5 rounded-md bg-muted/25 px-2 py-1.5",
        className,
      )}
    >
      <span className="shrink-0 text-muted-foreground">{icon}</span>
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate text-right font-mono" title={valueTitle ?? value}>
        {value}
      </span>
    </div>
  );
}

function VirtualGpuProcessList({ processes }: { processes: RemoteGpuProcess[] }) {
  const listHeight = Math.min(
    processes.length * GPU_PROCESS_ROW_HEIGHT,
    GPU_PROCESS_LIST_MAX_HEIGHT,
  );
  const { containerRef, visibleItems, paddingTop, paddingBottom, onScroll } = useVirtualList(
    processes,
    {
      itemHeight: GPU_PROCESS_ROW_HEIGHT,
      overscan: 6,
    },
  );

  return (
    <div
      ref={containerRef}
      className="overflow-y-auto terminal-scroll"
      style={{ height: listHeight }}
      onScroll={onScroll}
    >
      <div style={{ paddingTop, paddingBottom }}>
        {visibleItems.map(({ item: process, index }) => (
          <div
            key={`${process.gpu_uuid}-${process.pid}-${index}`}
            className="pb-1.5"
            style={{ height: GPU_PROCESS_ROW_HEIGHT }}
          >
            <GpuProcessRow process={process} />
          </div>
        ))}
      </div>
    </div>
  );
}

function GpuProcessRow({ process }: { process: RemoteGpuProcess }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full items-center gap-2 rounded-md border bg-background/30 px-2.5 py-2 transition-colors hover:bg-sky-500/[0.06]">
      <div className="min-w-0 flex-1">
        <div className="truncate text-xs font-semibold" title={process.process_name}>
          {process.process_name || "-"}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 font-mono text-[0.625rem] text-muted-foreground">
          <span>
            {t("gpuMonitor.pid")} {process.pid}
          </span>
          <span>
            {t("gpuMonitor.gpu")} {process.gpu_index ?? "-"}
          </span>
        </div>
      </div>
      <span className="shrink-0 font-mono text-xs">{formatMemoryMb(process.used_memory_mb)}</span>
    </div>
  );
}

function EmptyState({ icon, text }: { icon: ReactNode; text: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-4 text-center">
      <span className="text-2xl" style={{ color: "var(--df-text-dimmed)" }}>
        {icon}
      </span>
      <span className="text-sm" style={{ color: "var(--df-text-muted)" }}>
        {text}
      </span>
    </div>
  );
}

function LoadingState({ label }: { label: string }) {
  return (
    <div className="flex h-full items-center justify-center">
      <RefreshCw className="h-5 w-5 animate-spin text-muted-foreground" aria-label={label} />
    </div>
  );
}

interface GpuSummary {
  gpuCount: number;
  maxGpu: number;
  maxTemperature: number | null;
  memoryTotalMb: number;
  memoryUsedMb: number;
}

function buildGpuSummary(overview: RemoteGpuOverview | null): GpuSummary {
  const gpus = overview?.gpus ?? [];
  const temperatures = gpus
    .map((gpu) => gpu.temperature_c)
    .filter((value): value is number => value != null);

  return {
    gpuCount: gpus.length,
    maxGpu: Math.max(0, ...gpus.map((gpu) => gpu.utilization_gpu_percent ?? 0)),
    maxTemperature: temperatures.length > 0 ? Math.max(...temperatures) : null,
    memoryTotalMb: gpus.reduce((total, gpu) => total + gpu.memory_total_mb, 0),
    memoryUsedMb: gpus.reduce((total, gpu) => total + gpu.memory_used_mb, 0),
  };
}

function processMatches(process: RemoteGpuProcess, query: string) {
  if (!query) return true;
  const haystack =
    `${process.pid} ${process.gpu_index ?? ""} ${process.gpu_uuid} ${process.process_name}`.toLowerCase();
  return haystack.includes(query);
}

function clampPercent(value: number) {
  return Math.min(100, Math.max(0, value));
}

function formatPercent(value: number) {
  return `${Math.round(clampPercent(value))}%`;
}

function formatMemoryMb(value: number) {
  if (value >= 1024) {
    const gib = value / 1024;
    return `${gib < 10 ? gib.toFixed(1) : Math.round(gib)} GiB`;
  }
  return `${Math.round(value)} MiB`;
}

function formatPower(draw?: number | null, limit?: number | null) {
  if (draw == null && limit == null) return "-";
  if (draw == null) return `- / ${formatWatts(limit)}`;
  if (limit == null) return formatWatts(draw);
  return `${formatWatts(draw)} / ${formatWatts(limit)}`;
}

function formatWatts(value?: number | null) {
  if (value == null) return "-";
  return `${value < 100 ? value.toFixed(1) : Math.round(value)} W`;
}
