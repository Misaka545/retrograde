pub mod device_reservation;
pub mod gpu_policy;
pub mod mpris;
pub mod portal;
pub mod power_profile;
pub mod single_instance;
pub mod tray;

pub use device_reservation::{DeviceReservation, ReservationError};
pub use gpu_policy::{
    GraphicsEnvPlan, GraphicsPolicyReport, PCI_VENDOR_AMD, PCI_VENDOR_INTEL, PCI_VENDOR_NVIDIA,
    apply_startup_graphics_policy, detect_drm_vendors, find_vulkan_icd_files, resolve_graphics_env,
};
pub use mpris::{MprisCommand, MprisService, MprisState};
pub use portal::{pick_cover_image, pick_music_files, pick_music_folders};
pub use power_profile::{PowerProfile, PowerProfileMonitor, RenderTier};
pub use single_instance::{SingleInstance, SingleInstanceCommand, SingleInstanceGuard, SingleInstanceOutcome};
pub use tray::{TrayCommand, TrayService, TrayToggles};
