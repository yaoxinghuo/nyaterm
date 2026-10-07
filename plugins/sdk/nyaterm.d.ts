/** Include nyaterm-sdk.js as a classic script. Await NyaTerm.ready before host calls. */
type NyaTermJson =
  | null
  | boolean
  | number
  | string
  | NyaTermJson[]
  | { [key: string]: NyaTermJson };
interface NyaTermPluginContext {
  pluginId: string;
  version: string;
  /** Host CSS variables, including --font-sans, --font-display and --font-mono. */
  theme: Record<string, string>;
  language?: string;
  monitorIntervalSeconds?: number;
}
interface NyaTermPluginSdk {
  readonly ready: Promise<NyaTermPluginContext>;
  readonly context: NyaTermPluginContext | null;
  onContextChange(
    listener: (context: NyaTermPluginContext) => void,
  ): () => void;
  call<T = NyaTermJson>(method: string, params?: NyaTermJson): Promise<T>;
  session(): Promise<{
    id: string;
    name: string;
    type: string;
    connected: boolean;
  } | null>;
  terminal: {
    read(lines?: number): Promise<{ output: string }>;
    execute(command: string, timeoutMs?: number): Promise<NyaTermJson>;
  };
  filesystem: { read(path: string): Promise<{ content: string }> };
  storage: {
    get(key: string): Promise<NyaTermJson>;
    set(key: string, value: NyaTermJson): Promise<null>;
  };
  network: {
    request(
      url: string,
      options?: {
        method?: "GET" | "POST";
        body?: string;
        contentType?: string;
      },
    ): Promise<{ status: number; contentType: string; body: string }>;
  };
  monitoring: {
    subscribe(
      monitorId: string,
      listener: (snapshot: {
        revision: number;
        sessionId: string;
        overview: NyaTermJson;
        error: boolean;
        refreshing: boolean;
        paused: boolean;
      }) => void,
    ): Promise<{ refresh(): Promise<null>; unsubscribe(): Promise<null> }>;
  };
  backend<T = NyaTermJson>(method: string, input?: NyaTermJson): Promise<T>;
}
declare const NyaTerm: NyaTermPluginSdk;
