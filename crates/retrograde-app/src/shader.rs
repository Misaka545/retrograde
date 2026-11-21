use retrograde_core::GpuPreference;
use retrograde_linux::{GraphicsPolicyReport, RenderTier};

pub fn can_use_wgsl_shader(
    tier: &RenderTier,
    graphics: &GraphicsPolicyReport,
    gpu_pref: GpuPreference,
) -> bool {
    tier.enable_shader
        && gpu_pref != GpuPreference::Software
        && !graphics
            .iced_backend
            .as_deref()
            .is_some_and(|b| b.eq_ignore_ascii_case("tiny-skia"))
}

/// Adaptive tick interval in milliseconds, returning `None` (0 FPS) when idle/paused/PowerSaver.
pub fn animation_interval_ms(
    tier: &RenderTier,
    shader_ok: bool,
    accent_animating: bool,
    shader_visible_playing: bool,
    window_focused: bool,
) -> Option<u64> {
    if accent_animating {
        let hz = tier.fps_cap.clamp(30, 60);
        return Some((1000 / hz).max(16) as u64);
    }
    if shader_ok && tier.enable_animations && shader_visible_playing {
        if window_focused {
            let hz = tier.fps_cap.clamp(30, 120);
            return Some((1000 / hz).max(8) as u64);
        } else {
            return Some(66);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrograde_core::{DisplayBackend, PerformanceMode};
    use retrograde_linux::PowerProfile;

    fn report(iced_backend: Option<&str>) -> GraphicsPolicyReport {
        GraphicsPolicyReport {
            gpu_preference: GpuPreference::Igpu,
            display_backend: DisplayBackend::Wayland,
            detected_vendors: vec![],
            vk_driver_files: None,
            wgpu_backend: Some("vulkan".to_owned()),
            wgpu_power_pref: Some("low".to_owned()),
            iced_backend: iced_backend.map(str::to_owned),
            effective_display: "Wayland".to_owned(),
            summary: String::new(),
        }
    }

    #[test]
    fn shader_gate_disables_for_software_and_power_saver() {
        let bal = RenderTier::resolve(PerformanceMode::Balanced, PowerProfile::Balanced);
        let ps = RenderTier::resolve(PerformanceMode::PowerSaver, PowerProfile::Balanced);

        assert!(can_use_wgsl_shader(&bal, &report(None), GpuPreference::Igpu));
        assert!(!can_use_wgsl_shader(&ps, &report(None), GpuPreference::Igpu));
        assert!(!can_use_wgsl_shader(
            &bal,
            &report(None),
            GpuPreference::Software
        ));
        assert!(!can_use_wgsl_shader(
            &bal,
            &report(Some("tiny-skia")),
            GpuPreference::Igpu
        ));
        assert!(!can_use_wgsl_shader(
            &bal,
            &report(Some("Tiny-Skia")),
            GpuPreference::Igpu
        ));
        assert!(can_use_wgsl_shader(
            &bal,
            &report(Some("wgpu")),
            GpuPreference::Igpu
        ));
    }

    #[test]
    fn animation_interval_is_adaptive_and_zero_when_idle() {
        let perf = RenderTier::resolve(PerformanceMode::Performance, PowerProfile::Performance);
        let bal = RenderTier::resolve(PerformanceMode::Balanced, PowerProfile::Balanced);
        let ps = RenderTier::resolve(PerformanceMode::PowerSaver, PowerProfile::PowerSaver);

        assert_eq!(animation_interval_ms(&perf, true, false, true, true), Some(8));
        assert_eq!(animation_interval_ms(&bal, true, false, true, true), Some(16));
        assert_eq!(animation_interval_ms(&perf, true, false, true, false), Some(66));
        assert_eq!(animation_interval_ms(&perf, true, false, false, true), None);
        assert_eq!(animation_interval_ms(&ps, false, false, true, true), None);
        assert_eq!(animation_interval_ms(&ps, false, true, false, true), Some(33));
        assert_eq!(animation_interval_ms(&bal, true, true, false, true), Some(16));
    }
}
