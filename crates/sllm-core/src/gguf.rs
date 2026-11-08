use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::config::EngineConfig;
use crate::error::{ErrorCode, SllmError};
use crate::model::Capability;

const MAX_STRING: u64 = 16 * 1024 * 1024;
const MAX_TENSORS: u64 = 2_000_000;
const MAX_KV: u64 = 10_000_000;
const MAX_ARRAY: u64 = 50_000_000;
const MAX_DIMS: u32 = 8;

#[derive(Debug, Clone, Default)]
pub struct GgufMeta {
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub general_type: Option<String>,
    pub block_count: Option<u32>,
    pub context_length: Option<u32>,
    pub expert_count: Option<u32>,
    pub expert_used: Option<u32>,
    pub embedding: Option<u32>,
    pub file_type: Option<u32>,
    pub head_count: Option<u32>,
    pub head_count_kv: Option<u32>,
    pub key_length: Option<u32>,
    pub value_length: Option<u32>,
    pub split_count: Option<u32>,
    pub split_no: Option<u32>,
    pub has_chat_template: bool,
    pub has_clip: bool,
    pub license: Option<String>,
    pub tensor_count: u32,
    pub tensor_types: BTreeMap<u32, usize>,
}

impl GgufMeta {
    pub fn is_adapter(&self) -> bool {
        self.general_type.as_deref() == Some("adapter")
    }

    pub fn is_mmproj(&self) -> bool {
        self.has_clip || self.architecture.as_deref() == Some("clip")
    }

    pub fn is_split(&self) -> bool {
        self.split_count.unwrap_or(0) > 1
    }

    pub fn is_first_shard(&self) -> bool {
        self.split_no.unwrap_or(0) == 0
    }

    pub fn is_moe(&self) -> bool {
        self.expert_count.unwrap_or(0) > 0
    }
}

#[derive(Debug, Clone)]
pub struct PreflightReport {
    pub ok: bool,
    pub code: Option<ErrorCode>,
    pub message: Option<String>,
    pub detail: Option<String>,
    pub warnings: Vec<String>,
    pub arch: Option<String>,
    pub file_type: Option<u32>,
    pub quant: Option<String>,
    pub n_layer: Option<u32>,
    pub n_ctx_train: Option<u32>,
    pub n_embd: Option<u32>,
    pub n_head_kv: Option<u32>,
    pub estimate_mb: u32,
    pub capabilities: Vec<Capability>,
}

impl PreflightReport {
    fn blocked(
        code: ErrorCode,
        message: impl Into<String>,
        detail: impl Into<String>,
        meta: Option<&GgufMeta>,
        estimate_mb: u32,
    ) -> Self {
        Self {
            ok: false,
            code: Some(code),
            message: Some(message.into()),
            detail: Some(detail.into()),
            warnings: Vec::new(),
            arch: meta.and_then(|m| m.architecture.clone()),
            file_type: meta.and_then(|m| m.file_type),
            quant: meta.map(quant_of),
            n_layer: meta.and_then(|m| m.block_count),
            n_ctx_train: meta.and_then(|m| m.context_length),
            n_embd: meta.and_then(|m| m.embedding),
            n_head_kv: meta.and_then(|m| m.head_count_kv),
            estimate_mb,
            capabilities: Vec::new(),
        }
    }
}

pub fn read_meta(path: &Path) -> Result<GgufMeta, SllmError> {
    let file =
        File::open(path).map_err(|e| SllmError::io(format!("open {}", path.display()), e))?;
    let mut r = Reader::new(BufReader::new(file));

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|e| {
        SllmError::new(ErrorCode::GgufHeader, "read magic").with_detail(e.to_string())
    })?;
    if &magic != b"GGUF" {
        return Err(SllmError::new(ErrorCode::GgufHeader, "not a GGUF file"));
    }
    let version = r.u32()?;
    if !(2..=3).contains(&version) {
        return Err(SllmError::new(
            ErrorCode::GgufHeader,
            format!("unsupported GGUF version {version}"),
        ));
    }
    let tensor_count = r.u64()?;
    if tensor_count > MAX_TENSORS {
        return Err(SllmError::new(ErrorCode::GgufHeader, "too many tensors"));
    }
    let kv_count = r.u64()?;
    if kv_count > MAX_KV {
        return Err(SllmError::new(
            ErrorCode::GgufHeader,
            "too many metadata keys",
        ));
    }

    let mut meta = GgufMeta {
        tensor_count: tensor_count as u32,
        ..Default::default()
    };
    let mut values: BTreeMap<String, Value> = BTreeMap::new();

    for _ in 0..kv_count {
        let key = r.string()?;
        let ty = r.u32()?;
        if key.starts_with("clip.") {
            meta.has_clip = true;
        }
        let wanted = matches!(
            key.as_str(),
            "general.architecture"
                | "general.name"
                | "general.type"
                | "general.file_type"
                | "general.license"
                | "general.license.name"
                | "tokenizer.chat_template"
                | "split.count"
                | "split.no"
        ) || key.ends_with(".block_count")
            || key.ends_with(".context_length")
            || key.ends_with(".expert_count")
            || key.ends_with(".expert_used_count")
            || key.ends_with(".embedding_length")
            || key.ends_with(".attention.head_count")
            || key.ends_with(".attention.head_count_kv")
            || key.ends_with(".attention.key_length")
            || key.ends_with(".attention.value_length");

        if wanted {
            values.insert(key, r.value(ty)?);
        } else {
            r.skip_value(ty)?;
        }
    }

    for _ in 0..tensor_count {
        let _name = r.string()?;
        let dims = r.u32()?;
        if dims > MAX_DIMS {
            return Err(SllmError::new(
                ErrorCode::GgufHeader,
                "tensor rank too large",
            ));
        }
        for _ in 0..dims {
            let _ = r.u64()?;
        }
        let ty = r.u32()?;
        let _offset = r.u64()?;
        *meta.tensor_types.entry(ty).or_default() += 1;
    }

    let arch = values
        .get("general.architecture")
        .and_then(Value::as_str)
        .map(str::to_owned);
    meta.architecture = arch.clone();
    meta.name = values
        .get("general.name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    meta.general_type = values
        .get("general.type")
        .and_then(Value::as_str)
        .map(str::to_owned);
    meta.file_type = values
        .get("general.file_type")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    meta.has_chat_template = values
        .get("tokenizer.chat_template")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    meta.license = values
        .get("general.license.name")
        .or_else(|| values.get("general.license"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    meta.split_count = values
        .get("split.count")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    meta.split_no = values
        .get("split.no")
        .and_then(Value::as_u64)
        .map(|v| v as u32);

    meta.block_count = num(&values, arch.as_deref(), "block_count");
    meta.context_length = num(&values, arch.as_deref(), "context_length");
    meta.expert_count = num(&values, arch.as_deref(), "expert_count");
    meta.expert_used = num(&values, arch.as_deref(), "expert_used_count");
    meta.embedding = num(&values, arch.as_deref(), "embedding_length");
    meta.head_count = num(&values, arch.as_deref(), "attention.head_count");
    meta.head_count_kv = num(&values, arch.as_deref(), "attention.head_count_kv");
    meta.key_length = num(&values, arch.as_deref(), "attention.key_length");
    meta.value_length = num(&values, arch.as_deref(), "attention.value_length");

    Ok(meta)
}

pub fn preflight(
    path: &Path,
    bytes: u64,
    available_mb: u64,
    config: &EngineConfig,
) -> PreflightReport {
    let meta = match read_meta(path) {
        Ok(m) => m,
        Err(e) => {
            return PreflightReport::blocked(
                e.code,
                "Не удалось прочитать заголовок GGUF",
                e.detail.unwrap_or(e.message),
                None,
                0,
            );
        }
    };
    let estimate_mb = estimate_mb(bytes, &meta, config);
    if meta.is_adapter() {
        return PreflightReport::blocked(
            ErrorCode::LoraNotModel,
            "Это LoRA-адаптер, а не модель",
            "Добавьте адаптер в пакет модели; отдельно он не загружается.",
            Some(&meta),
            estimate_mb,
        );
    }
    if meta.is_mmproj() {
        return PreflightReport::blocked(
            ErrorCode::MmprojNotModel,
            "Это mmproj / vision tower",
            "Нужен как companion-файл к текстовой модели, а не как чат-модель.",
            Some(&meta),
            estimate_mb,
        );
    }
    if meta.is_split() && !meta.is_first_shard() {
        return PreflightReport::blocked(
            ErrorCode::SplitNotFirst,
            "Выбран не первый шард split-модели",
            "Откройте шард 00001-of-…; остальные загрузятся автоматически.",
            Some(&meta),
            estimate_mb,
        );
    }

    if let Some((ty, _)) = meta
        .tensor_types
        .iter()
        .find(|(ty, _)| !type_supported(**ty))
    {
        let name = type_name(*ty);
        return PreflightReport::blocked(
            ErrorCode::QuantUnsupported,
            "Формат тензоров не поддерживается этой сборкой",
            format!("Тип {name} (id {ty}). Нужен Q2_0/Q4_K/Q8_0/TQ или пересборка движка."),
            Some(&meta),
            estimate_mb,
        );
    }

    let arch = meta
        .architecture
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if !SUPPORTED_ARCH.contains(&arch.as_str()) {
        return PreflightReport::blocked(
            ErrorCode::ArchUnknown,
            "Архитектура не поддерживается этой сборкой",
            format!("Arch: {arch}. Обновите движок или используйте поддерживаемую модель."),
            Some(&meta),
            estimate_mb,
        );
    }

    let mut warnings = Vec::new();
    if !meta.has_chat_template {
        warnings.push("E_TEMPLATE_FALLBACK: нет chat template, будет ChatML".to_string());
    }
    if meta.is_moe() {
        warnings.push("MoE: эксперты читаются с UFS, первый токен медленнее".to_string());
    }
    if let Some(train) = meta.context_length {
        if train < config.ctx {
            warnings.push(format!("E_CTX_CLAMPED: ctx будет уменьшен до {train}"));
        }
    }
    if estimate_mb as u64 > available_mb + 1024 {
        warnings.push(format!(
            "E_RAM_LOW: нужно ~{estimate_mb} MB, доступно ~{available_mb} MB"
        ));
    }

    let mut capabilities = vec![Capability::Text];
    if meta.is_mmproj() {
        capabilities.push(Capability::Vision);
    }

    PreflightReport {
        ok: true,
        code: None,
        message: None,
        detail: None,
        warnings,
        arch: Some(arch),
        file_type: meta.file_type,
        quant: Some(quant_of(&meta)),
        n_layer: meta.block_count,
        n_ctx_train: meta.context_length,
        n_embd: meta.embedding,
        n_head_kv: meta.head_count_kv,
        estimate_mb,
        capabilities,
    }
}

pub fn estimate_mb(bytes: u64, meta: &GgufMeta, config: &EngineConfig) -> u32 {
    let weights = bytes as f64 / 1_048_576.0;
    let layers = meta.block_count.unwrap_or(0) as f64;
    let heads = meta.head_count_kv.or(meta.head_count).unwrap_or(0) as f64;
    let head_dim = meta.key_length.map(|v| v as f64).unwrap_or_else(|| {
        let emb = meta.embedding.unwrap_or(0) as f64;
        let hc = meta.head_count.unwrap_or(0) as f64;
        if hc > 0.0 { emb / hc } else { 0.0 }
    });
    let kv = layers * config.ctx as f64 * heads * head_dim * 2.0 * config.kv.bytes_per_element()
        / 1_048_576.0;
    (weights + kv + 128.0 + 400.0) as u32
}

pub fn quant_of(meta: &GgufMeta) -> String {
    for (ty, name) in [(35u32, "TQ2_0"), (34, "TQ1_0"), (42, "Q2_0"), (39, "MXFP4")] {
        if meta.tensor_types.get(&ty).copied().unwrap_or(0) > 0 {
            return name.to_string();
        }
    }
    match meta.file_type {
        Some(0) => "F32".into(),
        Some(1) => "F16".into(),
        Some(2) => "Q4_0".into(),
        Some(3) => "Q4_1".into(),
        Some(7) => "Q8_0".into(),
        Some(8) => "Q5_0".into(),
        Some(9) => "Q5_1".into(),
        Some(10) => "Q2_K".into(),
        Some(11) => "Q3_K_S".into(),
        Some(12) => "Q3_K_M".into(),
        Some(13) => "Q3_K_L".into(),
        Some(14) => "Q4_K_S".into(),
        Some(15) => "Q4_K_M".into(),
        Some(16) => "Q5_K_S".into(),
        Some(17) => "Q5_K_M".into(),
        Some(18) => "Q6_K".into(),
        Some(19) => "IQ2_XXS".into(),
        Some(20) => "IQ2_XS".into(),
        Some(21) => "Q2_K_S".into(),
        Some(22) => "IQ3_XS".into(),
        Some(23) => "IQ3_XXS".into(),
        Some(24) => "IQ1_S".into(),
        Some(25) => "IQ4_NL".into(),
        Some(26) => "IQ3_S".into(),
        Some(27) => "IQ3_M".into(),
        Some(28) => "IQ2_S".into(),
        Some(29) => "IQ2_M".into(),
        Some(30) => "IQ4_XS".into(),
        Some(31) => "IQ1_M".into(),
        Some(32) => "BF16".into(),
        Some(36) => "TQ1_0".into(),
        Some(37) => "TQ2_0".into(),
        Some(38) => "MXFP4_MOE".into(),
        Some(39) => "NVFP4".into(),
        Some(40) => "Q1_0".into(),
        Some(41) => "Q2_0".into(),
        _ => "GGUF".into(),
    }
}

fn num(values: &BTreeMap<String, Value>, arch: Option<&str>, suffix: &str) -> Option<u32> {
    let key = arch.map(|a| format!("{a}.{suffix}"));
    if let Some(v) = key.as_ref().and_then(|k| values.get(k)) {
        return v.as_u64().map(|n| n as u32);
    }
    values
        .iter()
        .find(|(k, _)| k.ends_with(&format!(".{suffix}")))
        .and_then(|(_, v)| v.as_u64())
        .map(|n| n as u32)
}

fn type_supported(ty: u32) -> bool {
    match ty {
        0..=42 => ty != 4 && ty != 5 && ty != 36,
        _ => false,
    }
}

fn type_name(ty: u32) -> &'static str {
    match ty {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        6 => "Q5_0",
        7 => "Q5_1",
        8 => "Q8_0",
        9 => "Q8_1",
        10 => "Q2_K",
        11 => "Q3_K",
        12 => "Q4_K",
        13 => "Q5_K",
        14 => "Q6_K",
        15 => "Q8_K",
        16 => "IQ2_XXS",
        17 => "IQ2_XS",
        18 => "IQ3_XXS",
        19 => "IQ1_S",
        20 => "IQ4_NL",
        21 => "IQ3_S",
        22 => "IQ2_S",
        23 => "IQ4_XS",
        24 => "I8",
        25 => "I16",
        26 => "I32",
        27 => "I64",
        28 => "F64",
        29 => "IQ1_M",
        30 => "BF16",
        34 => "TQ1_0",
        35 => "TQ2_0",
        39 => "MXFP4",
        40 => "NVFP4",
        41 => "Q1_0",
        42 => "Q2_0",
        43 => "F8_E4M3",
        142 => "PQ2_0",
        143 => "PTQ1_0",
        _ => "unknown",
    }
}

const SUPPORTED_ARCH: &[&str] = &[
    "llama",
    "llama4",
    "deci",
    "falcon",
    "falcon-h1",
    "grok",
    "gpt2",
    "gptj",
    "gptneox",
    "mpt",
    "baichuan",
    "starcoder",
    "starcoder2",
    "refact",
    "bert",
    "modern-bert",
    "nomic-bert",
    "nomic-bert-moe",
    "neo-bert",
    "jina-bert-v2",
    "jina-bert-v3",
    "eurobert",
    "bloom",
    "stablelm",
    "qwen",
    "qwen2",
    "qwen2moe",
    "qwen2vl",
    "qwen3",
    "qwen3moe",
    "qwen3next",
    "qwen3vl",
    "qwen3vlmoe",
    "qwen35",
    "qwen35moe",
    "qwen4exp",
    "gemma",
    "gemma2",
    "gemma3",
    "gemma3n",
    "gemma4",
    "gemma4-assistant",
    "gemma-embedding",
    "gemma-embedding2",
    "phi2",
    "phi3",
    "phimoe",
    "plamo",
    "plamo2",
    "plamo3",
    "codeshell",
    "orion",
    "internlm2",
    "minicpm",
    "minicpm3",
    "mamba",
    "mamba2",
    "jamba",
    "maple",
    "xverse",
    "command-r",
    "cohere2",
    "cohere2moe",
    "dbrx",
    "olmo",
    "olmo2",
    "olmoe",
    "openelm",
    "arctic",
    "deepseek",
    "deepseek2",
    "deepseek32",
    "deepseek4",
    "deepseek41",
    "deepseek2ocr",
    "dots1",
    "dots3note",
    "granite",
    "granite-moe",
    "granite-hybrid",
    "granite-swa",
    "granite-switch",
    "grovemoe",
    "hunyuan-dense",
    "hunyuan-moe",
    "hunyuan-vl",
    "jais",
    "jais2",
    "kimi-k3",
    "kimi-k31",
    "kimi-linear",
    "laguna",
    "lfm2",
    "lfm2moe",
    "lfm3",
    "llada",
    "llada-moe",
    "maincoder",
    "mellum",
    "mimo2",
    "minimax-01",
    "minimax-m2",
    "minimax-m3",
    "mistral3",
    "mistral4",
    "mistral-large4",
    "muse-glimmer",
    "muse-spark",
    "nanbeige",
    "nemotron",
    "nemotron-h",
    "nemotron-h-moe",
    "openai-moe",
    "pangu-embed",
    "pockettts",
    "qwen3tts",
    "rwkv6",
    "rwkv6qwen2",
    "rwkv7",
    "seed-oss",
    "sensenova-u1",
    "smollm3",
    "step35",
    "t5",
    "t5encoder",
    "talkie",
    "wavtokenizer-dec",
    "xing4-0",
    "clover_moe",
    "clef",
    "flint",
    "spark2-5",
    "k2-horizon",
    "kolibri1",
    "metis16",
    "limite",
    "ricursive",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn reads_qwen_header() {
        let path = root().join("models/tiny/Qwen2.5-0.5B-Instruct-Q4_K_M.gguf");
        let meta = read_meta(&path).expect("qwen header");
        assert_eq!(meta.architecture.as_deref(), Some("qwen2"));
        assert_eq!(meta.block_count, Some(24));
        assert!(!meta.is_adapter());
        assert!(!meta.is_mmproj());
    }

    #[test]
    fn reads_bonsai_q2_header() {
        let path = root().join("models/ternary/Ternary-Bonsai-2-4B-Q2_0.gguf");
        let meta = read_meta(&path).expect("bonsai header");
        assert_eq!(meta.architecture.as_deref(), Some("qwen35"));
        assert_eq!(quant_of(&meta), "Q2_0");
        assert!(meta.tensor_types.contains_key(&42));
    }

    #[test]
    fn reads_clover_header() {
        let path = root().join("models/clover/Clover-35B-A3B-Preview.APEX-I-NanoPlus-V2.1.gguf");
        let meta = read_meta(&path).expect("clover header");
        assert_eq!(meta.architecture.as_deref(), Some("clover_moe"));
        assert_eq!(meta.block_count, Some(40));
        assert_eq!(meta.expert_count, Some(256));
        assert!(!meta.tensor_types.is_empty());
    }

    #[test]
    fn rejects_lora_as_model() {
        let path = root().join("models/ternary/Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf");
        let meta = read_meta(&path).expect("lora header");
        assert!(meta.is_adapter());
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Value {
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    Array(Vec<Value>),
    None,
}

impl Value {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(v) => Some(*v),
            Self::I64(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
}

struct Reader<R: Read> {
    inner: R,
}

impl<R: Read> Reader<R> {
    fn new(inner: R) -> Self {
        Self { inner }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        self.inner.read_exact(buf)
    }

    fn u32(&mut self) -> Result<u32, SllmError> {
        let mut b = [0u8; 4];
        self.read_exact(&mut b).map_err(|e| {
            SllmError::new(ErrorCode::GgufTruncated, "read u32").with_detail(e.to_string())
        })?;
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64, SllmError> {
        let mut b = [0u8; 8];
        self.read_exact(&mut b).map_err(|e| {
            SllmError::new(ErrorCode::GgufTruncated, "read u64").with_detail(e.to_string())
        })?;
        Ok(u64::from_le_bytes(b))
    }

    fn string(&mut self) -> Result<String, SllmError> {
        let len = self.u64()?;
        if len > MAX_STRING {
            return Err(SllmError::new(ErrorCode::GgufHeader, "string too large"));
        }
        let mut buf = vec![0u8; len as usize];
        self.read_exact(&mut buf).map_err(|e| {
            SllmError::new(ErrorCode::GgufTruncated, "read string").with_detail(e.to_string())
        })?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn skip(&mut self, count: u64) -> Result<(), SllmError> {
        let mut left = count;
        let mut buf = [0u8; 64 * 1024];
        while left > 0 {
            let n = left.min(buf.len() as u64) as usize;
            self.read_exact(&mut buf[..n]).map_err(|e| {
                SllmError::new(ErrorCode::GgufTruncated, "skip").with_detail(e.to_string())
            })?;
            left -= n as u64;
        }
        Ok(())
    }

    fn value(&mut self, ty: u32) -> Result<Value, SllmError> {
        Ok(match ty {
            0 => Value::U64(self.u8()? as u64),
            1 => Value::I64(self.u8()? as i8 as i64),
            2 => Value::U64(self.u16()? as u64),
            3 => Value::I64(self.u16()? as i16 as i64),
            4 => Value::U64(self.u32()? as u64),
            5 => Value::I64(self.u32()? as i32 as i64),
            6 => Value::F64(f32::from_le_bytes(self.u32()?.to_le_bytes()) as f64),
            7 => Value::Bool(self.u8()? != 0),
            8 => Value::Str(self.string()?),
            9 => {
                let elem = self.u32()?;
                let count = self.u64()?;
                if count > MAX_ARRAY {
                    return Err(SllmError::new(ErrorCode::GgufHeader, "array too large"));
                }
                if count <= 4096 && elem != 9 && elem != 8 {
                    let mut values = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        values.push(self.value(elem)?);
                    }
                    Value::Array(values)
                } else {
                    self.skip_array(elem, count)?;
                    Value::None
                }
            }
            10 => Value::U64(self.u64()?),
            11 => Value::I64(self.u64()? as i64),
            12 => Value::F64(f64::from_bits(self.u64()?)),
            _ => {
                return Err(SllmError::new(
                    ErrorCode::GgufHeader,
                    format!("bad value type {ty}"),
                ));
            }
        })
    }

    fn skip_value(&mut self, ty: u32) -> Result<(), SllmError> {
        match ty {
            0 | 1 | 7 => self.skip(1),
            2 | 3 => self.skip(2),
            4 | 5 | 6 => self.skip(4),
            8 => {
                let len = self.u64()?;
                if len > MAX_STRING {
                    return Err(SllmError::new(ErrorCode::GgufHeader, "string too large"));
                }
                self.skip(len)
            }
            9 => {
                let elem = self.u32()?;
                let count = self.u64()?;
                self.skip_array(elem, count)
            }
            10 | 11 | 12 => self.skip(8),
            _ => Err(SllmError::new(
                ErrorCode::GgufHeader,
                format!("bad value type {ty}"),
            )),
        }
    }

    fn skip_array(&mut self, elem: u32, count: u64) -> Result<(), SllmError> {
        if count > MAX_ARRAY {
            return Err(SllmError::new(ErrorCode::GgufHeader, "array too large"));
        }
        match elem {
            0 | 1 | 7 => self.skip(count),
            2 | 3 => self.skip(count * 2),
            4 | 5 | 6 => self.skip(count * 4),
            10 | 11 | 12 => self.skip(count * 8),
            8 => {
                for _ in 0..count {
                    let len = self.u64()?;
                    if len > MAX_STRING {
                        return Err(SllmError::new(ErrorCode::GgufHeader, "string too large"));
                    }
                    self.skip(len)?;
                }
                Ok(())
            }
            _ => Err(SllmError::new(
                ErrorCode::GgufHeader,
                format!("bad array element {elem}"),
            )),
        }
    }

    fn u8(&mut self) -> Result<u8, SllmError> {
        let mut b = [0u8; 1];
        self.read_exact(&mut b).map_err(|e| {
            SllmError::new(ErrorCode::GgufTruncated, "read u8").with_detail(e.to_string())
        })?;
        Ok(b[0])
    }

    fn u16(&mut self) -> Result<u16, SllmError> {
        let mut b = [0u8; 2];
        self.read_exact(&mut b).map_err(|e| {
            SllmError::new(ErrorCode::GgufTruncated, "read u16").with_detail(e.to_string())
        })?;
        Ok(u16::from_le_bytes(b))
    }
}
