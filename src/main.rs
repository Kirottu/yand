use std::{fs, path::PathBuf, thread};

use dbus::{DbusInput, DbusOutput};
use gtk::{gdk, prelude::*};
use gtk4 as gtk;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use log::error;
use notification::{Notification, NotificationOutput};
use relm4::prelude::*;
use serde::Deserialize;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

mod dbus;
mod notification;

#[derive(Clone, Deserialize, Debug)]
struct AppOverride {
    app_name: String,
    timeout: Option<u32>,
    max_lines: Option<i32>,
}

#[derive(Clone, Deserialize, Debug)]
enum ConfigLayer {
    Background,
    Bottom,
    Top,
    Overlay,
}

impl From<ConfigLayer> for Layer {
    fn from(value: ConfigLayer) -> Self {
        match value {
            ConfigLayer::Background => Self::Background,
            ConfigLayer::Bottom => Self::Bottom,
            ConfigLayer::Top => Self::Top,
            ConfigLayer::Overlay => Self::Overlay,
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
#[serde(default)]
pub struct Config {
    width: i32,
    spacing: i32,
    output: Option<String>,
    timeout: u32,
    layer: ConfigLayer,
    /// Maximum amount of text lines in notification body
    max_lines: i32,
    icon_size: i32,
    // Looks nicer in TOML
    #[serde(rename = "app_override")]
    app_overrides: Vec<AppOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 400,
            spacing: 10,
            output: None,
            timeout: 10,
            layer: ConfigLayer::Overlay,
            max_lines: 5,
            icon_size: 64,
            app_overrides: vec![],
        }
    }
}

impl Config {
    /// Return the same config entry with overridden options
    fn overridden(mut self, app_override: AppOverride) -> (Self, bool) {
        if let Some(val) = app_override.max_lines {
            self.max_lines = val;
        }
        let mut timeout_overridden = false;
        if let Some(val) = app_override.timeout {
            self.timeout = val;
            timeout_overridden = true;
        }
        (self, timeout_overridden)
    }
}

struct App {
    config_path: PathBuf,
    style_path: PathBuf,
    config: Config,
    /// Stored reference to global gdk::Display
    display: gdk::Display,
    /// Used for managing custom CSS
    css_provider: gtk::CssProvider,
    notifications: FactoryVecDeque<Notification>,
    tx: UnboundedSender<dbus::DbusInput>,
}

struct AppInit {
    rx: UnboundedReceiver<DbusOutput>,
    tx: UnboundedSender<dbus::DbusInput>,
}

#[relm4::component]
impl Component for App {
    type Init = AppInit;
    type Input = NotificationOutput;
    type Output = ();
    type CommandOutput = DbusOutput;

    view! {
        gtk::Window {
            init_layer_shell: (),
            #[watch]
            set_layer: model.config.layer.clone().into(),
            set_anchor: (Edge::Right, true),
            set_anchor: (Edge::Top, true),
            set_namespace: Some("yand"),
            #[watch]
            set_monitor: {
                let monitors = model.display.monitors();

                if let Some(output) = &model.config.output {
                    monitors.into_iter().find_map(|item| {
                        let monitor = item.unwrap().downcast::<gdk::Monitor>().unwrap();

                        if monitor.connector() == Some(output.clone().into()) {
                            Some(monitor)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }.as_ref(),
            #[watch]
            set_default_size: (model.config.width, 1),

            #[local_ref]
            notification_box -> gtk::Box {
                set_css_classes: &["notification-box"],
                set_orientation: gtk::Orientation::Vertical,
                #[watch]
                set_spacing: model.config.spacing,
                set_hexpand: true,
            }

        }
    }

    fn init(
        init: AppInit,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let notifications = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(sender.input_sender(), |output| output);

        let dirs = xdg::BaseDirectories::with_prefix("yand");

        let config_path = dirs
            .place_config_file("config.toml")
            .expect("Failed to create config directory");
        let style_path = dirs
            .get_config_file("style.css")
            .expect("Failed to get style path");

        let mut model = Self {
            display: gdk::Display::default().unwrap(),
            css_provider: gtk::CssProvider::default(),
            style_path,
            config_path,
            config: Config::default(),
            notifications,
            tx: init.tx,
        };

        let notification_box = model.notifications.widget();
        let widgets = view_output!();

        model.reload();

        let mut rx = init.rx;

        // For some reason using a gtk::Grid inside the Notification causes a ton of GTK warnings
        // in the log output. The UI works perfectly fine so this is used to suppress the warnings
        // glib::log_set_writer_func(|level, fields| {
        //     if level == glib::LogLevel::Error || level == glib::LogLevel::Critical {
        //         glib::log_writer_default(level, fields);
        //     }
        //     glib::LogWriterOutput::Handled
        // });

        sender.command(async move |sender, _shutdown_receiver| {
            while let Some(msg) = rx.recv().await {
                sender.send(msg).unwrap();
            }
        });

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NotificationOutput::Close { index, reason } => {
                if let Some(notification) = self.notifications.guard().remove(index.current_index())
                {
                    self.tx
                        .send(DbusInput::NotificationClosed {
                            id: notification.id,
                            reason,
                        })
                        .unwrap()
                }
            }
            NotificationOutput::ActionInvoked { index, action } => {
                if let Some(notification) = self.notifications.guard().remove(index.current_index())
                {
                    self.tx
                        .send(DbusInput::ActionInvoked {
                            id: notification.id,
                            action,
                        })
                        .unwrap()
                }
            }
        }

        self.update_window(root);
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            DbusOutput::Notification(dbus_notification) => {
                // It is fine to run the replacement routine here as if replace_id is 0 no notification
                // will match it anyways
                let mut notifications = self.notifications.guard();

                let index = notifications
                    .iter()
                    .enumerate()
                    .find_map(|(i, notification)| {
                        if notification.id == dbus_notification.replaces_id {
                            Some(i)
                        } else {
                            None
                        }
                    });
                if let Some(index) = index {
                    notifications.remove(index);
                    notifications.insert(index, (dbus_notification, self.config.clone()));
                } else {
                    notifications.push_back((dbus_notification, self.config.clone()));
                }
            }
            DbusOutput::CloseNotification(id) => {
                let i =
                    self.notifications
                        .guard()
                        .iter()
                        .enumerate()
                        .find_map(
                            |(i, notification)| if notification.id == id { Some(i) } else { None },
                        );

                if let Some(i) = i {
                    self.notifications.guard().remove(i);
                }
            }
            DbusOutput::Reload => {
                self.reload();
            }
            DbusOutput::Quit => {
                root.destroy();
            }
        }

        self.update_window(root);
        self.update_view(widgets, sender);
    }
}

impl App {
    fn update_window(&self, root: &<App as Component>::Root) {
        if !self.notifications.is_empty() && !root.is_visible() {
            root.set_visible(true);
        } else if self.notifications.is_empty() && root.is_visible() {
            root.set_visible(false);
        }
    }

    fn reload(&mut self) {
        self.config = if let Ok(str) = fs::read_to_string(&self.config_path) {
            toml::from_str::<Config>(&str).unwrap_or_else(|why| {
                error!("Failed to parse config file: {}", why);
                Config::default()
            })
        } else {
            Config::default()
        };

        gtk::style_context_remove_provider_for_display(&self.display, &self.css_provider);

        self.css_provider = gtk::CssProvider::new();

        if let Ok(str) = fs::read_to_string(&self.style_path) {
            self.css_provider.load_from_string(&str);
        } else {
            self.css_provider
                .load_from_string(include_str!("../res/style.css"));
        }
        gtk::style_context_add_provider_for_display(
            &self.display,
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn main() {
    colog::init();

    let app = RelmApp::new("com.kirottu.yand").visible_on_activate(false);

    let (dbus_tx, app_rx) = mpsc::unbounded_channel();
    let (app_tx, dbus_rx) = mpsc::unbounded_channel();

    thread::spawn(|| dbus::start(dbus_rx, dbus_tx));

    app.run::<App>(AppInit {
        rx: app_rx,
        tx: app_tx,
    });
}
