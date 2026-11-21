use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use retrograde_core::{AppSettings, DisplayBackend, GpuPreference};
use tracing::{debug, info};

pub const PCI_VENDOR_NVIDIA: u32 = 0x10de;
pub const PCI_VENDOR_INTEL: u32 = 0x8086;
pub const PCI_VENDOR_AMD: u32 = 0x1002;

const VULKAN_ICD_DIRS: &[&str] = &[
    "/usr/share/vulkan/icd.d",
    "/etc/vulkan/icd.d",
    "/usr/local/share/vulkan/icd.d",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsPolicyReport {
    pub gpu_preference: GpuPreference,
    pub display_backend: DisplayBackend,
    pub detected_vendors: Vec<u32>,
    pub vk_driver_files: Option<String>,
    pub wgpu_backend: Option<String>,
    pub wgpu_power_pref: Option<String>,
    pub iced_backend: Option<String>,
    pub effective_display: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsEnvPlan {
    pub set_vars: Vec<(&'static str, String)>,
    pub unset_vars: Vec<&'static str>,
    pub report: GraphicsPolicyReport,
}

/// Inspects `/sys/class/drm/card*/device/vendor`, scans standard Vulkan ICD directories,
/// and applies non-destructive startup graphics & display backend policy before Iced initializes.
///
/// Any variable already defined in the caller's environment (`VK_DRIVER_FILES`,
/// `VK_ICD_FILENAMES`, `WGPU_BACKEND`, `WGPU_POWER_PREF`, `ICED_BACKEND`) is respected.
///
/// # Safety
/// Must be called from the main thread at process startup **before** spawning any worker threads
/// or starting the Tokio/Iced runtime, because mutating process environment variables is unsynchronized
/// across concurrent threads on Unix.
pub unsafe fn apply_startup_graphics_policy(settings: &AppSettings) -> GraphicsPolicyReport {
    let drm_vendors = detect_drm_vendors(Path::new("/sys/class/drm"));
    let icd_paths = find_vulkan_icd_files(VULKAN_ICD_DIRS);
    let existing_env: HashMap<String, String> = std::env::vars().collect();

    let plan = resolve_graphics_env(settings, &drm_vendors, &icd_paths, &existing_env);
    for (key, value) in &plan.set_vars {
        // SAFETY: caller guarantees single-threaded startup before threads/runtime are spawned.
        unsafe {
            std::env::set_var(key, value);
        }
        debug!(key, value, "Applied startup graphics env policy");
    }
    for key in &plan.unset_vars {
        // SAFETY: caller guarantees single-threaded startup before threads/runtime are spawned.
        unsafe {
            std::env::remove_var(key);
        }
        debug!(key, "Cleared conflicting display env var per startup policy");
    }
    info!(summary = %plan.report.summary, "Graphics policy initialized");
    plan.report
}

/// Pure policy resolver so unit tests can verify hybrid vs single-GPU behavior across distros.
pub fn resolve_graphics_env(
    settings: &AppSettings,
    drm_vendors: &[u32],
    icd_paths: &[PathBuf],
    existing_env: &HashMap<String, String>,
) -> GraphicsEnvPlan {
    let mut set_vars: Vec<(&'static str, String)> = Vec::new();
    let mut unset_vars: Vec<&'static str> = Vec::new();
    let has_nvidia = drm_vendors.contains(&PCI_VENDOR_NVIDIA);
    let has_intel = drm_vendors.contains(&PCI_VENDOR_INTEL);
    let has_amd = drm_vendors.contains(&PCI_VENDOR_AMD);
    let is_hybrid_nvidia = has_nvidia && (has_intel || has_amd);

    let has_user_vk_override = existing_env.contains_key("VK_DRIVER_FILES")
        || existing_env.contains_key("VK_ICD_FILENAMES");

    match settings.gpu_preference {
        GpuPreference::Igpu => {
            if is_hybrid_nvidia && !has_user_vk_override {
                let selected: Vec<String> = icd_paths
                    .iter()
                    .filter(|p| is_matching_igpu_icd(p, has_intel, has_amd))
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if !selected.is_empty() {
                    let joined = selected.join(":");
                    // Set both modern (VK_DRIVER_FILES) and legacy (VK_ICD_FILENAMES) vars for
                    // compatibility with both newer Vulkan loaders and older distro LTS loaders.
                    set_vars.push(("VK_DRIVER_FILES", joined.clone()));
                    set_vars.push(("VK_ICD_FILENAMES", joined));
                }
            }
            if !existing_env.contains_key("WGPU_BACKEND") {
                // Keep wgpu off GL/EGL probing on hybrid laptops so it doesn't load NVIDIA libEGL.
                set_vars.push(("WGPU_BACKEND", "vulkan".to_owned()));
            }
            if !existing_env.contains_key("WGPU_POWER_PREF") {
                set_vars.push(("WGPU_POWER_PREF", "low".to_owned()));
            }
        }
        GpuPreference::Auto => {
            if !existing_env.contains_key("WGPU_POWER_PREF") {
                // iced_wgpu defaults to HighPerformance unless WGPU_POWER_PREF=none/low is set.
                set_vars.push(("WGPU_POWER_PREF", "none".to_owned()));
            }
        }
        GpuPreference::Dgpu => {
            if !existing_env.contains_key("WGPU_POWER_PREF") {
                set_vars.push(("WGPU_POWER_PREF", "high".to_owned()));
            }
        }
        GpuPreference::Software => {
            if !existing_env.contains_key("ICED_BACKEND") {
                set_vars.push(("ICED_BACKEND", "tiny-skia".to_owned()));
            }
        }
    }

    let has_wayland = existing_env.contains_key("WAYLAND_DISPLAY")
        || existing_env.contains_key("WAYLAND_SOCKET");
    let has_x11 = existing_env.contains_key("DISPLAY");

    // winit 0.30 removed WINIT_UNIX_BACKEND and chooses Wayland whenever WAYLAND_DISPLAY /
    // WAYLAND_SOCKET are present, or X11 when only DISPLAY is present.
    let effective_display = match settings.display_backend {
        DisplayBackend::Auto => {
            if has_wayland {
                "Wayland (Auto)".to_owned()
            } else if has_x11 {
                "X11 (Auto)".to_owned()
            } else {
                "Auto".to_owned()
            }
        }
        DisplayBackend::Wayland => {
            if has_wayland {
                if has_x11 {
                    unset_vars.push("DISPLAY");
                }
                "Wayland".to_owned()
            } else if has_x11 {
                "X11 (Wayland unavailable)".to_owned()
            } else {
                "Wayland".to_owned()
            }
        }
        DisplayBackend::X11 => {
            if has_x11 {
                if existing_env.contains_key("WAYLAND_DISPLAY") {
                    unset_vars.push("WAYLAND_DISPLAY");
                }
                if existing_env.contains_key("WAYLAND_SOCKET") {
                    unset_vars.push("WAYLAND_SOCKET");
                }
                "X11".to_owned()
            } else if has_wayland {
                "Wayland (DISPLAY unavailable)".to_owned()
            } else {
                "X11".to_owned()
            }
        }
    };

    let get_effective = |key: &str| -> Option<String> {
        set_vars
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .or_else(|| existing_env.get(key).cloned())
    };

    let vk_driver_files = get_effective("VK_DRIVER_FILES")
        .or_else(|| existing_env.get("VK_ICD_FILENAMES").cloned());
    let wgpu_backend = get_effective("WGPU_BACKEND");
    let wgpu_power_pref = get_effective("WGPU_POWER_PREF");
    let iced_backend = get_effective("ICED_BACKEND");

    let gpu_label = match settings.gpu_preference {
        GpuPreference::Igpu => {
            if iced_backend.as_deref() == Some("tiny-skia") {
                "Software (env override)".to_owned()
            } else if is_hybrid_nvidia && vk_driver_files.is_some() {
                if has_intel {
                    "iGPU locked (Intel, hybrid dGPU dormant)".to_owned()
                } else {
                    "iGPU locked (AMD, hybrid dGPU dormant)".to_owned()
                }
            } else if has_nvidia && !is_hybrid_nvidia {
                "Single NVIDIA GPU (low power hint)".to_owned()
            } else {
                "iGPU / Low-Power".to_owned()
            }
        }
        GpuPreference::Auto => "Auto (wgpu default)".to_owned(),
        GpuPreference::Dgpu => "dGPU (high performance)".to_owned(),
        GpuPreference::Software => "Software CPU (tiny-skia)".to_owned(),
    };

    let summary = format!("{gpu_label} | {effective_display}");
    let report = GraphicsPolicyReport {
        gpu_preference: settings.gpu_preference,
        display_backend: settings.display_backend,
        detected_vendors: drm_vendors.to_vec(),
        vk_driver_files,
        wgpu_backend,
        wgpu_power_pref,
        iced_backend,
        effective_display,
        summary,
    };

    GraphicsEnvPlan {
        set_vars,
        unset_vars,
        report,
    }
}

fn is_matching_igpu_icd(path: &Path, has_intel: bool, has_amd: bool) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".json") {
        return false;
    }
    if lower.contains("nvidia") || lower.contains("lvp") || lower.contains("lavapipe") {
        return false;
    }
    (has_intel && (lower.contains("intel") || lower.contains("hasvk")))
        || (has_amd && (lower.contains("radeon") || lower.contains("amd")))
}

pub fn detect_drm_vendors(drm_root: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };
    let mut vendors = BTreeSet::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }
        let vendor_path = entry.path().join("device/vendor");
        if let Ok(raw) = fs::read_to_string(vendor_path) {
            let trimmed = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
            if let Ok(id) = u32::from_str_radix(trimmed, 16) {
                vendors.insert(id);
            }
        }
    }
    vendors.into_iter().collect()
}

pub fn find_vulkan_icd_files(dirs: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut dir_paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect();
        dir_paths.sort();
        found.extend(dir_paths);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_icds() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/usr/share/vulkan/icd.d/intel_hasvk_icd.x86_64.json"),
            PathBuf::from("/usr/share/vulkan/icd.d/intel_icd.x86_64.json"),
            PathBuf::from("/usr/share/vulkan/icd.d/lvp_icd.x86_64.json"),
            PathBuf::from("/usr/share/vulkan/icd.d/nvidia_icd.json"),
            PathBuf::from("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json"),
        ]
    }

    #[test]
    fn hybrid_intel_nvidia_igpu_locks_intel_icds() {
        let settings = AppSettings {
            gpu_preference: GpuPreference::Igpu,
            display_backend: DisplayBackend::Wayland,
            ..AppSettings::default()
        };
        let mut env = HashMap::new();
        env.insert("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned());
        env.insert("DISPLAY".to_owned(), ":0".to_owned());

        let plan = resolve_graphics_env(
            &settings,
            &[PCI_VENDOR_NVIDIA, PCI_VENDOR_INTEL],
            &sample_icds(),
            &env,
        );
        let expected_icd =
            "/usr/share/vulkan/icd.d/intel_hasvk_icd.x86_64.json:/usr/share/vulkan/icd.d/intel_icd.x86_64.json";
        assert_eq!(plan.report.vk_driver_files.as_deref(), Some(expected_icd));
        assert!(
            plan.set_vars
                .contains(&("VK_ICD_FILENAMES", expected_icd.to_owned()))
        );
        assert_eq!(plan.report.wgpu_backend.as_deref(), Some("vulkan"));
        assert_eq!(plan.report.wgpu_power_pref.as_deref(), Some("low"));
        assert_eq!(plan.unset_vars, vec!["DISPLAY"]);
        assert_eq!(plan.report.effective_display, "Wayland");
        assert_eq!(plan.report.iced_backend, None);
    }

    #[test]
    fn hybrid_amd_nvidia_igpu_locks_radeon_icd() {
        let settings = AppSettings {
            gpu_preference: GpuPreference::Igpu,
            ..AppSettings::default()
        };
        let plan = resolve_graphics_env(
            &settings,
            &[PCI_VENDOR_AMD, PCI_VENDOR_NVIDIA],
            &sample_icds(),
            &HashMap::new(),
        );
        assert_eq!(
            plan.report.vk_driver_files.as_deref(),
            Some("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json")
        );
        assert_eq!(plan.report.wgpu_power_pref.as_deref(), Some("low"));
    }

    #[test]
    fn single_nvidia_gpu_does_not_force_vk_driver_files() {
        let settings = AppSettings {
            gpu_preference: GpuPreference::Igpu,
            ..AppSettings::default()
        };
        let plan = resolve_graphics_env(
            &settings,
            &[PCI_VENDOR_NVIDIA],
            &sample_icds(),
            &HashMap::new(),
        );
        assert_eq!(plan.report.vk_driver_files, None);
        assert_eq!(plan.report.wgpu_power_pref.as_deref(), Some("low"));
    }

    #[test]
    fn preserves_user_existing_environment_variables() {
        let settings = AppSettings {
            gpu_preference: GpuPreference::Igpu,
            display_backend: DisplayBackend::Wayland,
            ..AppSettings::default()
        };
        let mut env = HashMap::new();
        env.insert("VK_ICD_FILENAMES".to_owned(), "/custom/my_icd.json".to_owned());
        env.insert("WGPU_BACKEND".to_owned(), "gl".to_owned());

        let plan = resolve_graphics_env(
            &settings,
            &[PCI_VENDOR_NVIDIA, PCI_VENDOR_INTEL],
            &sample_icds(),
            &env,
        );
        assert!(!plan.set_vars.iter().any(|(k, _)| *k == "VK_DRIVER_FILES"));
        assert!(!plan.set_vars.iter().any(|(k, _)| *k == "VK_ICD_FILENAMES"));
        assert!(!plan.set_vars.iter().any(|(k, _)| *k == "WGPU_BACKEND"));
        assert_eq!(
            plan.report.vk_driver_files.as_deref(),
            Some("/custom/my_icd.json")
        );
        assert_eq!(plan.report.wgpu_backend.as_deref(), Some("gl"));
    }

    #[test]
    fn supports_auto_dgpu_software_and_x11_modes() {
        let auto_plan = resolve_graphics_env(
            &AppSettings {
                gpu_preference: GpuPreference::Auto,
                display_backend: DisplayBackend::Auto,
                ..AppSettings::default()
            },
            &[PCI_VENDOR_NVIDIA, PCI_VENDOR_INTEL],
            &sample_icds(),
            &HashMap::new(),
        );
        assert_eq!(auto_plan.report.wgpu_power_pref.as_deref(), Some("none"));

        let mut env = HashMap::new();
        env.insert("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned());
        env.insert("DISPLAY".to_owned(), ":0".to_owned());
        let dgpu_plan = resolve_graphics_env(
            &AppSettings {
                gpu_preference: GpuPreference::Dgpu,
                display_backend: DisplayBackend::X11,
                ..AppSettings::default()
            },
            &[PCI_VENDOR_NVIDIA, PCI_VENDOR_INTEL],
            &sample_icds(),
            &env,
        );
        assert_eq!(dgpu_plan.report.wgpu_power_pref.as_deref(), Some("high"));
        assert_eq!(dgpu_plan.unset_vars, vec!["WAYLAND_DISPLAY"]);
        assert_eq!(dgpu_plan.report.effective_display, "X11");

        let sw_plan = resolve_graphics_env(
            &AppSettings {
                gpu_preference: GpuPreference::Software,
                ..AppSettings::default()
            },
            &[PCI_VENDOR_INTEL],
            &sample_icds(),
            &HashMap::new(),
        );
        assert_eq!(sw_plan.report.iced_backend.as_deref(), Some("tiny-skia"));
    }
}
