import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { Icon } from "./icons";
import {
  cancelHuggingFace,
  cancelNativeStream,
  downloadHuggingFaceFile,
  fetchHuggingFaceFiles,
  getHuggingFaceToken,
  hasNativeStreaming,
  llamaVersion,
  pollNativeEvent,
  prepareNativeModel,
  requestNativeModels,
  searchHuggingFace,
  setHuggingFaceToken,
  startNativeStream,
  type GenStats,
  type HfFile,
  type HfModel,
} from "./native";
import {
  chats,
  device,
  initialMessages,
  models,
  type Chat,
  type Message,
  type Model,
  type Section,
} from "./data";

export type ThemePref = "system" | "light" | "dark" | "night" | "amoled";

const THEMES: { id: ThemePref; label: string; paper: string; ink: string; accent: string }[] = [
  { id: "system", label: "System", paper: "#F5F2EA", ink: "#171813", accent: "#245246" },
  { id: "light", label: "Light", paper: "#F5F2EA", ink: "#171813", accent: "#245246" },
  { id: "dark", label: "Dark", paper: "#1B1C1A", ink: "#EDEAE2", accent: "#8FCFAD" },
  { id: "night", label: "Night", paper: "#201C17", ink: "#F2EADB", accent: "#9AC7A5" },
  { id: "amoled", label: "AMOLED", paper: "#000000", ink: "#E8E4DA", accent: "#86C9A6" },
];

const THEME_COLOR: Record<string, string> = {
  light: "#F5F2EA",
  dark: "#1B1C1A",
  night: "#201C17",
  amoled: "#000000",
};

function readThemePref(): ThemePref {
  try {
    const raw = window.Sllm?.themePref?.() ?? localStorage.getItem("sllm.theme");
    if (raw && THEMES.some((t) => t.id === raw)) return raw as ThemePref;
  } catch {
    // no bridge and no storage
  }
  return "system";
}

function resolvedTheme(pref: ThemePref): Exclude<ThemePref, "system"> {
  if (pref !== "system") return pref;
  try {
    const native = window.Sllm?.theme?.();
    if (native === "light" || native === "dark" || native === "night" || native === "amoled") {
      return native;
    }
  } catch {
    // ignore
  }
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(pref: ThemePref): void {
  try {
    localStorage.setItem("sllm.theme", pref);
    window.Sllm?.setTheme?.(pref);
  } catch {
    // ignore storage/bridge failures
  }
  const resolved = resolvedTheme(pref);
  const root = document.documentElement;
  root.setAttribute("data-theme-switching", "");
  root.dataset.theme = resolved;
  root.style.colorScheme = resolved === "light" ? "light" : "dark";
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", THEME_COLOR[resolved] ?? THEME_COLOR.light);
  requestAnimationFrame(() =>
    requestAnimationFrame(() => root.removeAttribute("data-theme-switching")),
  );
}

const NAV: { id: Section; label: string; icon: "chat" | "layers" | "sliders" }[] = [
  { id: "chats", label: "Chats", icon: "chat" },
  { id: "models", label: "Models", icon: "layers" },
  { id: "settings", label: "Settings", icon: "sliders" },
];

function modelById(id: string): Model {
  return models.find((m) => m.id === id) ?? models[2];
}

function family(id: string): "clover" | "bonsai" | "qwen" {
  const key = id.toLowerCase();
  if (key.includes("clover")) return "clover";
  if (key.includes("bonsai")) return "bonsai";
  return "qwen";
}

/** Status line under an assistant message, filled from `done.stats`. */
function formatStats(stats: GenStats): string {
  const parts: string[] = [];
  if (Number.isFinite(stats.tokPerS)) parts.push(`${stats.tokPerS.toFixed(1)} tok/s`);
  if (Number.isFinite(stats.ttftMs)) parts.push(`${(stats.ttftMs / 1000).toFixed(2)} s TTFT`);
  parts.push(`${stats.generatedTokens} tok`);
  if (stats.mock) parts.push("mock");
  if (stats.canceled) parts.push("stopped");
  return parts.join(" · ");
}

function ModelMark({ id, size = 40 }: { id: string; size?: number }) {
  const key = id.toLowerCase();
  const letter = key.includes("clover") ? "C" : key.includes("bonsai") ? "B" : "Q";
  return (
    <span class="model-mark" style={{ width: `${size}px`, height: `${size}px` }}>
      {letter}
    </span>
  );
}

function Rail({
  section,
  onSection,
}: {
  section: Section;
  onSection: (s: Section) => void;
}) {
  return (
    <aside class="rail">
      <div class="rail-brand" aria-label="Lumen">
        <span class="brand-mark">L</span>
        <span class="brand-name">LUMEN</span>
      </div>
      <nav class="rail-nav">
        {NAV.map((item) => (
          <button
            key={item.id}
            class={`rail-item ${section === item.id ? "is-active" : ""}`}
            onClick={() => onSection(item.id)}
          >
            <Icon name={item.icon} size={21} />
            <span>{item.label}</span>
          </button>
        ))}
      </nav>
      <div class="rail-foot">
        <span class="rail-device">
          <span class="status-dot" />
          {device.ram}
        </span>
        <span class="rail-soc">{device.soc}</span>
      </div>
    </aside>
  );
}

function BottomNav({
  section,
  onSection,
}: {
  section: Section;
  onSection: (s: Section) => void;
}) {
  return (
    <nav class="bottom-nav" aria-label="Primary">
      {NAV.map((item) => (
        <button
          key={item.id}
          class={`bottom-item ${section === item.id ? "is-active" : ""}`}
          onClick={() => onSection(item.id)}
        >
          <Icon name={item.icon} size={21} />
          <span>{item.label}</span>
        </button>
      ))}
    </nav>
  );
}

function dayLabel(updatedAt?: number): string {
  if (!updatedAt) return "Recent";
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (updatedAt >= startOfToday) return "Today";
  if (updatedAt >= startOfToday - 86_400_000) return "Yesterday";
  if (updatedAt >= startOfToday - 7 * 86_400_000) return "Previous 7 days";
  return "Older";
}

function ChatsList({
  chats,
  selected,
  onSelect,
  onNew,
  compact = false,
}: {
  chats: Chat[];
  selected: string;
  onSelect: (id: string) => void;
  onNew: () => void;
  compact?: boolean;
}) {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return chats;
    return chats.filter((c) =>
      `${c.title} ${c.snippet} ${c.model}`.toLowerCase().includes(q),
    );
  }, [query, chats]);
  const pinned = filtered.filter((c) => c.pinned);
  const today = new Date().toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
  const rest = filtered
    .filter((c) => !c.pinned)
    .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
  const groups: { label: string; items: Chat[] }[] = [];
  for (const chat of rest) {
    const label = dayLabel(chat.updatedAt);
    const last = groups[groups.length - 1];
    if (last && last.label === label) last.items.push(chat);
    else groups.push({ label, items: [chat] });
  }

  return (
    <section class={`chats-screen ${compact ? "is-compact" : ""}`}>
      <header class="screen-head">
        <div>
          <p class="eyebrow">{today}</p>
          <h1>Chats</h1>
        </div>
        <button class="icon-button is-accent" onClick={onNew} aria-label="New chat">
          <Icon name="plus" size={20} />
        </button>
      </header>

      <label class="search">
        <Icon name="search" size={18} />
        <input
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          placeholder="Search chats"
          aria-label="Search chats"
        />
      </label>

      <div class="chat-list">
        {pinned.length > 0 && <p class="list-label">Pinned</p>}
        {pinned.map((chat) => (
          <ChatRow key={chat.id} chat={chat} active={chat.id === selected} onClick={() => onSelect(chat.id)} />
        ))}
        {groups.map((group) => (
          <div key={group.label} class="chat-group">
            <p class="list-label">{group.label}</p>
            {group.items.map((chat) => (
              <ChatRow
                key={chat.id}
                chat={chat}
                active={chat.id === selected}
                onClick={() => onSelect(chat.id)}
              />
            ))}
          </div>
        ))}
        {filtered.length === 0 && (
          <div class="empty">
            <Icon name="search" size={22} />
            <p>No chats match “{query}”.</p>
          </div>
        )}
      </div>
    </section>
  );
}

function ChatRow({
  chat,
  active,
  onClick,
}: {
  chat: Chat;
  active: boolean;
  onClick: () => void;
}) {
  const model = chat.model.includes("Clover")
    ? "clover"
    : chat.model.includes("Bonsai")
      ? "bonsai"
      : "qwen";
  return (
    <button class={`chat-row ${active ? "is-active" : ""}`} onClick={onClick}>
      <ModelMark id={model} size={38} />
      <span class="chat-copy">
        <span class="chat-title">{chat.title}</span>
        <span class="chat-snippet">{chat.snippet}</span>
        <span class="chat-meta">
          <span class={`mini-dot ${model}`} />
          {chat.model}
        </span>
      </span>
      <span class="chat-time">{chat.time}</span>
    </button>
  );
}

function AssistantBody({ text, streaming }: { text: string; streaming?: boolean }) {
  const lead = text.trimStart();
  if (!lead.startsWith("<think>")) {
    return (
      <>
        {text}
        {streaming && <span class="caret" />}
      </>
    );
  }
  const close = lead.indexOf("</think>");
  const reasoning = close < 0 ? lead.slice(7) : lead.slice(7, close);
  const answer = close < 0 ? "" : lead.slice(close + 8).trimStart();
  return (
    <>
      <details class="reasoning" open={!answer && Boolean(streaming)}>
        <summary>
          <span class="reasoning-dot" />
          Reasoning
          <span class="reasoning-hint">{answer ? "tap to open" : "live"}</span>
        </summary>
        <div class="reasoning-text">{reasoning}</div>
      </details>
      {answer && <div>{answer}</div>}
      {streaming && <span class="caret" />}
    </>
  );
}

function ChatPane({
  chat,
  messages,
  draft,
  streaming,
  engineModel,
  onDraft,
  onSend,
  onStop,
  onComposerFocus,
  onBack,
  isTablet,
}: {
  chat: Chat;
  messages: Message[];
  draft: string;
  streaming: boolean;
  engineModel?: Model;
  onDraft: (v: string) => void;
  onSend: () => void;
  onStop: () => void;
  onComposerFocus: (focused: boolean) => void;
  onBack: () => void;
  isTablet: boolean;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const focusTimer = useRef<number | null>(null);
  // Only follow new tokens when the user is already near the bottom; scrolling
  // up by more than 64 px during a stream keeps their position.
  const stickToBottom = useRef(true);
  const model =
    engineModel ??
    modelById(chat.model.includes("Clover") ? "clover" : chat.model.includes("Bonsai") ? "bonsai" : "qwen");
  const shortName = model.name.split(" ")[0];

  useEffect(() => {
    if (!stickToBottom.current) return;
    const first = requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const el = listRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    });
    return () => cancelAnimationFrame(first);
  }, [messages]);

  useEffect(
    () => () => {
      if (focusTimer.current) window.clearTimeout(focusTimer.current);
      onComposerFocus(false);
    },
    [onComposerFocus],
  );

  return (
    <section class="chat-pane">
      <header class="chat-head">
        <button class="icon-button chat-back" onClick={onBack} aria-label="Back to chats">
          <Icon name="back" size={21} />
        </button>
        <div class="chat-title-block">
          <div class="chat-title-line">
            <span class="status-dot" />
            <h2>{chat.title}</h2>
          </div>
          <p class="chat-subtitle">
            {model.name} · {model.quant} · {model.layers} layers
          </p>
        </div>
      </header>

      <div
        class="messages"
        ref={listRef}
        aria-live="polite"
        onScroll={(e) => {
          const el = e.currentTarget;
          stickToBottom.current =
            el.scrollHeight - el.scrollTop - el.clientHeight <= 64;
        }}
      >
        {messages.map((m) =>
          m.role === "user" ? (
            <article key={m.id} class="msg user">
              <p class="role">You</p>
              <div class="user-bubble">{m.text}</div>
            </article>
          ) : (
            <article key={m.id} class="msg assistant">
              <div class="assistant-role">
                <ModelMark id={model.id} size={26} />
                <p class="role">{model.name}</p>
                {m.streaming && <span class="live">streaming</span>}
              </div>
              <div class="assistant-body">
                <AssistantBody text={m.text} streaming={m.streaming} />
              </div>
              {m.error && (
                <p class="msg-error" role="alert">
                  {m.error}
                </p>
              )}
              {m.meta && <p class="msg-meta">{m.meta}</p>}
            </article>
          ),
        )}
      </div>

      <div class="composer-wrap">
        <form
          class="composer"
          onPointerDown={(e) => {
            const target = e.target as HTMLElement;
            if (!target.closest("button")) {
              inputRef.current?.focus();
            }
          }}
          onSubmit={(e) => {
            e.preventDefault();
            stickToBottom.current = true;
            onSend();
          }}
        >
          <textarea
            ref={inputRef}
            class="composer-input"
            rows={1}
            value={draft}
            placeholder={`Message ${shortName}…`}
            aria-label="Message"
            onFocus={() => {
              if (focusTimer.current) window.clearTimeout(focusTimer.current);
              // Delayed: hiding the bottom nav during the tap shifts layout and
              // makes WebView drop focus before the IME connects.
              focusTimer.current = window.setTimeout(() => {
                focusTimer.current = null;
                onComposerFocus(true);
              }, 200);
            }}
            onBlur={() => {
              if (focusTimer.current) {
                window.clearTimeout(focusTimer.current);
                focusTimer.current = null;
              }
              onComposerFocus(false);
            }}
            onInput={(e) => onDraft((e.target as HTMLTextAreaElement).value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                stickToBottom.current = true;
                onSend();
              }
            }}
          />
          <button
            type="button"
            class={`send-button ${streaming ? "is-stop" : ""}`}
            disabled={!streaming && !draft.trim()}
            aria-label={streaming ? "Stop generating" : "Send message"}
            onPointerDown={(e) => {
              e.preventDefault();
              if (streaming) {
                onStop();
              } else {
                stickToBottom.current = true;
                onSend();
              }
            }}
          >
            <Icon name={streaming ? "stop" : "send"} size={streaming ? 17 : 19} />
          </button>
        </form>
        <p class="composer-hint">
          {isTablet ? "Enter to send · Shift+Enter for a new line" : "Local model · nothing leaves this device"}
        </p>
      </div>
    </section>
  );
}

function ModelsScreen({
  active,
  onActive,
  list,
  ramGb,
  status,
}: {
  active: string;
  onActive: (id: string) => void;
  list: Model[];
  ramGb: number;
  status: Record<string, "loading" | "ready" | "error">;
}) {
  const [filter, setFilter] = useState("All");
  const [source, setSource] = useState<"installed" | "hf">("installed");
  const filters = ["All", "Text", "MoE", "LoRA"];
  const visible = list.filter((m) => {
    if (filter === "All") return true;
    return m.caps.some((c) => c.toLowerCase().includes(filter.toLowerCase()));
  });
  const totalUsed = list.reduce((n, m) => n + m.sizeGb, 0);

  return (
    <section class="models-screen">
      <header class="screen-head">
        <div>
          <p class="eyebrow">
            {source === "hf" ? "Hugging Face · search & download" : `Local GGUF · ${list.length} installed`}
          </p>
          <h1>Models</h1>
        </div>
        <div class="source-tabs" role="tablist" aria-label="Model source">
          <button
            class={source === "installed" ? "is-active" : ""}
            onClick={() => setSource("installed")}
          >
            Installed
          </button>
          <button class={source === "hf" ? "is-active" : ""} onClick={() => setSource("hf")}>
            Hugging Face
          </button>
        </div>
      </header>

      {source === "hf" ? (
        <HuggingFacePanel ramGb={ramGb} />
      ) : (
        <>
          <div class="storage-card">
        <div class="storage-top">
          <span class="storage-icon">
            <Icon name="drive" size={19} />
          </span>
          <div>
            <p class="storage-title">Model storage</p>
            <p class="storage-sub">{totalUsed.toFixed(1)} GB used · local files</p>
          </div>
          <span class="device-chip">
            <Icon name="cpu" size={14} />
            {device.name} · {device.ram}
          </span>
        </div>
        <div class="storage-meter" role="img" aria-label={`${totalUsed.toFixed(1)} gigabytes of models`}>
          <span style={{ width: `${Math.min(100, Math.round((totalUsed / 64) * 100))}%` }} />
        </div>
        <div class="storage-legend">
          {list.map((m) => (
            <span key={m.id}>
              <i class={`legend-dot ${family(m.id)}`} />
              {m.name.split(" ")[0]} {m.size}
            </span>
          ))}
        </div>
      </div>

      <div class="filter-row" role="tablist" aria-label="Model filters">
        {filters.map((f) => (
          <button
            key={f}
            class={`filter ${filter === f ? "is-active" : ""}`}
            onClick={() => setFilter(f)}
            role="tab"
            aria-selected={filter === f}
          >
            {f}
          </button>
        ))}
      </div>

      <div class="model-grid">
        {visible.map((model) => (
          <ModelCard
            key={model.id}
            model={model}
            active={model.id === active}
            ramGb={ramGb}
            status={status[model.id]}
            onActive={() => onActive(model.id)}
          />
        ))}
      </div>
        </>
      )}
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "unknown size";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function formatCount(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

function estimateRamGb(bytes: number): number {
  // Weights + KV/compute reserve. KV depends on ctx/layers; this is a
  // deliberately conservative envelope, not a promise for every arch.
  return Math.max(0.6, (bytes / 1_073_741_824) * 1.12 + 0.55);
}

function HuggingFacePanel({ ramGb }: { ramGb: number }) {
  const [query, setQuery] = useState("");
  const [author, setAuthor] = useState("");
  const [sort, setSort] = useState("relevance");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [results, setResults] = useState<HfModel[]>([]);
  const [onlyFavorites, setOnlyFavorites] = useState(false);
  const [openRepo, setOpenRepo] = useState<string | null>(null);
  const [filesByRepo, setFilesByRepo] = useState<Record<string, HfFile[]>>({});
  const [downloads, setDownloads] = useState<
    Record<string, { received: number; total: number; state: string }>
  >({});
  const [token, setToken] = useState(getHuggingFaceToken);
  const [favorites, setFavorites] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("lumen.hf.favorites") ?? "[]") as string[];
    } catch {
      return [];
    }
  });

  useEffect(() => {
    try {
      localStorage.setItem("lumen.hf.favorites", JSON.stringify(favorites));
    } catch {
      // ignore
    }
  }, [favorites]);

  useEffect(() => {
    window.sllmHfResults = (payload) => {
      setBusy(false);
      if (!payload.ok) {
        setError(payload.error ?? "Hugging Face request failed");
        return;
      }
      setError("");
      setResults(payload.models ?? []);
    };
    window.sllmHfFiles = (payload) => {
      if (!payload.ok || !payload.repo) {
        setError(payload.error ?? "Could not read repository files");
        return;
      }
      setFilesByRepo((prev) => ({ ...prev, [payload.repo!]: payload.files ?? [] }));
    };
    window.sllmHfProgress = (payload) => {
      const key = `${payload.repo}::${payload.file}`;
      setDownloads((prev) => ({
        ...prev,
        [key]: { received: payload.received, total: payload.total, state: payload.state },
      }));
    };
    window.sllmHfDone = (payload) => {
      if (!payload.repo || !payload.file) return;
      const key = `${payload.repo}::${payload.file}`;
      setDownloads((prev) => ({
        ...prev,
        [key]: {
          received: payload.bytes ?? prev[key]?.received ?? 0,
          total: prev[key]?.total ?? payload.bytes ?? 0,
          state: payload.ok ? "done" : payload.canceled ? "canceled" : "error",
        },
      }));
      if (!payload.ok) setError(payload.error ?? "Download failed");
    };
    return () => {
      delete window.sllmHfResults;
      delete window.sllmHfFiles;
      delete window.sllmHfProgress;
      delete window.sllmHfDone;
    };
  }, []);

  function run() {
    setError("");
    setBusy(true);
    if (!searchHuggingFace({ query, author, sort })) {
      setBusy(false);
      setError("Hugging Face bridge is not available in this build.");
    }
  }

  function toggleFiles(repo: string) {
    if (openRepo === repo) {
      setOpenRepo(null);
      return;
    }
    setOpenRepo(repo);
    if (!filesByRepo[repo]) fetchHuggingFaceFiles(repo);
  }

  function toggleFavorite(repo: string) {
    setFavorites((prev) =>
      prev.includes(repo) ? prev.filter((id) => id !== repo) : [...prev, repo],
    );
  }

  function saveToken() {
    setHuggingFaceToken(token.trim());
    setError("Token saved locally on this device.");
  }

  const visible = onlyFavorites ? results.filter((m) => favorites.includes(m.id)) : results;

  return (
    <div class="hf-panel">
      <form
        class="hf-search"
        onSubmit={(event) => {
          event.preventDefault();
          run();
        }}
      >
        <input
          value={query}
          onInput={(event) => setQuery((event.target as HTMLInputElement).value)}
          placeholder="Search GGUF models, e.g. Qwen2.5 0.5B"
          aria-label="Search Hugging Face"
        />
        <input
          value={author}
          onInput={(event) => setAuthor((event.target as HTMLInputElement).value)}
          placeholder="Author (any case)"
          aria-label="Filter by author"
          class="hf-author"
        />
        <select
          value={sort}
          onChange={(event) => setSort((event.target as HTMLSelectElement).value)}
          aria-label="Sort results"
        >
          <option value="relevance">Relevance</option>
          <option value="downloads">Downloads</option>
          <option value="likes">Likes</option>
          <option value="updated">Updated</option>
          <option value="created">Created</option>
        </select>
        <button type="button" class="use-button" onClick={run} disabled={busy}>
          {busy ? "Searching…" : "Search"}
        </button>
      </form>

      <div class="hf-tools">
        <button
          class={`filter ${onlyFavorites ? "is-active" : ""}`}
          onClick={() => setOnlyFavorites((value) => !value)}
        >
          ★ Favorites · {favorites.length}
        </button>
        <span class="hf-hint">Only GGUF repositories. Author match ignores case.</span>
      </div>

      <details class="hf-token">
        <summary>Gated models need a Hugging Face token</summary>
        <div>
          <input
            type="password"
            value={token}
            onInput={(event) => setToken((event.target as HTMLInputElement).value)}
            placeholder="hf_…"
            aria-label="Hugging Face token"
          />
          <button type="button" class="use-button" onClick={saveToken}>
            Save
          </button>
        </div>
        <p>Stored in app preferences. Never sent anywhere except huggingface.co.</p>
      </details>

      {error && <p class="msg-error hf-error">{error}</p>}
      {busy && <p class="hf-status">Contacting huggingface.co…</p>}
      {!busy && !error && results.length === 0 && (
        <p class="hf-status">
          Search the public GGUF catalogue. Downloads go to this device and are
          resumable; nothing is installed until you finish a download.
        </p>
      )}

      <div class="hf-list">
        {visible.map((model) => {
          const files = filesByRepo[model.id];
          const open = openRepo === model.id;
          const favorite = favorites.includes(model.id);
          return (
            <article class={`hf-card ${open ? "is-open" : ""}`} key={model.id}>
              <div class="hf-card-head">
                <div>
                  <h3>{model.id.split("/").slice(1).join("/") || model.id}</h3>
                  <p>
                    {model.author} · {formatCount(model.downloads)} downloads ·{" "}
                    {formatCount(model.likes)} likes
                  </p>
                </div>
                <div class="hf-card-actions">
                  {model.gated && <span class="chip chip-warn">Gated</span>}
                  <button
                    class={`icon-button ${favorite ? "is-accent" : ""}`}
                    aria-label={favorite ? "Remove from favorites" : "Add to favorites"}
                    onClick={() => toggleFavorite(model.id)}
                  >
                    {favorite ? "★" : "☆"}
                  </button>
                </div>
              </div>
              <div class="chips">
                <span class="chip">GGUF</span>
                {model.tags?.includes("moe") && <span class="chip">MoE</span>}
                {model.tags?.includes("ternary") && <span class="chip">Ternary</span>}
                {model.pipelineTag && <span class="chip">{model.pipelineTag}</span>}
                {model.lastModified && (
                  <span class="chip">updated {model.lastModified.slice(0, 10)}</span>
                )}
              </div>
              <button class="use-button hf-open" onClick={() => toggleFiles(model.id)}>
                {open ? "Hide files" : "Files & download"}
              </button>

              {open && (
                <div class="hf-files">
                  {!files && <p class="hf-status">Reading file list…</p>}
                  {files?.length === 0 && <p class="hf-status">No GGUF files in this repo.</p>}
                  {files?.map((file) => {
                    const key = `${model.id}::${file.name}`;
                    const progress = downloads[key];
                    const need = estimateRamGb(file.size);
                    const moe =
                      model.tags?.includes("moe") ||
                      /moe|a\d+b|expert/i.test(model.id);
                    const ternary =
                      model.tags?.includes("ternary") ||
                      /q2_0|tq1_0|tq2_0|ternary/i.test(file.name);
                    const label =
                      file.kind === "mmproj"
                        ? "Vision projector"
                        : file.kind === "lora"
                          ? "LoRA adapter"
                          : "Model";
                    return (
                      <div class="hf-file" key={file.name}>
                        <div class="hf-file-info">
                          <strong>{file.name}</strong>
                          <span>
                            {label} · {formatBytes(file.size)}
                            {file.kind === "model" && ` · needs ~${need.toFixed(1)} GB`}
                          </span>
                          {file.kind === "model" && (
                            <span class={moe || need <= ramGb ? "fit-ok" : "fit-warn"}>
                              {moe
                                ? "MoE: storage-bound, mmap may work"
                                : need <= ramGb
                                  ? "Fits in RAM"
                                  : "Likely too large for this phone"}
                            </span>
                          )}
                          {ternary && <span class="fit-warn">Ternary: engine support required</span>}
                        </div>
                        <div class="hf-file-actions">
                          {progress?.state === "downloading" && progress.total > 0 && (
                            <span class="hf-progress">
                              {Math.round((progress.received / progress.total) * 100)}%
                            </span>
                          )}
                          {progress?.state === "done" ? (
                            <span class="fit-ok">Downloaded</span>
                          ) : (
                            <button
                              class="use-button"
                              onClick={() => {
                                setError("");
                                if (!downloadHuggingFaceFile(model.id, file.name)) {
                                  setError("Download bridge is not available.");
                                }
                              }}
                            >
                              {progress?.state === "downloading" ? "Resume" : "Download"}
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                  {files && files.length > 0 && (
                    <button
                      class="filter"
                      onClick={() => cancelHuggingFace()}
                      title="Cancel the current download"
                    >
                      Cancel download
                    </button>
                  )}
                </div>
              )}
            </article>
          );
        })}
      </div>
    </div>
  );
}

function ModelCard({
  model,
  active,
  ramGb,
  status,
  onActive,
}: {
  model: Model;
  active: boolean;
  ramGb: number;
  status?: "loading" | "ready" | "error";
  onActive: () => void;
}) {
  const fits = model.estimateGb <= ramGb;
  const pct = Math.min(100, Math.round((model.estimateGb / ramGb) * 100));
  return (
    <article class={`model-card ${active ? "is-active" : ""}`}>
      <div class="model-top">
        <ModelMark id={model.id} size={44} />
        <div class="model-title">
          <h3>{model.name}</h3>
          <p>
            {model.arch} · {model.layers} layers · {model.context}
          </p>
        </div>
        <span class="quant">{model.quant}</span>
      </div>

      <div class="chips">
        {model.caps.map((cap) => (
          <span key={cap} class="chip">
            {cap}
          </span>
        ))}
      </div>

      {model.hasLora && (
        <p class="model-adapter">
          <span>LoRA · runtime</span>
          <span>{model.loraCount ?? 1} adapter(s) attached, weights untouched</span>
        </p>
      )}
      {model.hasMmproj && (
        <p class="model-adapter is-warn">
          <span>mmproj found</span>
          <span>vision is not enabled by the engine in this beta</span>
        </p>
      )}

      <p class="model-note">{model.note}</p>

      <div class="fit">
        <div class="fit-line">
          <span class={fits ? "fit-ok" : "fit-warn"}>
            <Icon name={model.status === "unsupported" ? "info" : fits ? "check" : "info"} size={13} />
            {model.status === "unsupported"
              ? "Unsupported by this build"
              : fits
                ? "Fits in RAM"
                : "Storage-bound"}
          </span>
          <span class="fit-numbers">
            {model.estimateGb.toFixed(1)} / {ramGb.toFixed(1)} GB RAM
          </span>
        </div>
        <div class={`fit-meter ${fits ? "ok" : "warn"}`}>
          <span style={{ width: `${pct}%` }} />
        </div>
      </div>

      <div class="model-foot">
        <div class="model-size">
          <strong>{model.size}</strong>
          <span>on disk</span>
        </div>
        <button class={`use-button ${active ? "is-active" : ""}`} onClick={onActive}>
          {status === "loading" ? (
            "Loading…"
          ) : status === "error" ? (
            "Load failed"
          ) : active ? (
            <>
              <Icon name="check" size={15} /> Active
            </>
          ) : status === "ready" ? (
            "Loaded"
          ) : (
            "Set active"
          )}
        </button>
      </div>
    </article>
  );
}

function SettingsScreen({
  theme,
  onTheme,
  settings,
  onSettings,
}: {
  theme: ThemePref;
  onTheme: (pref: ThemePref) => void;
  settings: GenSettings;
  onSettings: (next: GenSettings) => void;
}) {
  const [reduced, setReduced] = useState(() => {
    try {
      const stored = localStorage.getItem("sllm.motion");
      if (stored) return stored === "off";
    } catch {
      // ignore
    }
    return matchMedia("(prefers-reduced-motion: reduce)").matches;
  });

  function setMotion(off: boolean) {
    setReduced(off);
    document.documentElement.dataset.motion = off ? "off" : "on";
    try {
      localStorage.setItem("sllm.motion", off ? "off" : "on");
    } catch {
      // ignore
    }
  }

  return (
    <section class="settings-screen">
      <header class="screen-head">
        <div>
          <p class="eyebrow">Device · {device.name}</p>
          <h1>Settings</h1>
        </div>
      </header>

      <Group title="Generation">
        <label class="setting-row">
          <span>Temperature</span>
          <input
            class="setting-input"
            type="number"
            min="0"
            max="2"
            step="0.05"
            value={settings.temperature}
            onInput={(event) =>
              onSettings({
                ...settings,
                temperature: Math.min(2, Math.max(0, Number((event.target as HTMLInputElement).value))),
              })
            }
          />
        </label>
        <label class="setting-row">
          <span>Top-p</span>
          <input
            class="setting-input"
            type="number"
            min="0.05"
            max="1"
            step="0.05"
            value={settings.topP}
            onInput={(event) =>
              onSettings({
                ...settings,
                topP: Math.min(1, Math.max(0.05, Number((event.target as HTMLInputElement).value))),
              })
            }
          />
        </label>
        <label class="setting-row">
          <span>Max tokens</span>
          <input
            class="setting-input"
            type="number"
            min="16"
            max="8192"
            step="16"
            value={settings.maxTokens}
            onInput={(event) =>
              onSettings({
                ...settings,
                maxTokens: Math.min(8192, Math.max(16, Math.round(Number((event.target as HTMLInputElement).value)))),
              })
            }
          />
        </label>
        <label class="setting-row setting-row-prompt">
          <span>System prompt</span>
          <textarea
            class="setting-input setting-prompt"
            rows={2}
            value={settings.systemPrompt}
            placeholder="Default"
            onInput={(event) =>
              onSettings({ ...settings, systemPrompt: (event.target as HTMLTextAreaElement).value })
            }
          />
        </label>
        <p class="group-note">Applied to the next message. No restart needed.</p>
      </Group>

      <Group title="Performance">
        <Row label="Threads" value="4 · big cores" />
        <Row label="Context" value="4096" />
        <Row label="KV cache" value="q8_0" />
        <Row label="Backend" value="CPU" />
      </Group>

      <Group title="Appearance">
        <div class="theme-picker">
          <div class="theme-picker-head">
            <span>Theme</span>
            <span>{THEMES.find((t) => t.id === theme)?.label ?? "System"}</span>
          </div>
          <div class="theme-grid">
            {THEMES.map((t) => (
              <button
                key={t.id}
                class={`theme-option ${theme === t.id ? "is-active" : ""}`}
                onClick={() => onTheme(t.id)}
                aria-pressed={theme === t.id}
              >
                <span
                  class="theme-swatch"
                  style={{ background: t.paper, borderColor: t.ink }}
                >
                  <i style={{ background: t.accent }} />
                </span>
                <span class="theme-name">{t.label}</span>
                {theme === t.id && <Icon name="check" size={14} />}
              </button>
            ))}
          </div>
        </div>
        <Row label="Text size" value="Default" />
        <Toggle label="Reduce motion" value={reduced} onChange={setMotion} />
      </Group>

      <Group title="Privacy">
        <Row label="Prompt logging" value="Off" />
        <p class="group-note">
          Chats and prompts stay on this device. Nothing is uploaded.
        </p>
      </Group>

      <Group title="About">
        <Row label="Version" value="0.2.0-beta.1 · Rust" />
        <Row label="Engine" value={`llama.cpp · ${llamaVersion()}`} />
        <Row label="Licenses" value="Open source" />
      </Group>

      <p class="settings-foot">
        Lumen · Rust core · local inference
      </p>
    </section>
  );
}

function Group({ title, children }: { title: string; children: ComponentChildren }) {
  return (
    <section class="group">
      <h3>{title}</h3>
      <div class="group-card">{children}</div>
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div class="setting-row">
      <span>{label}</span>
      <span class="setting-value">{value}</span>
    </div>
  );
}

function Toggle({
  label,
  value,
  onChange,
}: {
  label: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div class="setting-row">
      <span>{label}</span>
      <button
        class={`toggle ${value ? "is-on" : ""}`}
        role="switch"
        aria-checked={value}
        aria-label={label}
        onClick={() => onChange(!value)}
      >
        <span />
      </button>
    </div>
  );
}

interface GenSettings {
  temperature: number;
  topP: number;
  maxTokens: number;
  systemPrompt: string;
}

const SETTINGS_KEY = "lumen.settings.v1";

function loadSettings(): GenSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<GenSettings>;
      return {
        temperature: Number(parsed.temperature ?? 0.7),
        topP: Number(parsed.topP ?? 0.95),
        maxTokens: Number(parsed.maxTokens ?? 512),
        systemPrompt: String(parsed.systemPrompt ?? ""),
      };
    }
  } catch {
    // ignore
  }
  return { temperature: 0.7, topP: 0.95, maxTokens: 512, systemPrompt: "" };
}

const CHAT_STORE = "lumen.chats.v1";

interface StoredChats {
  chats: Chat[];
  conversations: Record<string, Message[]>;
  chatId: string;
}

function loadChats(): StoredChats {
  try {
    const raw = localStorage.getItem(CHAT_STORE);
    if (raw) {
      const parsed = JSON.parse(raw) as StoredChats;
      if (Array.isArray(parsed.chats) && parsed.conversations && typeof parsed.chatId === "string") {
        return parsed;
      }
    }
  } catch {
    // Corrupt local state should never block the app.
  }
  const now = Date.now();
  return {
    chats: chats.map((chat, index) => ({
      ...chat,
      updatedAt: chat.updatedAt ?? now - (index + 1) * 86_400_000,
    })),
    conversations: { moe: initialMessages },
    chatId: "moe",
  };
}

export function App() {
  const params = new URLSearchParams(window.location.search);
  const frame = params.get("frame");
  const forcedPhone = frame === "phone";
  const initial = useMemo(loadChats, []);
  const [section, setSection] = useState<Section>("chats");
  const [theme, setTheme] = useState<ThemePref>(readThemePref);
  const [settings, setSettings] = useState<GenSettings>(loadSettings);
  const [chatId, setChatId] = useState(initial.chatId);
  const [chatList, setChatList] = useState<Chat[]>(initial.chats);
  const [open, setOpen] = useState(false);
  const [activeModel, setActiveModel] = useState("qwen");
  const [conversations, setConversations] =
    useState<Record<string, Message[]>>(initial.conversations);
  const messages = conversations[chatId] ?? [];
  const activeChat = useRef(chatId);
  const [draft, setDraft] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [composerFocused, setComposerFocused] = useState(false);
  const [modelList, setModelList] = useState<Model[]>(models);
  const [modelStatus, setModelStatus] = useState<Record<string, "loading" | "ready" | "error">>({});
  const [ramGb, setRamGb] = useState(device.freeGb);
  const [isTablet, setIsTablet] = useState(
    forcedPhone ? false : typeof window !== "undefined" ? window.innerWidth >= 980 : false,
  );

  useEffect(() => {
    activeChat.current = chatId;
  }, [chatId]);

  useEffect(() => {
    try {
      localStorage.setItem(
        CHAT_STORE,
        JSON.stringify({ chats: chatList, conversations, chatId }),
      );
    } catch {
      // Private mode / full storage: keep the session in memory.
    }
  }, [chatList, conversations, chatId]);

  useEffect(() => {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      // ignore
    }
  }, [settings]);

  function updateMessages(updater: (prev: Message[]) => Message[]) {
    const id = activeChat.current;
    setConversations((prev) => ({ ...prev, [id]: updater(prev[id] ?? []) }));
  }

  // Native streaming bookkeeping: the active generation, the assistant message
  // it writes into, and a token that invalidates pending start retries.
  const generation = useRef(0);
  const assistant = useRef<number | null>(null);
  const startAttempt = useRef(0);
  const stopFallback = useRef<number | null>(null);

  useEffect(() => {
    if (forcedPhone) return;
    const onResize = () => setIsTablet(window.innerWidth >= 980);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [forcedPhone]);

  useEffect(() => () => {
    if (stopFallback.current) window.clearTimeout(stopFallback.current);
  }, []);

  // Events arrive from the Rust worker through `Sllm.poll()` on a timer. Guard
  // by generationId so a late event from a stopped/replaced stream is ignored.
  useEffect(() => {
    window.sllmEvent = (event) => {
      if (!event || typeof event !== "object") return;
      const gid = event.generationId ?? 0;
      if (gid && generation.current && gid !== generation.current) return;

      if (event.event === "model") {
        setModelStatus((prev) => ({ ...prev, [event.modelId]: event.state }));
        return;
      }

      const aid = assistant.current;
      if (aid == null) return;

      if (event.event === "token") {
        if (!event.text) return;
        updateMessages((prev) =>
          prev.map((m) => (m.id === aid ? { ...m, text: m.text + event.text } : m)),
        );
        return;
      }

      if (event.event === "started") {
        return;
      }

      if (event.event === "done") {
        if (stopFallback.current) {
          window.clearTimeout(stopFallback.current);
          stopFallback.current = null;
        }
        generation.current = 0;
        assistant.current = null;
        setStreaming(false);
        updateMessages((prev) =>
          prev.map((m) =>
            m.id === aid ? { ...m, streaming: false, meta: formatStats(event.stats) } : m,
          ),
        );
        return;
      }

      if (event.event === "error") {
        if (stopFallback.current) {
          window.clearTimeout(stopFallback.current);
          stopFallback.current = null;
        }
        generation.current = 0;
        assistant.current = null;
        setStreaming(false);
        updateMessages((prev) =>
          prev.map((m) =>
            m.id === aid
              ? {
                  ...m,
                  streaming: false,
                  error: `${event.code}: ${event.message}${event.retryable ? " (retryable)" : ""}`,
                }
              : m,
          ),
        );
      }
    };
    return () => {
      delete window.sllmEvent;
    };
  }, []);

  // Drain native generation events. Java never calls back into the WebView, so
  // a late event cannot race a destroyed Activity.
  useEffect(() => {
    const pump = window.setInterval(() => {
      const event = pollNativeEvent();
      if (event) window.sllmEvent?.(event);
    }, 40);
    return () => window.clearInterval(pump);
  }, []);

  useEffect(() => {
    requestNativeModels((native) => {
      setModelList(native.models);
      setRamGb(native.ramGb);
      setActiveModel((current) => {
        if (native.models.some((m) => m.id === current)) return current;
        const ready = native.models
          .filter((m) => m.status === "ready")
          .sort((a, b) => a.estimateGb - b.estimateGb);
        return ready[0]?.id ?? native.models[0]?.id ?? current;
      });
    });
  }, []);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useEffect(() => {
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      if (theme === "system") applyTheme("system");
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  useEffect(() => {
    window.sllmSystemTheme = () => {
      if (readThemePref() === "system") applyTheme("system");
    };
    return () => {
      delete window.sllmSystemTheme;
    };
  }, []);

  const chat = chatList.find((c) => c.id === chatId) ?? chatList[0];

  function selectModel(id: string) {
    setActiveModel(id);
    if (!hasNativeStreaming()) return;
    const selected = modelList.find((m) => m.id === id);
    const result = prepareNativeModel(id, selected?.path);
    if (!result.ok) {
      setModelStatus((prev) => ({ ...prev, [id]: "error" }));
      return;
    }
    setModelStatus((prev) => {
      const next: Record<string, "loading" | "ready" | "error"> = {};
      for (const [key, value] of Object.entries(prev)) {
        if (key !== id && value === "loading") next[key] = value;
      }
      next[id] = result.state === "ready" ? "ready" : "loading";
      return next;
    });
  }

  function send() {
    const text = draft.trim();
    if (!text || streaming) return;
    const uid = Date.now();
    const aid = uid + 1;
    setDraft("");
    updateMessages((prev) => [...prev, { id: uid, role: "user", text }]);
    setChatList((prev) =>
      prev.map((c) =>
        c.id === chatId
          ? {
              ...c,
              title: c.title === "New chat" ? text.slice(0, 48) : c.title,
              snippet: text.slice(0, 96),
              time: "Now",
              updatedAt: Date.now(),
            }
          : c,
      ),
    );
    setStreaming(true);
    updateMessages((prev) => [...prev, { id: aid, role: "assistant", text: "", streaming: true }]);

    if (hasNativeStreaming()) {
      assistant.current = aid;
      generation.current = 0;
      const attempt = ++startAttempt.current;
      const selected = modelList.find((m) => m.id === activeModel) ?? modelList[0];

      // The engine sees the whole conversation and applies the model's chat
      // template. `modelPath` is absolute and comes from the native registry.
      const conversation: { role: "user" | "assistant" | "system"; content: string }[] = [
        ...messages
          .filter((m) => m.role === "user" || m.role === "assistant")
          .map((m) => ({ role: m.role as "user" | "assistant", content: m.text }))
          .filter((m) => m.content.trim().length > 0),
        { role: "user", content: text },
      ];
      if (settings.systemPrompt.trim()) {
        conversation.unshift({ role: "system", content: settings.systemPrompt.trim() });
      }
      const request = {
        messages: conversation,
        maxTokens: settings.maxTokens,
        temperature: settings.temperature,
        topP: settings.topP,
        seed: -1,
        modelId: selected?.id ?? activeModel,
        modelPath: selected?.path,
      };

      const start = (retriesLeft: number) => {
        if (attempt !== startAttempt.current) return;
        const result = startNativeStream(request);
        if (result.ok) {
          generation.current = result.generationId ?? 0;
          return;
        }
        if (result.error?.code === "E_BUSY" && retriesLeft > 0) {
          // A preload or a stopped generation may still be unwinding; poll a
          // few seconds instead of failing the message immediately.
          window.setTimeout(() => start(retriesLeft - 1), 250);
          return;
        }
        const code = result.error?.code ?? "E_BRIDGE";
        const message = result.error?.message ?? "could not start generation";
        generation.current = 0;
        assistant.current = null;
        setStreaming(false);
        updateMessages((prev) =>
          prev.map((m) =>
            m.id === aid ? { ...m, streaming: false, error: `${code}: ${message}` } : m,
          ),
        );
      };
      start(40);
      return;
    }

    setStreaming(false);
    updateMessages((prev) =>
      prev.map((m) =>
        m.id === aid
          ? {
              ...m,
              streaming: false,
              text:
                "Engine is not connected. This build has no local inference: llama.cpp is not linked yet. The chat is disabled until the model runs on the device.",
              meta: "engine offline",
            }
          : m,
      ),
    );
  }

  function stop() {
    if (!streaming) return;
    const aid = assistant.current;
    startAttempt.current += 1;

    if (hasNativeStreaming()) {
      // Keep the partial text. `done` (stats.canceled = true) normally arrives
      // in a few ms; the fallback only covers a lost terminal event.
      cancelNativeStream();
      if (stopFallback.current) window.clearTimeout(stopFallback.current);
      stopFallback.current = window.setTimeout(() => {
        stopFallback.current = null;
        if (assistant.current !== aid) return;
        assistant.current = null;
        generation.current = 0;
        setStreaming(false);
        updateMessages((prev) =>
          prev.map((m) =>
            m.id === aid ? { ...m, streaming: false, meta: "stopped" } : m,
          ),
        );
      }, 1500);
      return;
    }

    assistant.current = null;
    generation.current = 0;
    setStreaming(false);
    if (aid != null) {
      updateMessages((prev) =>
        prev.map((m) =>
          m.id === aid ? { ...m, streaming: false, meta: "engine offline" } : m,
        ),
      );
    }
  }

  function newChat() {
    startAttempt.current += 1;
    if (streaming) {
      if (hasNativeStreaming()) {
        cancelNativeStream();
        try {
          window.Sllm?.reset?.();
        } catch {
          // bridge may be gone
        }
      }
    }
    generation.current = 0;
    assistant.current = null;
    setStreaming(false);
    const id = `chat-${Date.now()}`;
    const model =
      modelList.find((m) => m.id === activeModel) ?? modelList[0] ?? models[2];
    const fresh: Chat = {
      id,
      title: "New chat",
      snippet: "",
      model: model.name,
      time: "Now",
      updatedAt: Date.now(),
    };
    setChatList((prev) => [fresh, ...prev]);
    setConversations((prev) => ({ ...prev, [id]: [] }));
    activeChat.current = id;
    setChatId(id);
    setDraft("");
    setSection("chats");
    setOpen(true);
  }

  function go(s: Section) {
    setSection(s);
    if (s === "chats" && !isTablet) setOpen(false);
  }

  const shell = (
    <div
      class={`app ${isTablet ? "is-tablet" : "is-phone"} ${
        composerFocused ? "has-composer-focus" : ""
      }`}
    >
      {isTablet && <Rail section={section} onSection={go} />}

      <main class="main">
        {section === "chats" && isTablet && (
          <div class="split">
            <div class="list-pane">
              <ChatsList
                chats={chatList}
                selected={chatId}
                onSelect={(id) => setChatId(id)}
                onNew={newChat}
                compact
              />
            </div>
            <div class="pane-pane">
              <ChatPane
                chat={chat}
                messages={messages}
                draft={draft}
                streaming={streaming}
                engineModel={modelList.find((m) => m.id === activeModel) ?? modelList[0]}
                onDraft={setDraft}
                onSend={send}
                onStop={stop}
                onComposerFocus={setComposerFocused}
                onBack={() => setOpen(false)}
                isTablet
              />
            </div>
          </div>
        )}

        {section === "chats" && !isTablet && !open && (
          <ChatsList
            chats={chatList}
            selected={chatId}
            onSelect={(id) => {
              setChatId(id);
              setOpen(true);
            }}
            onNew={newChat}
          />
        )}

        {section === "chats" && !isTablet && open && (
          <ChatPane
            chat={chat}
            messages={messages}
            draft={draft}
            streaming={streaming}
            engineModel={modelList.find((m) => m.id === activeModel) ?? modelList[0]}
            onDraft={setDraft}
            onSend={send}
            onStop={stop}
            onComposerFocus={setComposerFocused}
            onBack={() => setOpen(false)}
            isTablet={false}
          />
        )}

        {section === "models" && (
          <ModelsScreen
            active={activeModel}
            onActive={selectModel}
            list={modelList}
            ramGb={ramGb}
            status={modelStatus}
          />
        )}

        {section === "settings" && (
          <SettingsScreen theme={theme} onTheme={setTheme} settings={settings} onSettings={setSettings} />
        )}
      </main>

      {!isTablet && <BottomNav section={section} onSection={go} />}
    </div>
  );

  if (frame === "phone") {
    return (
      <div class="preview-stage">
        <div class="device-frame">{shell}</div>
      </div>
    );
  }

  return shell;
}
