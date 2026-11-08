export type Section = "chats" | "models" | "settings";

export interface Chat {
  id: string;
  title: string;
  snippet: string;
  model: string;
  time: string;
  pinned?: boolean;
  /** Unix ms; used for stable "Today / Yesterday / Older" grouping. */
  updatedAt?: number;
  project?: string;
}

export interface Model {
  id: string;
  name: string;
  arch: string;
  quant: string;
  size: string;
  sizeGb: number;
  layers: number;
  context: string;
  caps: string[];
  estimateGb: number;
  note: string;
  status: "ready" | "storage-bound" | "unsupported";
  /** Absolute on-device path when the list came from the native registry. */
  path?: string;
  hasLora?: boolean;
  loraCount?: number;
  hasMmproj?: boolean;
}

export interface Message {
  id: number;
  role: "user" | "assistant";
  text: string;
  meta?: string;
  streaming?: boolean;
  /** Inline engine/bridge failure, shown under the assistant body. */
  error?: string;
}

export const device = {
  name: "Xiaomi MI 8",
  soc: "Snapdragon 845",
  ram: "6 GB",
  freeGb: 4.8,
  backend: "CPU · 4 threads",
};

export const chats: Chat[] = [
  {
    id: "moe",
    title: "Explain MoE routing",
    snippet: "Only eight experts run per token, so why does it still feel slow?",
    model: "Clover 35B-A3B",
    time: "12:41",
    pinned: true,
  },
  {
    id: "bonsai",
    title: "Bonsai on 6 GB",
    snippet: "Q2_0 with the uncensored LoRA, 4 threads, ctx 4096.",
    model: "Ternary-Bonsai 2-4B",
    time: "Yesterday",
  },
  {
    id: "storage",
    title: "Clover: storage-bound?",
    snippet: "Measure majflt and /proc/self/io before blaming the CPU.",
    model: "Clover 35B-A3B",
    time: "Yesterday",
  },
  {
    id: "qwen",
    title: "Quick drafts with Qwen",
    snippet: "The 0.5B is the daily driver: instant first token, tiny memory.",
    model: "Qwen2.5 0.5B",
    time: "Nov 6",
  },
  {
    id: "ffi",
    title: "Rust FFI notes",
    snippet: "bindgen, a C++ shim, and why exceptions must not cross the boundary.",
    model: "Qwen2.5 0.5B",
    time: "Nov 5",
  },
];

export const models: Model[] = [
  {
    id: "clover",
    name: "Clover 35B-A3B",
    arch: "clover_moe",
    quant: "Q4_0",
    size: "11.69 GB",
    sizeGb: 11.69,
    layers: 40,
    context: "262K train",
    caps: ["Text", "MoE 256e / 8a", "Reasoning"],
    estimateGb: 12.7,
    note: "Active path ≈ 3B. Needs page cache; first token is slow.",
    status: "storage-bound",
  },
  {
    id: "bonsai",
    name: "Ternary-Bonsai 2-4B",
    arch: "qwen35",
    quant: "Q2_0",
    size: "1.13 GB",
    sizeGb: 1.13,
    layers: 32,
    context: "262K train",
    caps: ["Text", "Ternary", "LoRA"],
    estimateGb: 1.9,
    note: "Uncensored LoRA is 2.37 MB and loads at runtime.",
    status: "ready",
  },
  {
    id: "qwen",
    name: "Qwen2.5 0.5B Instruct",
    arch: "qwen2",
    quant: "Q4_K_M",
    size: "0.37 GB",
    sizeGb: 0.37,
    layers: 24,
    context: "32K train",
    caps: ["Text", "Fast", "Drafting"],
    estimateGb: 0.9,
    note: "The baseline. Under a second to first token on 845.",
    status: "ready",
  },
];

export const initialMessages: Message[] = [
  {
    id: 1,
    role: "user",
    text: "Explain how mixture-of-experts models work, in five short bullet points.",
  },
  {
    id: 2,
    role: "assistant",
    text:
      "Lumen is running locally. This is a sample conversation.\n\n" +
      "Open Models to load a GGUF, then ask anything. The model file, the prompt " +
      "and the answer never leave this device.",
    meta: "local · no cloud",
  },
];

export const streamedReply =
  "Exactly. Clover keeps 256 experts on disk but activates eight per token, " +
  "so the math is cheap and the I/O is not. With mmap, a token touches roughly " +
  "350–420 MB of weights. Keep the file on internal storage, use ctx 4096 and " +
  "KV q8_0, and watch majflt before changing anything else.";
