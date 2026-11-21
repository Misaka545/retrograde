use std::time::Duration;

use tracing::{debug, warn};
use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::proxy::Builder as ProxyBuilder;

const PRIORITY: i32 = 100;

/// Why the card could not be reserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationError {
    /// No session bus (or it failed): nobody can be asked, the card may still be free.
    NoBus(String),
    /// The program holding the card said no (`ApplicationName` when it gives one).
    Refused { holder: Option<String> },
    /// The holder agreed but the name never came over.
    Timeout,
}

impl std::fmt::Display for ReservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBus(e) => write!(f, "no session bus: {e}"),
            Self::Refused { holder } => write!(
                f,
                "held by {} which refused to release it",
                holder.as_deref().unwrap_or("another program")
            ),
            Self::Timeout => f.write_str("timed out taking the card from PipeWire"),
        }
    }
}

struct ReserveDeviceObject {
    device_desc: String,
}

#[zbus::interface(name = "org.freedesktop.ReserveDevice1")]
impl ReserveDeviceObject {
    #[zbus(property)]
    async fn priority(&self) -> i32 {
        PRIORITY
    }

    #[zbus(property)]
    async fn application_name(&self) -> &str {
        "Retrograde"
    }

    #[zbus(property)]
    async fn application_device_name(&self) -> &str {
        &self.device_desc
    }

    async fn request_release(&self, _priority: i32) -> bool {
        // Hold the DAC exclusively while direct ALSA playback is active.
        false
    }
}

/// Holds `org.freedesktop.ReserveDevice1.Audio{card}` over session D-Bus while
/// Retrograde claims an ALSA card exclusively via `alsa/hw:`.
///
/// Dropping or calling [`DeviceReservation::release`] closes the dedicated D-Bus
/// connection immediately so PipeWire/WirePlumber can reclaim the DAC without delay.
pub struct DeviceReservation {
    conn: zbus::Connection,
    bus_name: String,
    card: i32,
}

impl DeviceReservation {
    pub async fn acquire(
        card: i32,
        device_desc: impl Into<String>,
    ) -> Result<Self, ReservationError> {
        let bus_name = format!("org.freedesktop.ReserveDevice1.Audio{card}");
        let object_path = format!("/org/freedesktop/ReserveDevice1/Audio{card}");
        let device_desc = device_desc.into();

        // Dedicated session connection so synchronous Drop closes the bus name immediately.
        let conn = zbus::connection::Builder::session()
            .serve_at(object_path.as_str(), ReserveDeviceObject { device_desc })
            .build()
            .await
            .map_err(|e| ReservationError::NoBus(e.to_string()))?;

        let dbus = DBusProxy::new(&conn)
            .await
            .map_err(|e| ReservationError::NoBus(e.to_string()))?;

        let has_owner = dbus
            .name_has_owner(
                bus_name
                    .as_str()
                    .try_into()
                    .map_err(|e: zbus::Error| ReservationError::NoBus(e.to_string()))?,
            )
            .await
            .unwrap_or(false);

        if has_owner
            && let Ok(holder) = ProxyBuilder::<'_, zbus::Proxy<'_>>::new(&conn)
                .destination(bus_name.as_str())
                .path(object_path.as_str())
                .interface("org.freedesktop.ReserveDevice1")
                .build()
                .await
        {
            match holder.call::<_, _, bool>("RequestRelease", &(PRIORITY,)).await {
                Ok(false) => {
                    let holder = holder
                        .get_property::<String>("ApplicationName")
                        .await
                        .ok()
                        .filter(|n| !n.is_empty());
                    warn!(card, bus_name, ?holder, "Current ReserveDevice1 holder refused RequestRelease(100)");
                    let _ = conn.close().await;
                    return Err(ReservationError::Refused { holder });
                }
                Ok(true) => {
                    debug!(card, "Current ReserveDevice1 holder agreed to release device");
                }
                Err(err) => {
                    debug!(card, error = %err, "RequestRelease call failed (holder may have vanished); attempting claim");
                }
            }
        }

        let flags = RequestNameFlags::REPLACE_EXISTING | RequestNameFlags::DO_NOT_QUEUE;
        let mut acquired = false;
        for _ in 0..20 {
            match conn.request_name_with_flags(bus_name.as_str(), flags).await {
                Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                    acquired = true;
                    break;
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(card, error = %err, "ReserveDevice1 request_name attempt failed");
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if !acquired {
            let _ = conn.close().await;
            return Err(ReservationError::Timeout);
        }

        debug!(card, bus_name, "Acquired ReserveDevice1 lease");
        Ok(Self {
            conn,
            bus_name,
            card,
        })
    }

    pub fn card(&self) -> i32 {
        self.card
    }

    pub async fn release(self) {
        let _ = self.conn.release_name(self.bus_name.as_str()).await;
        let _ = self.conn.close().await;
        debug!(card = self.card, "Released ReserveDevice1 lease");
    }
}
