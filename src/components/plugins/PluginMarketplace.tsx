import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { usePlugins } from "@/context/PluginContext";
import { getErrorMessage } from "@/lib/errors";
import { pluginApi } from "@/lib/plugins";
import type { MarketplaceCatalog, MarketplacePlugin, MarketplacePreview } from "@/types/plugins";
import { PluginDetails } from "./PluginDetails";

export function PluginMarketplace() {
  const { t } = useTranslation();
  const { plugins, refresh, locked } = usePlugins();
  const [data, setData] = useState<MarketplaceCatalog | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [review, setReview] = useState<{
    plugin: MarketplacePlugin;
    preview: MarketplacePreview;
  } | null>(null);
  const mounted = useRef(false);
  const token = useRef<string | null>(null);
  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const catalog = await pluginApi.marketplace();
      if (mounted.current) setData(catalog);
    } catch (error) {
      if (mounted.current) setError(getErrorMessage(error));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    void load();
    return () => {
      mounted.current = false;
      if (token.current) void pluginApi.cancelMarketplaceReview(token.current).catch(() => {});
      token.current = null;
    };
  }, [load]);
  const cancel = useCallback(() => {
    if (token.current) void pluginApi.cancelMarketplaceReview(token.current).catch(() => {});
    token.current = null;
    setReview(null);
  }, []);
  useEffect(() => {
    if (locked) cancel();
  }, [locked, cancel]);
  const inspect = async (plugin: MarketplacePlugin, sha256: string) => {
    setBusy(true);
    try {
      const preview = await pluginApi.inspectMarketplace(plugin.id, plugin.latestVersion, sha256);
      if (!mounted.current) {
        await pluginApi.cancelMarketplaceReview(preview.token);
        return;
      }
      token.current = preview.token;
      setReview({ plugin, preview });
    } catch (error) {
      if (mounted.current) toast.error(getErrorMessage(error));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };
  const install = async () => {
    if (!review) return;
    setBusy(true);
    try {
      await pluginApi.installMarketplace(review.preview.token);
      await refresh();
      toast.success(t("plugins.installed"));
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      token.current = null;
      if (mounted.current) {
        setReview(null);
        setBusy(false);
      }
    }
  };
  const matches =
    data?.catalog.plugins.filter((p) =>
      `${p.name} ${p.id} ${p.publisher} ${p.description} ${p.tags.join(" ")}`
        .toLocaleLowerCase()
        .includes(query.trim().toLocaleLowerCase()),
    ) ?? [];
  return (
    <div className="space-y-4 p-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{t("plugins.store.official")}</h3>
        <Button size="sm" variant="outline" disabled={loading || busy} onClick={() => void load()}>
          {t("plugins.store.refresh")}
        </Button>
      </div>
      <Input
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t("plugins.store.search")}
        aria-label={t("plugins.store.search")}
      />
      {loading && <p className="text-sm text-muted-foreground">{t("plugins.loading")}</p>}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!loading && !error && !matches.length && (
        <p className="rounded-md border p-5 text-sm text-muted-foreground">
          {t("plugins.store.empty")}
        </p>
      )}
      {!loading &&
        !error &&
        matches.map((plugin) => {
          const version = plugin.versions.find((v) => v.version === plugin.latestVersion);
          const artifact =
            version?.artifacts.find((a) => a.target === data?.target) ??
            version?.artifacts.find((a) => a.target === "universal");
          const installed = plugins.find((p) => p.id === plugin.id);
          const current = installed?.activeVersion === plugin.latestVersion;
          const local =
            installed &&
            installed.versions[installed.activeVersion]?.provenance?.source !== "marketplace";
          return (
            <div key={plugin.id} className="space-y-3 rounded-lg border bg-card p-4">
              <h4 className="font-semibold">
                {plugin.name}{" "}
                <span className="text-xs font-normal text-muted-foreground">
                  {plugin.latestVersion}
                </span>
              </h4>
              <p className="text-xs text-muted-foreground">
                {t("plugins.publisher", { name: plugin.publisher })}{" "}
                {plugin.verified && (
                  <Badge variant="secondary">{t("plugins.store.publisherVerified")}</Badge>
                )}
              </p>
              <p className="text-sm text-muted-foreground">{plugin.description}</p>
              <div className="flex flex-wrap gap-1">
                {plugin.tags.map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))}
                {version?.permissions.includes("native") && (
                  <Badge variant="outline">{t("plugins.native")}</Badge>
                )}
              </div>
              {local && (
                <p className="text-xs text-muted-foreground">{t("plugins.store.sourceConflict")}</p>
              )}
              <Button
                size="sm"
                disabled={busy || locked || current || !artifact || Boolean(local)}
                onClick={() => artifact && void inspect(plugin, artifact.sha256)}
              >
                {t(
                  !artifact
                    ? "plugins.store.unsupported"
                    : current
                      ? "plugins.store.installedTab"
                      : installed
                        ? "plugins.store.update"
                        : "plugins.install",
                )}
              </Button>
            </div>
          );
        })}
      {review && (
        <PluginDetails
          plugin={review.plugin}
          preview={review.preview}
          busy={busy}
          locked={locked}
          onCancel={cancel}
          onInstall={() => void install()}
        />
      )}
    </div>
  );
}
