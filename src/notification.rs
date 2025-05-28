use std::time::Duration;

use gtk::{gdk, glib, pango, prelude::*};
use gtk4 as gtk;
use log::info;
use relm4::prelude::*;

use crate::{
    Config,
    dbus::{DbusNotification, NotificationCloseReason, Urgency},
};

const DEFAULT_ACTION: &str = "default";

#[derive(Debug)]
struct ActionButton {
    action: String,
    display: String,
}

#[relm4::factory(pub)]
impl FactoryComponent for ActionButton {
    type Init = (String, String);
    type Input = ();
    type Output = String;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;

    view! {
        gtk::Button {
            set_css_classes: &["action"],
            set_label: &self.display,
            set_hexpand: true,
            connect_clicked: glib::clone!(
                #[strong(rename_to = action)] self.action,
                move |_| {
                    sender.output(action.clone()).unwrap();
                }
            )
        }
    }

    fn init_model(init: Self::Init, _index: &Self::Index, _sender: FactorySender<Self>) -> Self {
        Self {
            action: init.0,
            display: init.1,
        }
    }
}

#[derive(Debug)]
enum NotificationIcon {
    Path(String),
    Name(String),
    Data(gdk::Texture),
    None,
}

#[derive(Debug)]
pub enum NotificationOutput {
    Close {
        index: DynamicIndex,
        reason: NotificationCloseReason,
    },
    ActionInvoked {
        index: DynamicIndex,
        action: String,
    },
}

#[derive(Debug)]
pub struct Notification {
    pub id: u32,
    icon: NotificationIcon,
    app_name: String,
    summary: String,
    body: String,
    urgency: Urgency,

    actions_factory: FactoryVecDeque<ActionButton>,
    default_action: Option<String>,
}

#[relm4::widget_template(pub)]
impl WidgetTemplate for IconWidget {
    view! {
        gtk::Image {
            set_pixel_size: 64, // TODO: Attempt to make this configurable somehow
            set_css_classes: &["icon"],
        }
    }
}

#[relm4::factory(pub)]
impl FactoryComponent for Notification {
    type Init = (DbusNotification, Config);
    type Input = ();
    type Output = NotificationOutput;
    type CommandOutput = NotificationOutput;
    type ParentWidget = gtk::Box;

    view! {
        gtk::Box {
            set_css_classes: &["notification", &self.urgency.to_string(), &self.app_name],
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            add_controller = gtk::GestureClick {
                connect_released: glib::clone!(
                    #[strong(rename_to = default)] self.default_action,
                    #[strong] index,
                    move |gesture, _, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        if default.is_some() {
                            sender.output(NotificationOutput::ActionInvoked {
                                index: index.clone(),
                                action: DEFAULT_ACTION.to_string()
                            }).unwrap();
                        } else {
                            sender.output(NotificationOutput::Close {
                                index: index.clone(),
                                reason: NotificationCloseReason::DismissedByUser
                            }).unwrap();
                        }
                    }
                )
            },

            gtk::Label {
                set_label: &self.summary,
                set_css_classes: &["summary"],
                set_justify: gtk::Justification::Left,
                set_halign: gtk::Align::Start,
                set_use_markup: true,
            },


            gtk::Grid {
                set_orientation: gtk4::Orientation::Horizontal,
                set_vexpand: true,
                set_hexpand: true,

                attach[0, 0, 1, 1] = match &self.icon {
                    NotificationIcon::Name(name) => #[template] IconWidget {
                        #[watch]
                        set_icon_name: Some(name),
                    },
                    NotificationIcon::Path(path) => #[template] IconWidget {
                        #[watch]
                        set_from_file: Some(path),
                    },
                    NotificationIcon::Data(data) => #[template] IconWidget {
                        #[watch]
                        set_paintable: Some(data),
                    },
                    NotificationIcon::None => gtk::Box { }
                },

                // TODO: Configurable height limit
                // set_hexpand: true,
                // set_orientation: gtk::Orientation::Vertical,
                attach[1, 0, 1, 1] = &gtk::Label {
                    set_label: &self.body,
                    set_css_classes: &["body"],
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,
                    set_wrap: true,
                    set_use_markup: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::Word,
                    set_wrap_mode: pango::WrapMode::WordChar,
                }
            },

            #[local_ref]
            action_buttons -> gtk::Box {
                set_hexpand: true,
                set_orientation: gtk4::Orientation::Horizontal,
                set_homogeneous: true,
            }
        }
    }

    fn init_widgets(
        &mut self,
        index: &Self::Index,
        root: Self::Root,
        _returned_widget: &<Self::ParentWidget as relm4::factory::FactoryView>::ReturnedWidget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let action_buttons = self.actions_factory.widget();

        let widgets = view_output!();

        widgets
    }

    fn init_model(
        (mut dbus_notification, config): Self::Init,
        index: &Self::Index,
        sender: FactorySender<Self>,
    ) -> Self {
        let mut timeout = if dbus_notification.expire_timeout == -1 {
            config.timeout
        } else {
            dbus_notification.expire_timeout as u32
        };

        let default_action = dbus_notification.actions.remove(DEFAULT_ACTION);

        let mut actions_factory: FactoryVecDeque<ActionButton> = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(
                sender.output_sender(),
                glib::clone!(
                    #[strong]
                    index,
                    move |output| {
                        NotificationOutput::ActionInvoked {
                            index: index.clone(),
                            action: output,
                        }
                    }
                ),
            );

        // If notification has 2 or more actions alongside a default
        // disable timeout
        //
        // Odds are the notification wants some user input (looking at you blueman)
        if dbus_notification.actions.len() >= 2 {
            timeout = 0;
        }

        for (action, display) in dbus_notification.actions {
            info!("Action added for notification: {}, {}", action, display);
            actions_factory.guard().push_back((action, display));
        }

        let index = index.clone();

        if timeout > 0 {
            sender.oneshot_command(async move {
                tokio::time::sleep(Duration::from_secs(timeout as u64)).await;
                NotificationOutput::Close {
                    index,
                    reason: NotificationCloseReason::Expired,
                }
            });
        }

        let icon = if let Some(data) = dbus_notification.image_data {
            let format = if data.has_alpha {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            };
            let tex = gdk::MemoryTexture::new(
                data.width,
                data.height,
                format,
                &glib::Bytes::from_owned(data.data),
                data.rowstride as usize,
            );
            NotificationIcon::Data(tex.into())
        } else if let Some(path) = dbus_notification.image_path {
            NotificationIcon::Path(path)
        } else if !dbus_notification.app_icon.is_empty() {
            NotificationIcon::Name(dbus_notification.app_icon)
        } else {
            NotificationIcon::None
        };

        Self {
            icon,
            default_action,
            actions_factory,
            id: dbus_notification.id,
            app_name: dbus_notification.app_name,
            summary: dbus_notification.summary,
            body: dbus_notification.body,
            urgency: dbus_notification.urgency.unwrap_or_default(),
        }
    }

    fn update_cmd(&mut self, message: Self::CommandOutput, sender: FactorySender<Self>) {
        sender.output_sender().emit(message);
    }
}
