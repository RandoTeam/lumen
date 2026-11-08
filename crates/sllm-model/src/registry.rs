use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use sllm_core::config::EngineConfig;
use sllm_core::error::ErrorCode;
use sllm_core::gguf::{self, GgufMeta, PreflightReport};
use sllm_core::model::Capability;

use crate::error::ModelError;
use crate::probe::{FileFacts, FsProbe, GgufProbe};
use crate::view::{FileInfo, FileRole, Fit, ModelView, RegistryView, Status, capability_label};

const GIB: f64 = 1_073_741_824.0;
const TIGHT_MARGIN_MB: u64 = 1024;

/// One grouped model: a base file (or split shard set) plus its companions.
#[derive(Debug, Clone)]
pub struct Pack {
    pub id: String,
    pub name: String,
    /// Relative path to the first shard, or to the base file.
    pub base_path: String,
    pub size_bytes: u64,
    pub quant: String,
    pub arch: String,
    pub capabilities: Vec<Capability>,
    pub chat_template: bool,
    pub license: Option<String>,
    pub layers: u32,
    pub context: u32,
    pub expert_count: u32,
    pub experts_active: u32,
    pub moe: bool,
    pub ternary: bool,
    pub estimate_mb: u32,
    pub files: Vec<FileInfo>,
    pub shards_total: u32,
    pub shards_present: u32,
    /// Preflight passed and all split shards are present.
    pub supported: bool,
    pub support_code: Option<ErrorCode>,
    /// At least one file was present but could not be parsed as GGUF.
    pub unreadable: bool,
    /// A split model is missing one or more shards, including possibly shard 1.
    pub missing_shards: bool,
}

impl Pack {
    pub fn has_mmproj(&self) -> bool {
        self.files.iter().any(|f| f.role == FileRole::Mmproj)
    }

    pub fn has_lora(&self) -> bool {
        self.files.iter().any(|f| f.role == FileRole::Lora)
    }

    pub fn lora_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.role == FileRole::Lora)
            .count()
    }

    pub fn fit(&self, avail_mb: u64) -> Fit {
        if !self.supported {
            return Fit::Unknown;
        }
        let need = u64::from(self.estimate_mb);
        if need <= avail_mb {
            Fit::Fits
        } else if need <= avail_mb.saturating_add(TIGHT_MARGIN_MB) {
            Fit::Tight
        } else {
            Fit::No
        }
    }

    pub fn status(&self, avail_mb: u64) -> Status {
        if !self.supported {
            Status::Unsupported
        } else if u64::from(self.estimate_mb) > avail_mb {
            Status::StorageBound
        } else {
            Status::Ready
        }
    }

    /// Short English explanation for the model card.
    pub fn reason(&self, avail_mb: u64) -> String {
        if self.missing_shards {
            return format!(
                "Split model is incomplete: {} of {} shards present. Restore the missing files.",
                self.shards_present, self.shards_total
            );
        }
        if self.unreadable {
            return "File is not a readable GGUF (bad magic, truncated or unsupported version)."
                .to_string();
        }
        if let Some(code) = self.support_code {
            return support_reason(code, &self.quant, &self.arch);
        }
        let need_gb = f64::from(self.estimate_mb) / 1024.0;
        if u64::from(self.estimate_mb) > avail_mb {
            let avail_gb = avail_mb as f64 / 1024.0;
            return format!(
                "Needs about {need_gb:.1} GB but only {avail_gb:.1} GB is available right now."
            );
        }
        let avail_gb = avail_mb as f64 / 1024.0;
        format!("Preflight passed; needs about {need_gb:.1} GB, {avail_gb:.1} GB available.")
    }

    pub fn view(&self, avail_mb: u64) -> ModelView {
        ModelView {
            id: self.id.clone(),
            name: self.name.clone(),
            arch: self.arch.clone(),
            quant: self.quant.clone(),
            size_bytes: self.size_bytes,
            size_gb: round2(self.size_bytes as f64 / GIB),
            estimate_mb: self.estimate_mb,
            base_path: self.base_path.clone(),
            files: self.files.clone(),
            capabilities: self
                .capabilities
                .iter()
                .copied()
                .map(capability_label)
                .map(str::to_string)
                .collect(),
            chat_template: self.chat_template,
            license: self.license.clone(),
            layers: self.layers,
            context: self.context,
            moe: self.moe,
            expert_count: self.expert_count,
            experts_active: self.experts_active,
            ternary: self.ternary,
            has_lora: self.has_lora(),
            lora_count: self.lora_count(),
            has_mmproj: self.has_mmproj(),
            shards_total: self.shards_total,
            shards_present: self.shards_present,
            status: self.status(avail_mb),
            fit: self.fit(avail_mb),
            reason: self.reason(avail_mb),
        }
    }
}

/// Discovered model packs for one directory.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    root: PathBuf,
    config: EngineConfig,
    packs: Vec<Pack>,
}

/// Short alias used in plans and docs.
pub type Registry = ModelRegistry;

impl ModelRegistry {
    /// Discover models with the real filesystem probe and default engine
    /// settings (`ctx = 4096`, KV `q8_0`).
    pub fn discover(root: impl AsRef<Path>) -> Result<Self, ModelError> {
        Self::discover_with(root, FsProbe::shared(), &EngineConfig::default())
    }

    /// Testable variant: inject a [`GgufProbe`] and the engine settings used
    /// for memory estimates.
    pub fn discover_with(
        root: impl AsRef<Path>,
        probe: &dyn GgufProbe,
        config: &EngineConfig,
    ) -> Result<Self, ModelError> {
        let root = root.as_ref();
        let meta = fs::metadata(root).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ModelError::RootNotFound {
                    path: root.to_path_buf(),
                }
            } else {
                ModelError::Stat {
                    path: root.to_path_buf(),
                    source,
                }
            }
        })?;
        if !meta.is_dir() {
            return Err(ModelError::RootNotDirectory {
                path: root.to_path_buf(),
            });
        }

        let mut raws = Vec::new();
        let mut seen = HashSet::new();
        walk(root, root, probe, config, &mut raws, &mut seen)?;

        let packs = build_packs(raws, config);
        Ok(Self {
            root: root.to_path_buf(),
            config: config.clone(),
            packs,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }

    pub fn get(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.id == id)
    }

    pub fn view(&self, avail_mb: u64) -> RegistryView {
        RegistryView {
            root: self.root.to_string_lossy().into_owned(),
            avail_mb,
            models: self.packs.iter().map(|p| p.view(avail_mb)).collect(),
        }
    }

    /// Compact JSON for the JNI `nativeModels` bridge.
    pub fn to_json(&self, avail_mb: u64) -> Result<String, ModelError> {
        serde_json::to_string(&self.view(avail_mb))
            .map_err(|e| ModelError::Serialize(e.to_string()))
    }

    pub fn to_json_pretty(&self, avail_mb: u64) -> Result<String, ModelError> {
        serde_json::to_string_pretty(&self.view(avail_mb))
            .map_err(|e| ModelError::Serialize(e.to_string()))
    }
}

struct RawFile {
    rel: String,
    rel_dir: String,
    name: String,
    stem: String,
    bytes: u64,
    facts: Option<FileFacts>,
    error_code: Option<ErrorCode>,
}

struct GroupFile {
    raw: RawFile,
    shard: Option<u32>,
}

struct Group {
    rel_dir: String,
    display_stem: String,
    total: Option<u32>,
    files: Vec<GroupFile>,
}

struct Seed {
    rel_dir: String,
    match_tokens: Vec<String>,
    name: String,
    base_path: String,
    size_bytes: u64,
    quant: String,
    arch: String,
    meta: Option<GgufMeta>,
    report: Option<PreflightReport>,
    chat_template: bool,
    license: Option<String>,
    layers: u32,
    context: u32,
    expert_count: u32,
    experts_active: u32,
    moe: bool,
    ternary: bool,
    estimate_mb: u32,
    files: Vec<FileInfo>,
    has_mmproj: bool,
    shards_total: u32,
    shards_present: u32,
    missing_shards: bool,
    unreadable: bool,
    support_code: Option<ErrorCode>,
    supported: bool,
}

fn walk(
    root: &Path,
    dir: &Path,
    probe: &dyn GgufProbe,
    config: &EngineConfig,
    out: &mut Vec<RawFile>,
    seen: &mut HashSet<(String, u64)>,
) -> Result<(), ModelError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| ModelError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ModelError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let stat = fs::metadata(&path).map_err(|source| ModelError::Stat {
            path: path.clone(),
            source,
        })?;
        if stat.is_dir() {
            walk(root, &path, probe, config, out, seen)?;
            continue;
        }
        if !stat.is_file() {
            continue;
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        // Dedupe identical copies in different folders. A different quant has
        // a different name or different bytes, so it is never hidden.
        if !seen.insert((name.to_ascii_lowercase(), stat.len())) {
            continue;
        }

        let rel = relative_path(root, &path);
        let rel_dir = parent_of(&rel);
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name)
            .to_string();
        let (facts, error_code) = match probe.probe(&path, stat.len(), config) {
            Ok(facts) => (Some(facts), None),
            Err(error) => (None, Some(error.code)),
        };
        out.push(RawFile {
            rel,
            rel_dir,
            name,
            stem,
            bytes: stat.len(),
            facts,
            error_code,
        });
    }
    Ok(())
}

enum Classified {
    Base,
    Shard { key: String, no: u32, total: u32 },
    Lora,
    Mmproj,
}

fn classify(raw: &RawFile) -> Classified {
    let lower = raw.name.to_ascii_lowercase();
    let meta = raw.facts.as_ref().map(|f| &f.meta);

    if meta.is_some_and(GgufMeta::is_adapter) || lower.contains("lora") {
        return Classified::Lora;
    }
    if meta.is_some_and(GgufMeta::is_mmproj)
        || lower.contains("mmproj")
        || has_token(&raw.stem, "clip")
    {
        return Classified::Mmproj;
    }
    if let Some((key, no, total)) = parse_shard_stem(&raw.stem.to_ascii_lowercase()) {
        return Classified::Shard { key, no, total };
    }
    if let Some(meta) = meta
        && meta.is_split()
    {
        let total = meta.split_count.unwrap_or(0);
        if total > 1 {
            let no = meta.split_no.unwrap_or(0) + 1;
            let key = meta
                .name
                .as_deref()
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| lower.clone());
            return Classified::Shard {
                key: format!("meta:{key}"),
                no,
                total,
            };
        }
    }
    Classified::Base
}

fn build_packs(raws: Vec<RawFile>, config: &EngineConfig) -> Vec<Pack> {
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut loras = Vec::new();
    let mut mmprojs = Vec::new();

    for raw in raws {
        match classify(&raw) {
            Classified::Lora => loras.push(raw),
            Classified::Mmproj => mmprojs.push(raw),
            Classified::Base => {
                let key = group_key(&raw.rel_dir, &raw.stem.to_ascii_lowercase(), None);
                groups
                    .entry(key)
                    .or_insert_with(|| Group {
                        rel_dir: raw.rel_dir.clone(),
                        display_stem: raw.stem.clone(),
                        total: None,
                        files: Vec::new(),
                    })
                    .files
                    .push(GroupFile { raw, shard: None });
            }
            Classified::Shard { key, no, total } => {
                let group_key = group_key(&raw.rel_dir, &key, Some(total));
                groups
                    .entry(group_key)
                    .or_insert_with(|| Group {
                        rel_dir: raw.rel_dir.clone(),
                        display_stem: key.clone(),
                        total: Some(total),
                        files: Vec::new(),
                    })
                    .files
                    .push(GroupFile {
                        raw,
                        shard: Some(no),
                    });
            }
        }
    }

    let mut seeds: Vec<Seed> = groups
        .into_values()
        .map(|group| make_seed(group, config))
        .collect();

    for raw in loras {
        if let Some(index) = pick_seed(&raw, &seeds, Companion::Lora) {
            seeds[index].files.push(FileInfo {
                role: FileRole::Lora,
                name: raw.name,
                path: raw.rel,
                bytes: raw.bytes,
                shard: None,
            });
        }
    }
    for raw in mmprojs {
        if let Some(index) = pick_seed(&raw, &seeds, Companion::Mmproj) {
            seeds[index].has_mmproj = true;
            seeds[index].files.push(FileInfo {
                role: FileRole::Mmproj,
                name: raw.name,
                path: raw.rel,
                bytes: raw.bytes,
                shard: None,
            });
        }
    }

    let mut packs: Vec<Pack> = seeds
        .into_iter()
        .map(|seed| {
            let capabilities = derive_capabilities(
                seed.meta.as_ref(),
                seed.report.as_ref(),
                seed.has_mmproj,
                &seed.name,
            );
            let mut files = seed.files;
            files.sort_by(file_order);
            Pack {
                id: String::new(),
                name: seed.name,
                base_path: seed.base_path,
                size_bytes: seed.size_bytes,
                quant: seed.quant,
                arch: seed.arch,
                capabilities,
                chat_template: seed.chat_template,
                license: seed.license,
                layers: seed.layers,
                context: seed.context,
                expert_count: seed.expert_count,
                experts_active: seed.experts_active,
                moe: seed.moe,
                ternary: seed.ternary,
                estimate_mb: seed.estimate_mb,
                files,
                shards_total: seed.shards_total,
                shards_present: seed.shards_present,
                supported: seed.supported,
                support_code: seed.support_code,
                unreadable: seed.unreadable,
                missing_shards: seed.missing_shards,
            }
        })
        .collect();

    assign_ids(&mut packs);
    packs.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.id.cmp(&b.id))
    });
    packs
}

fn make_seed(mut group: Group, config: &EngineConfig) -> Seed {
    let total = group.total;
    group.files.sort_by_key(|f| f.shard.unwrap_or(0));

    let facts = group
        .files
        .iter()
        .find(|f| f.shard == Some(1) && f.raw.facts.is_some())
        .or_else(|| group.files.iter().find(|f| f.raw.facts.is_some()))
        .and_then(|f| f.raw.facts.clone());
    let error_code = group.files.iter().find_map(|f| f.raw.error_code);

    let meta = facts.as_ref().map(|f| f.meta.clone());
    let report = facts.as_ref().map(|f| f.preflight.clone());
    let size_bytes = group.files.iter().map(|f| f.raw.bytes).sum::<u64>();

    let present: BTreeSet<u32> = group.files.iter().filter_map(|f| f.shard).collect();
    let shards_total = total
        .or_else(|| meta.as_ref().and_then(|m| m.split_count))
        .unwrap_or(1)
        .max(1);
    let shards_present = if shards_total > 1 {
        present.len() as u32
    } else {
        u32::from(!group.files.is_empty())
    };
    let missing_shards =
        shards_total > 1 && (shards_present < shards_total || !present.contains(&1));

    let base_path = group
        .files
        .iter()
        .min_by_key(|f| f.shard.unwrap_or(0))
        .map(|f| f.raw.rel.clone())
        .unwrap_or_default();

    let name = pack_name(meta.as_ref(), &group.display_stem);
    let arch = meta
        .as_ref()
        .and_then(|m| m.architecture.clone())
        .unwrap_or_else(|| "GGUF".to_string());
    let quant = meta
        .as_ref()
        .map(quant_label)
        .unwrap_or_else(|| "GGUF".into());
    let ternary = matches!(
        quant.as_str(),
        "Q2_0" | "TQ1_0" | "TQ2_0" | "PQ2_0" | "PTQ1_0"
    );
    let estimate_mb = match meta.as_ref() {
        Some(meta) => gguf::estimate_mb(size_bytes, meta, config),
        None => report.as_ref().map_or(0, |r| r.estimate_mb),
    };
    let unreadable = facts.is_none();
    let supported = !missing_shards && report.as_ref().is_some_and(|r| r.ok);
    let support_code = if unreadable {
        error_code
    } else {
        report.as_ref().and_then(|r| r.code)
    };

    let files = group
        .files
        .iter()
        .map(|f| FileInfo {
            role: if shards_total > 1 {
                FileRole::Shard
            } else {
                FileRole::Base
            },
            name: f.raw.name.clone(),
            path: f.raw.rel.clone(),
            bytes: f.raw.bytes,
            shard: if shards_total > 1 { f.shard } else { None },
        })
        .collect();

    Seed {
        rel_dir: group.rel_dir,
        match_tokens: tokenize(&group.display_stem),
        name,
        base_path,
        size_bytes,
        quant,
        arch,
        chat_template: meta.as_ref().is_some_and(|m| m.has_chat_template),
        license: meta.as_ref().and_then(|m| m.license.clone()),
        layers: meta.as_ref().and_then(|m| m.block_count).unwrap_or(0),
        context: meta.as_ref().and_then(|m| m.context_length).unwrap_or(0),
        expert_count: meta.as_ref().and_then(|m| m.expert_count).unwrap_or(0),
        experts_active: meta.as_ref().and_then(|m| m.expert_used).unwrap_or(0),
        moe: meta.as_ref().is_some_and(GgufMeta::is_moe),
        ternary,
        meta,
        report,
        estimate_mb,
        files,
        has_mmproj: false,
        shards_total,
        shards_present,
        missing_shards,
        unreadable,
        support_code,
        supported,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Companion {
    Lora,
    Mmproj,
}

fn pick_seed(companion: &RawFile, seeds: &[Seed], kind: Companion) -> Option<usize> {
    if seeds.is_empty() {
        return None;
    }
    let tokens = companion_tokens(companion, kind);
    let same_dir: Vec<usize> = seeds
        .iter()
        .enumerate()
        .filter(|(_, seed)| seed.rel_dir == companion.rel_dir)
        .map(|(index, _)| index)
        .collect();

    if same_dir.len() == 1 {
        return same_dir.first().copied();
    }

    if same_dir.len() > 1 {
        let best = best_seed(&same_dir, &tokens, seeds);
        if let Some((index, score)) = best
            && score >= 2
        {
            return Some(index);
        }
    }

    let all: Vec<usize> = (0..seeds.len()).collect();
    let best = best_seed(&all, &tokens, seeds);
    match best {
        Some((index, score)) if score >= 2 => Some(index),
        _ => None,
    }
}

fn best_seed(candidates: &[usize], tokens: &[String], seeds: &[Seed]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, (usize, u8, u64))> = None;
    for &index in candidates {
        let seed = &seeds[index];
        let score = common_prefix_len(tokens, &seed.match_tokens);
        let rank = (score, u8::from(seed.supported), seed.size_bytes);
        if best.as_ref().is_none_or(|(_, best_rank)| rank > *best_rank) {
            best = Some((index, rank));
        }
    }
    best.map(|(index, (score, _, _))| (index, score))
}

fn companion_tokens(raw: &RawFile, kind: Companion) -> Vec<String> {
    let mut tokens = tokenize(&raw.stem);
    match kind {
        Companion::Lora => {
            while tokens.last().is_some_and(|token| token == "lora") {
                tokens.pop();
            }
        }
        Companion::Mmproj => {
            tokens.retain(|token| token != "mmproj" && token != "clip");
        }
    }
    tokens
}

fn common_prefix_len(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn derive_capabilities(
    meta: Option<&GgufMeta>,
    report: Option<&PreflightReport>,
    has_mmproj: bool,
    name: &str,
) -> Vec<Capability> {
    let arch = meta
        .and_then(|m| m.architecture.as_deref())
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let reported = |cap: Capability| report.is_some_and(|r| r.capabilities.contains(&cap));

    let embedding = reported(Capability::Embedding)
        || arch.contains("embedding")
        || arch.contains("bert")
        || name.contains("embedding");
    let tts = reported(Capability::Tts)
        || arch.contains("tts")
        || matches!(arch.as_str(), "talkie" | "wavtokenizer-dec")
        || name.contains("tts");
    let audio = reported(Capability::Audio)
        || tts
        || arch.contains("audio")
        || name.contains("audio")
        || name.contains("whisper")
        || name.contains("voxtral");
    let video = reported(Capability::Video) || arch.contains("video") || name.contains("video");
    let vision = reported(Capability::Vision)
        || has_mmproj
        || arch.contains("vl")
        || arch.contains("vision")
        || arch == "clip"
        || name.contains("-vl");
    let rerank = reported(Capability::Rerank) || arch.contains("rerank") || name.contains("rerank");
    let diffusion =
        reported(Capability::Diffusion) || arch.contains("diffusion") || name.contains("diffusion");
    let text = reported(Capability::Text) || (!embedding && !tts && !audio);

    let mut capabilities = Vec::new();
    for (enabled, capability) in [
        (text, Capability::Text),
        (vision, Capability::Vision),
        (audio, Capability::Audio),
        (video, Capability::Video),
        (embedding, Capability::Embedding),
        (rerank, Capability::Rerank),
        (diffusion, Capability::Diffusion),
        (tts, Capability::Tts),
    ] {
        if enabled {
            capabilities.push(capability);
        }
    }
    capabilities
}

fn pack_name(meta: Option<&GgufMeta>, fallback: &str) -> String {
    if meta.and_then(|m| m.architecture.as_deref()) == Some("clover_moe") {
        return "Clover 35B-A3B".to_string();
    }
    if let Some(name) = meta.and_then(|m| m.name.as_deref()) {
        let name = name.trim();
        if !name.is_empty() && !name.ends_with(" Src") {
            return name.to_string();
        }
    }
    fallback.to_string()
}

fn quant_label(meta: &GgufMeta) -> String {
    // sllm-core's quant_of does not map these planned tensor ids yet.
    for (id, name) in [(143u32, "PTQ1_0"), (142, "PQ2_0"), (43, "F8_E4M3")] {
        if meta.tensor_types.contains_key(&id) {
            return name.to_string();
        }
    }
    gguf::quant_of(meta)
}

fn file_order(a: &FileInfo, b: &FileInfo) -> std::cmp::Ordering {
    fn rank(role: FileRole) -> u8 {
        match role {
            FileRole::Base => 0,
            FileRole::Shard => 1,
            FileRole::Mmproj => 2,
            FileRole::Lora => 3,
        }
    }
    rank(a.role)
        .cmp(&rank(b.role))
        .then_with(|| a.shard.unwrap_or(0).cmp(&b.shard.unwrap_or(0)))
        .then_with(|| a.name.cmp(&b.name))
}

fn assign_ids(packs: &mut [Pack]) {
    let bases: Vec<String> = packs.iter().map(pack_slug).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for base in &bases {
        *counts.entry(base.as_str()).or_default() += 1;
    }

    let ids: Vec<String> = packs
        .iter()
        .zip(&bases)
        .map(|(pack, base)| {
            if counts.get(base.as_str()).copied().unwrap_or(0) > 1 {
                format!(
                    "{base}-{}",
                    short_hash(&format!(
                        "{}|{}|{}",
                        pack.base_path, pack.size_bytes, pack.quant
                    ))
                )
            } else {
                base.clone()
            }
        })
        .collect();

    for (pack, id) in packs.iter_mut().zip(ids) {
        pack.id = id;
    }
}

fn pack_slug(pack: &Pack) -> String {
    let stem = Path::new(&pack.base_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_ascii_lowercase();
    let base = parse_shard_stem(&stem)
        .map(|(base, _, _)| base)
        .unwrap_or(stem);
    slugify(&base)
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "model".to_string()
    } else {
        out.to_string()
    }
}

fn short_hash(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn has_token(value: &str, needle: &str) -> bool {
    tokenize(value).iter().any(|token| token == needle)
}

fn parse_shard_stem(stem: &str) -> Option<(String, u32, u32)> {
    let (left, total_text) = stem.rsplit_once("-of-")?;
    if total_text.len() != 5 || !total_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let total: u32 = total_text.parse().ok()?;
    let (base, no_text) = left.rsplit_once('-')?;
    if no_text.len() != 5 || !no_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let no: u32 = no_text.parse().ok()?;
    if total == 0 || no == 0 || no > total {
        return None;
    }
    Some((base.to_string(), no, total))
}

fn group_key(dir: &str, stem: &str, total: Option<u32>) -> String {
    let total = total.map_or_else(|| "single".to_string(), |value| value.to_string());
    format!("{dir}\u{1f}{stem}\u{1f}{total}")
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn parent_of(relative: &str) -> String {
    relative
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn support_reason(code: ErrorCode, quant: &str, arch: &str) -> String {
    match code {
        ErrorCode::QuantUnsupported => format!(
            "Quant {quant} is not supported by this build. Use Q2_0, Q4_K, Q8_0 or a mainline engine."
        ),
        ErrorCode::ArchUnknown => {
            format!("Architecture {arch} is not supported by this build.")
        }
        ErrorCode::GgufHeader | ErrorCode::GgufTruncated => {
            "File is not a readable GGUF (bad header or truncated).".to_string()
        }
        ErrorCode::SplitNotFirst => {
            "This is not the first split shard; open the 00001-of-NNNNN file.".to_string()
        }
        ErrorCode::LoraNotModel => "This is a LoRA adapter, not a standalone model.".to_string(),
        ErrorCode::MmprojNotModel => {
            "This is an mmproj / vision tower, not a standalone model.".to_string()
        }
        ErrorCode::RamLow => "Not enough memory for this model right now.".to_string(),
        ErrorCode::TemplateFallback => {
            "No chat template in the file; ChatML will be used.".to_string()
        }
        other => format!("Preflight failed with {}.", other.as_str()),
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use sllm_core::config::EngineConfig;
    use sllm_core::error::{ErrorCode, SllmError};
    use sllm_core::gguf::{GgufMeta, PreflightReport};
    use sllm_core::model::Capability;

    use super::*;
    use crate::probe::{FileFacts, GgufProbe};

    #[derive(Default)]
    struct FakeProbe {
        files: BTreeMap<PathBuf, FileFacts>,
    }

    impl FakeProbe {
        fn add(&mut self, path: PathBuf, facts: FileFacts) {
            self.files.insert(path, facts);
        }
    }

    impl GgufProbe for FakeProbe {
        fn probe(
            &self,
            path: &Path,
            _bytes: u64,
            _config: &EngineConfig,
        ) -> Result<FileFacts, SllmError> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| SllmError::new(ErrorCode::GgufHeader, "missing fake header"))
        }
    }

    fn facts(mut meta: GgufMeta) -> FileFacts {
        meta.has_chat_template = true;
        meta.license = Some("apache-2.0".to_string());
        let report = PreflightReport {
            ok: true,
            code: None,
            message: None,
            detail: None,
            warnings: Vec::new(),
            arch: meta.architecture.clone(),
            file_type: meta.file_type,
            quant: Some("Q4_K_M".to_string()),
            n_layer: meta.block_count,
            n_ctx_train: meta.context_length,
            n_embd: meta.embedding,
            n_head_kv: meta.head_count_kv,
            estimate_mb: 529,
            capabilities: vec![Capability::Text],
        };
        FileFacts {
            meta,
            preflight: report,
        }
    }

    fn base_meta(arch: &str, name: &str) -> GgufMeta {
        GgufMeta {
            architecture: Some(arch.to_string()),
            name: Some(name.to_string()),
            block_count: Some(24),
            context_length: Some(32768),
            file_type: Some(15), // Q4_K_M for synthetic headers
            license: Some("apache-2.0".to_string()),
            has_chat_template: true,
            ..GgufMeta::default()
        }
    }

    fn q2_meta(name: &str) -> GgufMeta {
        let mut meta = base_meta("qwen35", name);
        meta.tensor_types.insert(42, 1); // Q2_0
        meta
    }

    fn q8_meta(name: &str) -> GgufMeta {
        let mut meta = base_meta("qwen3", name);
        meta.file_type = Some(7); // Q8_0
        meta
    }

    fn split_facts(arch: &str, name: &str, no: u32, total: u32) -> FileFacts {
        let mut meta = base_meta(arch, name);
        meta.split_no = Some(no - 1);
        meta.split_count = Some(total);
        facts(meta)
    }

    fn adapter_facts(name: &str) -> FileFacts {
        let mut meta = base_meta("qwen35", name);
        meta.general_type = Some("adapter".to_string());
        facts(meta)
    }

    fn mmproj_facts(name: &str) -> FileFacts {
        let mut meta = base_meta("clip", name);
        meta.has_clip = true;
        facts(meta)
    }

    fn quant_facts(name: &str, tensor: u32, code: Option<ErrorCode>) -> FileFacts {
        let mut meta = base_meta("qwen35", name);
        meta.tensor_types.insert(tensor, 12);
        let mut facts = facts(meta);
        if let Some(code) = code {
            facts.preflight.ok = false;
            facts.preflight.code = Some(code);
            facts.preflight.capabilities.clear();
        }
        facts
    }

    fn touch(dir: &Path, relative: &str, bytes: usize) -> PathBuf {
        let path = dir.join(relative);
        let parent = path.parent().expect("parent directory");
        std::fs::create_dir_all(parent).expect("create parent");
        std::fs::write(&path, vec![b'G'; bytes]).expect("write fake gguf");
        path
    }

    #[test]
    fn groups_split_shards_into_one_pack() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let first = touch(root, "qwen3/Qwen3-8B-Q4_K_M-00001-of-00002.gguf", 100);
        let second = touch(root, "qwen3/Qwen3-8B-Q4_K_M-00002-of-00002.gguf", 50);
        let mut probe = FakeProbe::default();
        probe.add(first, split_facts("qwen3", "Qwen3 8B", 1, 2));
        probe.add(second, split_facts("qwen3", "Qwen3 8B", 2, 2));

        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        assert_eq!(registry.packs().len(), 1);
        let pack = &registry.packs()[0];
        assert_eq!(pack.shards_total, 2);
        assert_eq!(pack.shards_present, 2);
        assert!(!pack.missing_shards);
        assert_eq!(pack.size_bytes, 150);
        assert_eq!(pack.files.len(), 2);
        assert!(pack.files.iter().all(|f| f.role == FileRole::Shard));
        assert_eq!(pack.files[0].shard, Some(1));
        assert_eq!(pack.id, "qwen3-8b-q4-k-m");
        assert_eq!(pack.status(4096), Status::Ready);
    }

    #[test]
    fn incomplete_split_is_unsupported() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let second = touch(root, "Qwen-00002-of-00003.gguf", 10);
        let mut probe = FakeProbe::default();
        probe.add(second, split_facts("qwen3", "Qwen", 2, 3));
        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        let pack = &registry.packs()[0];
        assert!(pack.missing_shards);
        assert_eq!(pack.shards_present, 1);
        assert_eq!(pack.status(65536), Status::Unsupported);
        assert!(pack.reason(65536).contains("incomplete"));
    }

    #[test]
    fn lora_is_a_companion_not_a_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let base = touch(root, "ternary/Ternary-Bonsai-2-4B-Q2_0.gguf", 100);
        let lora = touch(root, "ternary/Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf", 5);
        let mut probe = FakeProbe::default();
        probe.add(base, facts(base_meta("qwen35", "Ternary-Bonsai-2-4B")));
        probe.add(lora, adapter_facts("Ternary-Bonsai-2-4B-Uncensored-LoRA"));

        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        assert_eq!(registry.packs().len(), 1);
        let pack = &registry.packs()[0];
        assert!(pack.has_lora());
        assert_eq!(pack.lora_count(), 1);
        assert_eq!(pack.files.len(), 2);
        assert!(pack.files.iter().any(|f| f.role == FileRole::Lora));
        assert!(registry.packs().iter().all(|p| !p.id.contains("lora")));
    }

    #[test]
    fn lora_prefers_supported_quant_among_equal_prefixes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let unsupported = touch(root, "Ternary-Bonsai-2-4B-PTQ1_0.gguf", 10);
        let supported = touch(root, "Ternary-Bonsai-2-4B-Q2_0.gguf", 20);
        let lora = touch(root, "Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf", 5);
        let mut probe = FakeProbe::default();
        probe.add(
            unsupported,
            quant_facts(
                "Ternary-Bonsai-2-4B",
                143,
                Some(ErrorCode::QuantUnsupported),
            ),
        );
        probe.add(supported, facts(q2_meta("Ternary-Bonsai-2-4B")));
        probe.add(lora, adapter_facts("Ternary-Bonsai-2-4B-Uncensored-LoRA"));

        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        let with_lora: Vec<&Pack> = registry.packs().iter().filter(|p| p.has_lora()).collect();
        assert_eq!(with_lora.len(), 1);
        assert_eq!(with_lora[0].quant, "Q2_0");
    }

    #[test]
    fn mmproj_is_a_companion_and_adds_vision() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let base = touch(root, "vl/Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf", 100);
        let mmproj = touch(root, "vl/mmproj-Qwen2.5-VL-3B-f16.gguf", 20);
        let mut probe = FakeProbe::default();
        probe.add(base, facts(base_meta("qwen2vl", "Qwen2.5-VL-3B-Instruct")));
        probe.add(mmproj, mmproj_facts("mmproj-Qwen2.5-VL-3B-f16"));

        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        assert_eq!(registry.packs().len(), 1);
        let pack = &registry.packs()[0];
        assert!(pack.has_mmproj());
        assert!(pack.capabilities.contains(&Capability::Vision));
        assert_eq!(pack.files.len(), 2);
        assert!(registry.packs().iter().all(|p| !p.id.contains("mmproj")));
    }

    #[test]
    fn dedupes_identical_copies_but_keeps_different_quants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let a = touch(root, "a/Model-Q4_K_M.gguf", 64);
        let b = touch(root, "b/Model-Q4_K_M.gguf", 64);
        let q8 = touch(root, "a/Model-Q8_0.gguf", 64);
        let mut probe = FakeProbe::default();
        probe.add(a.clone(), facts(base_meta("qwen3", "Model")));
        probe.add(b, facts(base_meta("qwen3", "Model")));
        probe.add(q8, facts(q8_meta("Model")));

        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        assert_eq!(registry.packs().len(), 2);
        assert!(registry.packs().iter().any(|p| p.quant == "Q4_K_M"));
        assert!(registry.packs().iter().any(|p| p.quant == "Q8_0"));
        assert!(registry.packs().iter().all(|p| p.size_bytes == 64));
    }

    #[test]
    fn same_name_different_bytes_gets_stable_disambiguated_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let first = touch(root, "a/Model.gguf", 64);
        let second = touch(root, "b/Model.gguf", 96);
        let mut probe = FakeProbe::default();
        probe.add(first, facts(base_meta("qwen3", "Model")));
        probe.add(second, facts(base_meta("qwen3", "Model")));
        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        assert_eq!(registry.packs().len(), 2);
        assert!(registry.packs().iter().all(|p| p.id.starts_with("model-")));
        assert_ne!(registry.packs()[0].id, registry.packs()[1].id);
    }

    #[test]
    fn unsupported_quant_is_reported_with_english_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let path = touch(root, "Ternary-Bonsai-2-4B-PTQ1_0.gguf", 32);
        let mut probe = FakeProbe::default();
        probe.add(
            path,
            quant_facts(
                "Ternary-Bonsai-2-4B-PTQ1_0",
                143,
                Some(ErrorCode::QuantUnsupported),
            ),
        );
        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        let pack = &registry.packs()[0];
        assert_eq!(pack.quant, "PTQ1_0");
        assert!(!pack.supported);
        assert_eq!(pack.status(65536), Status::Unsupported);
        assert_eq!(pack.fit(65536), Fit::Unknown);
        let reason = pack.reason(65536);
        assert!(reason.contains("PTQ1_0"));
        assert!(reason.contains("not supported"));
    }

    #[test]
    fn fit_reflects_available_memory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let path = touch(root, "Model.gguf", 1_048_576);
        let mut meta = base_meta("qwen3", "Model");
        // Inflate the KV estimate without copying a large file.
        meta.block_count = Some(40);
        meta.head_count = Some(10);
        meta.head_count_kv = Some(10);
        meta.key_length = Some(512);
        let mut probe = FakeProbe::default();
        probe.add(path, facts(meta));
        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        let pack = &registry.packs()[0];
        let estimate = u64::from(pack.estimate_mb);
        assert!(estimate > 1024, "estimate should be large, got {estimate}");
        assert_eq!(pack.fit(estimate + 512), Fit::Fits);
        assert_eq!(pack.status(estimate + 512), Status::Ready);
        assert_eq!(pack.fit(estimate - 256), Fit::Tight);
        assert_eq!(pack.status(estimate - 256), Status::StorageBound);
        assert_eq!(pack.fit(0), Fit::No);
        assert_eq!(pack.status(0), Status::StorageBound);
    }

    #[test]
    fn json_has_stable_camel_case_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let path = touch(root, "Qwen2.5-0.5B-Instruct-Q4_K_M.gguf", 64);
        let mut probe = FakeProbe::default();
        probe.add(path, facts(base_meta("qwen2", "Qwen2.5 0.5B Instruct")));
        let registry =
            ModelRegistry::discover_with(root, &probe, &EngineConfig::default()).expect("registry");
        let estimate = u64::from(registry.packs()[0].estimate_mb);

        let json = registry.to_json(estimate + 512).expect("json");
        assert!(json.contains("\"availMb\""));
        assert!(json.contains("\"sizeBytes\""));
        assert!(json.contains("\"estimateMb\""));
        assert!(json.contains("\"chatTemplate\""));
        assert!(json.contains("\"basePath\""));
        assert!(json.contains("\"status\":\"ready\""));
        assert!(json.contains("\"capabilities\":[\"Text\"]"));

        let low = registry.to_json(0).expect("json");
        assert!(low.contains("\"status\":\"storage-bound\""));
    }

    #[test]
    fn discovers_real_repository_models() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("models");
        let registry = ModelRegistry::discover(&root).expect("discover real models");
        let ids: Vec<&str> = registry.packs().iter().map(|p| p.id.as_str()).collect();

        let clover = registry
            .get("clover-35b-a3b-preview-apex-i-nanoplus-v2-1")
            .expect("clover pack");
        assert_eq!(clover.arch, "clover_moe");
        assert_eq!(clover.quant, "Q4_0");
        assert_eq!(clover.name, "Clover 35B-A3B");
        assert_eq!(clover.size_bytes, 12_557_370_304);
        assert!(clover.moe);
        assert!(clover.files.len() == 1);
        assert_eq!(clover.status(131_072), Status::Ready);

        let q2 = registry.get("ternary-bonsai-2-4b-q2-0").expect("q2 pack");
        assert_eq!(q2.quant, "Q2_0");
        assert!(q2.ternary);
        assert!(q2.has_lora());
        assert_eq!(q2.lora_count(), 1);

        let ptq = registry
            .get("ternary-bonsai-2-4b-ptq1-0")
            .expect("ptq pack");
        assert_eq!(ptq.quant, "PTQ1_0");
        assert!(!ptq.supported);
        assert_eq!(ptq.status(131_072), Status::Unsupported);
        assert!(ptq.reason(131_072).contains("not supported"));

        assert!(ids.contains(&"qwen2-5-0-5b-instruct-q4-k-m"));
        assert!(ids.iter().all(|id| !id.contains("lora")));
        assert!(ids.iter().all(|id| !id.contains("mmproj")));
    }
}
