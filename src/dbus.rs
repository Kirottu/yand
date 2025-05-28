use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    sync::atomic::{self, AtomicU32},
};

use log::{error, info};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use zbus::{connection::Builder, object_server::SignalEmitter};

#[derive(Debug)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Display for Urgency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Urgency::Low => f.write_str("low"),
            Urgency::Normal => f.write_str("normal"),
            Urgency::Critical => f.write_str("critical"),
        }
    }
}

impl From<u8> for Urgency {
    fn from(val: u8) -> Self {
        match val {
            0 => Urgency::Low,
            1 => Urgency::Normal,
            2 => Urgency::Critical,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Default)]
pub struct DbusNotification {
    pub id: u32,
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: BTreeMap<String, String>,
    pub expire_timeout: i32,

    // Supported hints
    pub action_icons: Option<bool>,
    pub image_data: Option<ImageData>, // TODO
    pub image_path: Option<String>,
    pub resident: Option<bool>,
    pub urgency: Option<Urgency>,
}

#[derive(Debug)]
pub enum NotificationCloseReason {
    Expired,
    DismissedByUser,
    DismissedByApp,
    Undefined,
}

impl From<NotificationCloseReason> for u32 {
    fn from(val: NotificationCloseReason) -> Self {
        match val {
            NotificationCloseReason::Expired => 1,
            NotificationCloseReason::DismissedByUser => 2,
            NotificationCloseReason::DismissedByApp => 3,
            NotificationCloseReason::Undefined => 4,
        }
    }
}

#[derive(Debug)]
pub enum DbusInput {
    NotificationClosed {
        id: u32,
        reason: NotificationCloseReason,
    },
    ActionInvoked {
        id: u32,
        action: String,
    },
}

#[derive(Debug)]
pub enum DbusOutput {
    Notification(DbusNotification),
    CloseNotification(u32),
    Reload,
    Quit,
}

struct Notifications {
    tx: UnboundedSender<DbusOutput>,
    notification_id: AtomicU32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id == 0 {
            let mut id = self.notification_id.fetch_add(1, atomic::Ordering::Relaxed);
            if id == 0 {
                id = self.notification_id.fetch_add(1, atomic::Ordering::Relaxed);
            }
            id
        } else {
            replaces_id
        };

        let mut notification = DbusNotification {
            id,
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            actions: actions
                .chunks_exact(2)
                .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
                .collect(),
            expire_timeout,
            ..Default::default()
        };

        for (key, value) in hints {
            match key.as_str() {
                "action-icons" => notification.action_icons = value.downcast().ok(),
                "image-data" => {
                    if let Ok((
                        width,
                        height,
                        rowstride,
                        has_alpha,
                        _bits_per_sample,
                        _channels,
                        data,
                    )) = value.downcast::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()
                    {
                        notification.image_data = Some(ImageData {
                            width,
                            height,
                            rowstride,
                            has_alpha,
                            data,
                        });
                    }
                }
                "image-path" => notification.image_path = value.downcast().ok(),
                "resident" => notification.resident = value.downcast().ok(),
                "urgency" => notification.urgency = value.downcast::<u8>().ok().map(|u| u.into()),
                _ => (),
            }
        }

        info!(
            "Notification received: {}, Replaces: {}, Summary: {}",
            notification.id, notification.replaces_id, notification.summary
        );

        self.tx
            .send(DbusOutput::Notification(notification))
            .unwrap();

        id
    }

    async fn close_notification(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        id: u32,
    ) -> Result<(), zbus::fdo::Error> {
        self.tx.send(DbusOutput::CloseNotification(id)).unwrap();
        emitter
            .notification_closed(id, NotificationCloseReason::DismissedByApp.into())
            .await?;

        info!("Close requested for {}", id);

        Ok(())
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    async fn get_server_information(&self) -> (&str, &str, &str, &str) {
        ("Yand", "Kirottu", "0.1.0", "1.3")
    }

    async fn get_capabilities(&self) -> &[&str] {
        &["actions", "body", "body-markup"]
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> Result<(), zbus::Error>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action: String,
    ) -> Result<(), zbus::Error>;
}

pub struct Control {
    tx: UnboundedSender<DbusOutput>,
}

#[zbus::interface(
    name = "com.kirottu.Yand",
    proxy(
        default_service = "org.freedesktop.Notifications",
        default_path = "/com/kirottu/Yand"
    )
)]
impl Control {
    async fn reload(&self) {
        self.tx.send(DbusOutput::Reload).unwrap();
    }
}

pub fn start(rx: UnboundedReceiver<DbusInput>, tx: UnboundedSender<DbusOutput>) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            if let Err(why) = dbus_loop(rx, tx.clone()).await {
                error!("Dbus listener reported an error, exiting: {}", why);
                tx.send(DbusOutput::Quit).unwrap();
            }
        });
}

async fn dbus_loop(
    mut rx: UnboundedReceiver<DbusInput>,
    tx: UnboundedSender<DbusOutput>,
) -> Result<(), zbus::Error> {
    {
        let connection = Builder::session()?
            .name("org.freedesktop.Notifications")?
            .serve_at(
                "/org/freedesktop/Notifications",
                Notifications {
                    tx: tx.clone(),
                    notification_id: AtomicU32::new(1),
                },
            )?
            .serve_at("/com/kirottu/Yand", Control { tx })?
            .build()
            .await?;

        let object_server = connection
            .object_server()
            .interface::<&str, Notifications>("/org/freedesktop/Notifications")
            .await?;

        while let Some(msg) = rx.recv().await {
            match msg {
                DbusInput::NotificationClosed { id, reason } => {
                    info!("Notification {} closed: {:?}", id, reason);
                    object_server.notification_closed(id, reason.into()).await?
                }
                DbusInput::ActionInvoked { id, action } => {
                    object_server.action_invoked(id, action).await?
                }
            }
        }

        Ok(())
    }
}
