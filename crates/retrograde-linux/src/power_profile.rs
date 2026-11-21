use futures_util::StreamExt;
use retrograde_core::PerformanceMode;
use tokio::sync::mpsc;
use tracing::debug;
use zbus::proxy::Builder as ProxyBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerProfile {
    Performance,
    #[default]
    Balanced,
    PowerSaver,
    Unknown,
}

impl PowerProfile {
    pub fn from_ppd_str(raw: &str) -> Self {
        match raw.trim() {
            "performance" => Self::Performance,
            "balanced" => Self::Balanced,
            "power-saver" => Self::PowerSaver,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Balanced => "balanced",
            Self::PowerSaver => "power-saver",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderTier {
    pub effective_profile: PowerProfile,
    pub fps_cap: u32,
    pub enable_shader: bool,
    pub enable_blur: bool,
    pub enable_animations: bool,
    pub poll_interval_ms: u64,
    pub scan_threads: usize,
}

impl RenderTier {
    pub fn resolve(mode: PerformanceMode, system: PowerProfile) -> Self {
        let effective = match mode {
            PerformanceMode::Auto => match system {
                PowerProfile::Unknown => PowerProfile::Balanced,
                other => other,
            },
            PerformanceMode::Performance => PowerProfile::Performance,
            PerformanceMode::Balanced => PowerProfile::Balanced,
            PerformanceMode::PowerSaver => PowerProfile::PowerSaver,
        };

        let host_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        match effective {
            PowerProfile::Performance => Self {
                effective_profile: effective,
                fps_cap: 120,
                enable_shader: true,
                enable_blur: true,
                enable_animations: true,
                poll_interval_ms: 200,
                scan_threads: host_cpus.clamp(2, 8),
            },
            PowerProfile::Balanced | PowerProfile::Unknown => Self {
                effective_profile: effective,
                fps_cap: 60,
                enable_shader: true,
                enable_blur: true,
                enable_animations: true,
                poll_interval_ms: 250,
                scan_threads: (host_cpus / 2).clamp(2, 4),
            },
            PowerProfile::PowerSaver => Self {
                effective_profile: effective,
                fps_cap: 30,
                enable_shader: false,
                enable_blur: false,
                enable_animations: false,
                poll_interval_ms: 1000,
                scan_threads: 2,
            },
        }
    }
}

/// Live monitor for UPower's `power-profiles-daemon` (`org.freedesktop.UPower.PowerProfiles`,
/// falling back to `net.hadess.PowerProfiles`). Never panics or fails if PPD is absent.
pub struct PowerProfileMonitor;

impl PowerProfileMonitor {
    pub async fn start() -> (PowerProfile, mpsc::UnboundedReceiver<PowerProfile>) {
        let (tx, rx) = mpsc::unbounded_channel();

        let Ok(conn) = zbus::Connection::system().await else {
            debug!("System D-Bus unavailable; defaulting PPD profile to Unknown");
            return (PowerProfile::Unknown, rx);
        };

        let targets = [
            (
                "org.freedesktop.UPower.PowerProfiles",
                "/org/freedesktop/UPower/PowerProfiles",
                "org.freedesktop.UPower.PowerProfiles",
            ),
            (
                "net.hadess.PowerProfiles",
                "/net/hadess/PowerProfiles",
                "net.hadess.PowerProfiles",
            ),
        ];

        for (dest, path, iface) in targets {
            let Ok(proxy) = ProxyBuilder::<'_, zbus::Proxy<'static>>::new(&conn)
                .destination(dest)
                .path(path)
                .interface(iface)
                .build()
                .await
            else {
                continue;
            };
            let Ok(raw_profile) = proxy.get_property::<String>("ActiveProfile").await else {
                continue;
            };

            let initial = PowerProfile::from_ppd_str(&raw_profile);
            debug!(profile = initial.label(), service = dest, "Connected to PowerProfilesDaemon");

            tokio::spawn(async move {
                // Keep system D-Bus connection alive inside this monitor task.
                let _conn_keepalive = conn;
                let mut stream = proxy.receive_property_changed::<String>("ActiveProfile").await;
                let mut current = initial;
                while let Some(changed) = stream.next().await {
                    if let Ok(raw) = changed.get().await {
                        let next = PowerProfile::from_ppd_str(&raw);
                        if next != current {
                            current = next;
                            if tx.send(next).is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            return (initial, rx);
        }

        debug!("PowerProfilesDaemon service not found on system bus; falling back cleanly");
        (PowerProfile::Unknown, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_tier_tracks_ppd_and_manual_overrides() {
        let ps = RenderTier::resolve(PerformanceMode::Auto, PowerProfile::PowerSaver);
        assert_eq!(ps.fps_cap, 30);
        assert!(!ps.enable_shader);
        assert!(!ps.enable_blur);
        assert!(!ps.enable_animations);
        assert_eq!(ps.poll_interval_ms, 1000);

        let perf = RenderTier::resolve(PerformanceMode::Auto, PowerProfile::Performance);
        assert_eq!(perf.fps_cap, 120);
        assert!(perf.enable_shader && perf.enable_blur && perf.enable_animations);

        let forced_perf = RenderTier::resolve(PerformanceMode::Performance, PowerProfile::PowerSaver);
        assert_eq!(forced_perf.effective_profile, PowerProfile::Performance);
        assert_eq!(forced_perf.fps_cap, 120);

        let fallback = RenderTier::resolve(PerformanceMode::Auto, PowerProfile::Unknown);
        assert_eq!(fallback.effective_profile, PowerProfile::Balanced);
        assert_eq!(fallback.fps_cap, 60);
    }
}
