use std::{rc::Rc, thread};

use dbus::{DbusInput, DbusOutput};
use gtk::{gdk, prelude::*};
use gtk4 as gtk;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use notification::{Notification, NotificationOutput};
use relm4::prelude::*;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

mod dbus;
mod notification;

pub struct Config {
    width: i32,
    spacing: i32,
    output: String,
    timeout: u32,
}

struct App {
    config: Rc<Config>,
    notifications: FactoryVecDeque<Notification>,
    tx: UnboundedSender<dbus::DbusInput>,
}

struct AppInit {
    rx: UnboundedReceiver<DbusOutput>,
    tx: UnboundedSender<dbus::DbusInput>,
}

/// Messages that are transmitted inside the Relm4 App
#[derive(Debug)]
enum AppInput {
    Notification(NotificationOutput),
}

#[derive(Debug)]
enum AppCmdOutput {
    Dbus(DbusOutput),
}

#[relm4::component]
impl Component for App {
    type Init = AppInit;
    type Input = AppInput;
    type Output = ();
    type CommandOutput = AppCmdOutput;

    view! {
        gtk::Window {
            init_layer_shell: (),
            set_layer: Layer::Overlay, // TODO: Configurable
            set_anchor: (Edge::Right, true),
            set_anchor: (Edge::Top, true),
            set_monitor: monitor.as_ref(),
            set_namespace: Some("yand"),
            set_default_size: (model.config.width, 1),

            #[local_ref]
            notification_box -> gtk::Box {
                set_css_classes: &["notification-box"],
                set_orientation: gtk::Orientation::Vertical,
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
            .forward(sender.input_sender(), |output| {
                AppInput::Notification(output)
            });

        let model = Self {
            config: Rc::new(Config {
                spacing: 10,
                width: 400,
                output: "DP-3".to_string(),
                timeout: 0,
            }),
            notifications,
            tx: init.tx,
        };

        relm4::set_global_css_from_file("res/style.css").unwrap();

        let monitors = WidgetExt::display(&root).monitors();

        let monitor = monitors.into_iter().find_map(|item| {
            let monitor = item.unwrap().downcast::<gdk::Monitor>().unwrap();

            if monitor.connector() == Some(model.config.output.clone().into()) {
                Some(monitor)
            } else {
                None
            }
        });

        let notification_box = model.notifications.widget();
        let widgets = view_output!();

        let mut rx = init.rx;

        sender.command(async move |sender, _shutdown_receiver| {
            while let Some(msg) = rx.recv().await {
                sender.send(AppCmdOutput::Dbus(msg)).unwrap();
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
            AppInput::Notification(NotificationOutput::Close { index, reason }) => {
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
            AppInput::Notification(NotificationOutput::ActionInvoked { index, action }) => {
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
            AppCmdOutput::Dbus(dbus_output) => match dbus_output {
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
                    let i = self.notifications.guard().iter().enumerate().find_map(
                        |(i, notification)| if notification.id == id { Some(i) } else { None },
                    );

                    if let Some(i) = i {
                        self.notifications.guard().remove(i);
                    }
                }
                DbusOutput::Quit => {
                    root.destroy();
                }
            },
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
