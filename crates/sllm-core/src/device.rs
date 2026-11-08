use std::fs;

/// CPU feature set exposed by `/proc/cpuinfo` on Android/Linux.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuFeatures {
    pub dotprod: bool,
    pub i8mm: bool,
    pub bf16: bool,
    pub fp16: bool,
}

impl CpuFeatures {
    pub fn parse(cpuinfo: &str) -> Self {
        let lower = cpuinfo.to_ascii_lowercase();
        Self {
            dotprod: lower.contains("asimddp"),
            i8mm: lower.contains("i8mm"),
            bf16: lower.contains("bf16"),
            fp16: lower.contains("asimdhp"),
        }
    }
}

/// Everything the core needs to choose safe defaults and a build profile.
#[derive(Debug, Clone)]
pub struct DeviceProfile {
    pub soc: String,
    pub gpu: Option<String>,
    pub android_sdk: Option<u32>,
    pub ram_mb: u64,
    pub cores: usize,
    pub features: CpuFeatures,
}

impl DeviceProfile {
    /// Best-effort Linux/Android detection. The Android app can override the
    /// values with `from_parts` after reading `Build.SOC_MODEL`/GPU info.
    pub fn detect() -> Self {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let features = CpuFeatures::parse(&cpuinfo);
        let ram_mb = meminfo
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix("MemTotal:")?;
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                Some(kb / 1024)
            })
            .unwrap_or(0);
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let soc = detect_soc(&cpuinfo);
        Self {
            soc,
            gpu: std::env::var("SLLM_GPU").ok(),
            android_sdk: std::env::var("SLLM_SDK").ok().and_then(|v| v.parse().ok()),
            ram_mb,
            cores,
            features,
        }
    }

    pub fn from_parts(
        soc: impl Into<String>,
        gpu: Option<String>,
        android_sdk: Option<u32>,
        ram_mb: u64,
        cores: usize,
        features: CpuFeatures,
    ) -> Self {
        Self {
            soc: soc.into(),
            gpu,
            android_sdk,
            ram_mb,
            cores,
            features,
        }
    }

    pub fn is_low_ram(&self) -> bool {
        self.ram_mb > 0 && self.ram_mb < 8 * 1024
    }

    pub fn recommended_threads(&self) -> u32 {
        let soc = self.soc.to_ascii_uppercase();
        if soc.contains("SM8650") || soc.contains("SM8750") {
            // 1x X4/Oryon + 5 performance A720/Oryon cores.
            6
        } else if soc.contains("SDM845") {
            // 4x Cortex-A75.
            4
        } else {
            ((self.cores / 2).clamp(2, 6)) as u32
        }
    }

    pub fn recommended_batch(&self) -> u32 {
        if self.ram_mb >= 12 * 1024 { 256 } else { 128 }
    }

    pub fn recommended_ctx(&self) -> u32 {
        if self.ram_mb >= 12 * 1024 { 8192 } else { 4096 }
    }

    /// 845 is known to be CPU-only; newer Adreno/Hexagon profiles are opt-in
    /// until measured on the exact device.
    pub fn default_backend(&self) -> crate::config::Backend {
        crate::config::Backend::Cpu
    }

    pub fn build_profile(&self) -> BuildProfile {
        let soc = self.soc.to_ascii_uppercase();
        if soc.contains("SDM845") {
            BuildProfile::ArmV8A
        } else if self.features.dotprod && self.features.i8mm && self.features.bf16 {
            BuildProfile::ArmV82DotprodI8mmBf16
        } else if self.features.dotprod && self.features.i8mm {
            BuildProfile::ArmV82DotprodI8mm
        } else if self.features.dotprod {
            BuildProfile::ArmV82Dotprod
        } else {
            BuildProfile::ArmV8A
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    ArmV8A,
    ArmV82Dotprod,
    ArmV82DotprodI8mm,
    ArmV82DotprodI8mmBf16,
}

impl BuildProfile {
    /// Value for llama.cpp's `GGML_CPU_ARM_ARCH`.
    pub const fn llama_arch(self) -> &'static str {
        match self {
            Self::ArmV8A => "armv8-a",
            Self::ArmV82Dotprod => "armv8.2-a+dotprod",
            Self::ArmV82DotprodI8mm => "armv8.2-a+dotprod+i8mm",
            Self::ArmV82DotprodI8mmBf16 => "armv8.2-a+dotprod+i8mm+bf16",
        }
    }
}

fn detect_soc(cpuinfo: &str) -> String {
    if let Ok(v) = std::env::var("SLLM_SOC") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    // CPU part IDs: 0xd80/0xd81/0xd82 are Cortex-X4/A720/A520 (8 Gen 3),
    // 0x803/0x802 are Kryo 385 Gold/Silver (845).
    let lower = cpuinfo.to_ascii_lowercase();
    if lower.contains("0xd80") || lower.contains("0xd81") || lower.contains("0xd82") {
        return "SM8650".to_string();
    }
    if lower.contains("0x803") || lower.contains("0x802") {
        return "SDM845".to_string();
    }
    if let Some(line) = cpuinfo.lines().find(|l| l.starts_with("Hardware")) {
        if let Some(v) = line.split(':').nth(1) {
            return v.trim().to_string();
        }
    }
    "unknown".to_string()
}
