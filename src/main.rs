use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    rc::Rc,
    thread,
};

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

#[derive(Clone, Deserialize)]
pub struct Config {
    width: i32,
    spacing: i32,
    output: Option<String>,
    timeout: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 400,
            spacing: 10,
            output: None,
            timeout: 10,
        }
    }
}

struct App {
    config_path: PathBuf,
    style_path: PathBuf,
    config: Config,
    display: gdk::Display,
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
            set_layer: Layer::Overlay, // TODO: Configurable
            set_anchor: (Edge::Right, true),
            set_anchor: (Edge::Top, true),
            set_namespace: Some("yand"),
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

        model.reload(&root);

        let mut rx = init.rx;

        sender.command(async move |sender, _shutdown_receiver| {
            while let Some(msg) = rx.recv().await {
                sender.send(msg).unwrap();
            }
        });

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
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
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
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
                self.reload(root);
            }
            DbusOutput::Quit => {
                root.destroy();
            }
        }

        self.update_window(root);
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

    fn reload(&mut self, root: &<App as Component>::Root) {
        self.config = if let Ok(str) = fs::read_to_string(&self.config_path) {
            toml::from_str::<Config>(&str).unwrap_or_else(|why| {
                error!("Failed to parse config file: {}", why);
                Config::default()
            })
        } else {
            Config::default()
        };
        let monitors = self.display.monitors();

        let monitor = if let Some(output) = &self.config.output {
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
        };

        root.set_default_size(self.config.width, 1);
        root.set_monitor(monitor.as_ref());

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
