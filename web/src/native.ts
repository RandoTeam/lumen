import type { Model } from "./data";

interface NativeModel {
  id: string;
  name: string;
  arch: string;
  quant: string;
  sizeGb: number;
  layers: number;
  context: number;
  estimateGb?: number;
  estimateMb?: number;
  basePath?: string;
  caps?: string[];
  capabilities?: string[];
  note?: string;
  reason?: string;
  status: "ready" | "storage-bound" | "unsupported";
  moe?: boolean;
  ternary?: boolean;
  hasLora?: boolean;
  loraCount?: number;
  hasMmproj?: boolean;
}

interface NativePayload {
  root?: string;
  availMb: number;
  models: NativeModel[];
  error?: string;
}

export interface GenStats {
  promptTokens: number;
  generatedTokens: number;
  ttftMs: number;
  prefillMs: number;
  decodeMs: number;
  tokPerS: number;
  seed: number;
  canceled: boolean;
  mock: boolean;
}

/** Wire format from `sllm-engine::EngineEvent` plus `generationId` from JNI. */
export type GenEvent =
  | { generationId?: number; event: "started"; promptTokens: number }
  | { generationId?: number; event: "token"; id: number; text: string; count?: number }
  | { generationId?: number; event: "done"; stats: GenStats }
  | {
      generationId?: number;
      event: "error";
      code: string;
      message: string;
      retryable: boolean;
    }
  | {
      generationId?: number;
      event: "model";
      state: "loading" | "ready" | "error";
      modelId: string;
      message?: string;
    };

export interface StartResult {
  ok: boolean;
  generationId?: number;
  modelId?: string;
  state?: "loading" | "ready" | "error";
  mock?: boolean;
  error?: { code: string; message: string; retryable?: boolean };
}

export interface NativeStreamRequest {
  messages?: { role: "user" | "assistant" | "system"; content: string }[];
  prompt?: string;
  maxTokens?: number;
  temperature?: number;
  seed?: number;
  modelId?: string;
  modelPath?: string;
}

export interface HfModel {
  id: string;
  author: string;
  downloads: number;
  likes: number;
  lastModified: string;
  createdAt: string;
  gated: boolean;
  pipelineTag: string;
  tags?: string[];
  gguf: boolean;
}

export interface HfResults {
  ok: boolean;
  query?: string;
  author?: string;
  sort?: string;
  models?: HfModel[];
  error?: string;
}

export interface HfFile {
  name: string;
  kind: "model" | "mmproj" | "lora";
  size: number;
}

export interface HfFilesPayload {
  ok: boolean;
  repo?: string;
  files?: HfFile[];
  error?: string;
}

export interface HfProgress {
  repo: string;
  file: string;
  received: number;
  total: number;
  state: string;
}

export interface HfDone {
  ok: boolean;
  repo?: string;
  file?: string;
  path?: string;
  bytes?: number;
  canceled?: boolean;
  error?: string;
}

declare global {
  interface Window {
    Sllm?: {
      version?: () => string;
      models?: () => string;
      loadModels?: () => void;
      theme?: () => string;
      themePref?: () => string;
      setTheme?: (pref: string) => void;
      engine?: () => string;
      start?: (requestJson: string) => string;
      prepare?: (modelId: string, modelPath: string) => string;
      poll?: () => string | null;
      cancel?: () => void;
      reset?: () => void;
      unload?: () => void;
      stats?: () => string;
      llamaVersion?: () => string;
      hfSearch?: (requestJson: string) => void;
      hfFiles?: (repo: string) => void;
      hfDownload?: (requestJson: string) => void;
      hfCancel?: () => void;
      hfToken?: () => string;
      setHfToken?: (token: string) => void;
    };
    sllmModels?: (payload: NativePayload) => void;
    sllmSystemTheme?: (theme: string) => void;
    sllmEvent?: (event: GenEvent) => void;
    sllmHfResults?: (payload: HfResults) => void;
    sllmHfFiles?: (payload: HfFilesPayload) => void;
    sllmHfProgress?: (payload: HfProgress) => void;
    sllmHfDone?: (payload: HfDone) => void;
  }
}

/** True only when a real local engine is attached. MockEngine is not a model. */
export function hasNativeStreaming(): boolean {
  try {
    return window.Sllm?.engine?.() === "llama";
  } catch {
    return false;
  }
}

/** Drain one native event. Returns null when the queue is empty. */
export function pollNativeEvent(): GenEvent | null {
  try {
    const raw = window.Sllm?.poll?.();
    if (!raw) return null;
    return JSON.parse(raw) as GenEvent;
  } catch {
    return null;
  }
}

export function nativeEngineStats(): unknown {
  try {
    const raw = window.Sllm?.stats?.();
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

export function llamaVersion(): string {
  try {
    return window.Sllm?.llamaVersion?.() ?? "unavailable";
  } catch {
    return "unavailable";
  }
}

/** Start a generation; returns the parsed native ack or an error object. */
export function startNativeStream(request: NativeStreamRequest): StartResult {
  const bridge = window.Sllm;
  if (!bridge?.start) {
    return {
      ok: false,
      error: { code: "E_NO_BRIDGE", message: "native streaming bridge is not available" },
    };
  }
  try {
    return JSON.parse(bridge.start(JSON.stringify(request))) as StartResult;
  } catch (error) {
    return {
      ok: false,
      error: {
        code: "E_BRIDGE",
        message: error instanceof Error ? error.message : String(error),
      },
    };
  }
}

/** Preload one model. Progress arrives as `model` events through `poll`. */
export function prepareNativeModel(modelId: string, modelPath?: string): StartResult {
  const bridge = window.Sllm;
  if (!bridge?.prepare) {
    return {
      ok: false,
      error: { code: "E_NO_BRIDGE", message: "native model loader is not available" },
    };
  }
  try {
    return JSON.parse(bridge.prepare(modelId, modelPath ?? "")) as StartResult;
  } catch (error) {
    return {
      ok: false,
      error: {
        code: "E_BRIDGE",
        message: error instanceof Error ? error.message : String(error),
      },
    };
  }
}

export function cancelNativeStream(): void {
  try {
    window.Sllm?.cancel?.();
  } catch {
    // The Activity may already be going away.
  }
}

export function unloadNativeEngine(): void {
  try {
    window.Sllm?.unload?.();
  } catch {
    // ignore
  }
}

export interface NativeModels {
  models: Model[];
  ramGb: number;
  version: string;
  root: string;
}

function joinPath(root: string, relative?: string): string | undefined {
  if (!relative) return undefined;
  const normalized = relative.replace(/\\/g, "/");
  if (/^[a-zA-Z]:\//.test(normalized) || normalized.startsWith("/")) return normalized;
  return `${root.replace(/[\\/]+$/, "")}/${normalized.replace(/^\/+/, "")}`;
}

function mapPayload(payload: NativePayload): NativeModels | null {
  if (!Array.isArray(payload.models)) return null;
  const root = payload.root ?? "";
  const mapped: Model[] = payload.models.map((m) => {
    const caps = new Set(m.capabilities ?? m.caps ?? ["Text"]);
    if (m.moe) caps.add("MoE");
    if (m.ternary) caps.add("Ternary");
    if (m.hasLora) caps.add("LoRA");
    if (m.hasMmproj) caps.add("Vision");
    return {
      id: m.id,
      name: m.name,
      arch: m.arch,
      quant: m.quant,
      size: `${m.sizeGb.toFixed(2)} GB`,
      sizeGb: m.sizeGb,
      layers: m.layers,
      context: `${m.context} ctx`,
      caps: [...caps].sort(),
      estimateGb:
        m.estimateGb ?? (typeof m.estimateMb === "number" ? m.estimateMb / 1024 : 0),
      note: m.note ?? m.reason ?? "",
      status: m.status,
      path: joinPath(root, m.basePath),
      hasLora: m.hasLora,
      loraCount: m.loraCount,
      hasMmproj: m.hasMmproj,
    };
  });
  return {
    models: mapped,
    ramGb: Math.max(0.5, payload.availMb / 1024),
    version: window.Sllm?.version?.() ?? "rust-ok",
    root,
  };
}

export function requestNativeModels(onReady: (result: NativeModels) => void): void {
  const bridge = window.Sllm;
  if (!bridge) return;

  if (bridge.loadModels) {
    window.sllmModels = (payload) => {
      const mapped = mapPayload(payload);
      if (mapped) onReady(mapped);
    };
    bridge.loadModels();
    return;
  }

  try {
    const raw = bridge.models?.();
    if (!raw) return;
    const mapped = mapPayload(JSON.parse(raw) as NativePayload);
    if (mapped) onReady(mapped);
  } catch {
    // Keep the demo data when the bridge is absent or malformed.
  }
}

export function searchHuggingFace(request: {
  query: string;
  author: string;
  sort: string;
}): boolean {
  try {
    if (!window.Sllm?.hfSearch) return false;
    window.Sllm.hfSearch(JSON.stringify(request));
    return true;
  } catch {
    return false;
  }
}

export function fetchHuggingFaceFiles(repo: string): boolean {
  try {
    if (!window.Sllm?.hfFiles) return false;
    window.Sllm.hfFiles(repo);
    return true;
  } catch {
    return false;
  }
}

export function downloadHuggingFaceFile(repo: string, file: string): boolean {
  try {
    if (!window.Sllm?.hfDownload) return false;
    window.Sllm.hfDownload(JSON.stringify({ repo, file }));
    return true;
  } catch {
    return false;
  }
}

export function cancelHuggingFace(): void {
  try {
    window.Sllm?.hfCancel?.();
  } catch {
    // ignore
  }
}

export function getHuggingFaceToken(): string {
  try {
    return window.Sllm?.hfToken?.() ?? "";
  } catch {
    return "";
  }
}

export function setHuggingFaceToken(token: string): void {
  try {
    window.Sllm?.setHfToken?.(token);
  } catch {
    // ignore
  }
}
