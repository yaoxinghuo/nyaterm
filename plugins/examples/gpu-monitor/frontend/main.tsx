import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import GpuMonitor from "@/components/panel/GpuMonitor";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { RemoteGpuOverviewState } from "@/hooks/useRemoteGpuOverview";
import type { PluginMonitorSnapshot } from "@/types/plugins";
import translations from "./translations.json";
import "./gpu.css";

type Context = { language?: string; monitorIntervalSeconds?: number };
declare const NyaTerm: {
  ready: Promise<Context>;
  onContextChange(listener: (context: Context) => void): () => void;
  monitoring: {
    subscribe(
      id: string,
      listener: (snapshot: PluginMonitorSnapshot) => void,
    ): Promise<{
      refresh(): Promise<unknown>;
      unsubscribe(): Promise<unknown>;
    }>;
  };
};

i18n.use(initReactI18next).init({
  resources: translations,
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

function App() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [state, setState] = useState<RemoteGpuOverviewState>({
    sessionId: null,
    overview: null,
    error: false,
    isManualRefreshing: false,
    refresh: () => {},
  });
  useEffect(() => {
    let disposed = false;
    let subscription: Awaited<
      ReturnType<typeof NyaTerm.monitoring.subscribe>
    > | null = null;
    let sequence = 0;
    let interval: number | undefined;
    const onContext = (context: Context) => {
      void i18n.changeLanguage(context.language ?? "en");
      const changed = interval !== context.monitorIntervalSeconds;
      interval = context.monitorIntervalSeconds;
      return changed;
    };
    const unsubscribeContext = NyaTerm.onContextChange((context) => {
      const changed = onContext(context);
      // Reattach on interval changes; the host still shares the same actor.
      if (subscription && changed) void attach();
    });
    const attach = async () => {
      const request = ++sequence;
      try {
        const next = await NyaTerm.monitoring.subscribe("gpu", (snapshot) => {
          if (disposed || request !== sequence) return;
          setSessionId(snapshot.sessionId);
          setState({
            sessionId: snapshot.sessionId,
            overview: snapshot.overview,
            error: snapshot.error,
            isManualRefreshing: snapshot.refreshing,
            paused: snapshot.paused,
            refresh: () => {
              void subscription
                ?.refresh()
                .catch(() => setState((s) => ({ ...s, error: true })));
            },
          });
        });
        if (disposed || request !== sequence) {
          await next.unsubscribe();
          return;
        }
        const old = subscription;
        subscription = next;
        await old?.unsubscribe();
      } catch {
        if (!disposed && request === sequence)
          setState((s) => ({ ...s, error: true }));
      }
    };
    void (async () => {
      try {
        const context = await NyaTerm.ready;
        if (disposed) return;
        onContext(context);
        await attach();
      } catch {
        if (!disposed) setState((s) => ({ ...s, error: true }));
      }
    })();
    return () => {
      disposed = true;
      ++sequence;
      unsubscribeContext();
      void subscription?.unsubscribe().catch(() => {});
    };
  }, []);
  return (
    <TooltipProvider>
      <GpuMonitor
        activeSessionId={sessionId}
        enabled
        gpuOverviewState={state}
      />
    </TooltipProvider>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
