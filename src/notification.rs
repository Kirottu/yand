use std::{fmt::Display, time::Duration};

use gtk::{gdk, glib, pango, prelude::*};
use gtk4 as gtk;
use gtk4_layer_shell::LayerShell;
use log::info;
use relm4::prelude::*;

use crate::Config;

const DEFAULT_ACTION: &str = "default";

#[derive(Debug)]
struct ActionButton {
    action: String,
    display: String,
}

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

#[derive(Debug, Default)]
pub struct NotificationInit {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub expire_timeout: i32,

    // Supported hints
    pub action_icons: Option<bool>,
    pub image_data: Option<ImageData>, // TODO
    pub image_path: Option<String>,
    pub resident: Option<bool>,
    pub urgency: Option<Urgency>,

    // Extra data
    pub offset: i32,
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
    Closed {
        id: u32,
        reason: NotificationCloseReason,
    },
    ActionInvoked {
        id: u32,
        action: String,
    },
}

#[derive(Debug)]
pub enum NotificationInput {
    ChangeOffset(i32),
    Close(NotificationCloseReason),
}

#[derive(Debug)]
pub struct Notification {
    pub id: u32,
    icon: NotificationIcon,
    app_name: String,
    summary: String,
    body: String,
    urgency: Urgency,
    offset: i32,

    config: Config,

    actions_factory: FactoryVecDeque<ActionButton>,
    default_action: Option<String>,
}

#[allow(unused_assignments)]
#[relm4::component(pub)]
impl Component for Notification {
    type Init = (NotificationInit, Config);
    type Input = NotificationInput;
    type Output = NotificationOutput;
    type CommandOutput = NotificationInput;

    view! {
        gtk::Window {
            init_layer_shell: (),
            set_layer: model.config.layer.clone().into(),
            set_anchor: (gtk4_layer_shell::Edge::Right, true),
            set_anchor: (gtk4_layer_shell::Edge::Top, true),
            set_margin: (gtk4_layer_shell::Edge::Right, model.config.margin),
            #[watch]
            set_margin: (gtk4_layer_shell::Edge::Top, model.offset),
            set_namespace: Some("yand"),
            #[watch]
            set_monitor: {
                let monitors = gdk::Display::default().unwrap().monitors();

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

            #[name = "notification"]
            gtk::Box {
                set_css_classes: &["notification", &model.urgency.to_string(), &model.app_name],
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                add_controller = gtk::GestureClick {
                    connect_released: glib::clone!(
                        #[strong(rename_to = default)] model.default_action,
                        #[strong(rename_to = id)] model.id,
                        move |gesture, _, _, _| {
                            gesture.set_state(gtk::EventSequenceState::Claimed);
                            if default.is_some() {
                                sender.output(NotificationOutput::ActionInvoked {
                                    id,
                                    action: DEFAULT_ACTION.to_string()
                                }).unwrap();
                            } else {
                                sender.input(NotificationInput::Close(
                                    NotificationCloseReason::DismissedByUser
                                ));
                            }
                        }
                    )
                },

                gtk::Label {
                    set_label: &model.summary,
                    set_css_classes: &["summary"],
                    set_justify: gtk::Justification::Left,
                    set_halign: gtk::Align::Start,
                    set_wrap: false,
                    set_ellipsize: pango::EllipsizeMode::End,
                    set_use_markup: true,
                },


                gtk::Grid {
                    set_orientation: gtk4::Orientation::Horizontal,
                    set_vexpand: true,
                    set_hexpand: true,

                    // For some reason the Image becomes larger if it is not inside a Stack
                    attach[0, 0, 1, 1] = &gtk::Stack {
                        #[name = "icon"]
                        gtk::Image {
                            set_pixel_size: model.config.icon_size,
                            set_css_classes: &["icon"],
                        },
                    },

                    attach[1, 0, 1, 1] = &gtk::Label {
                        set_label: &model.body,
                        set_css_classes: &["body"],
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::Center,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_use_markup: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::Word,
                        set_wrap_mode: pango::WrapMode::WordChar,
                        set_lines: model.config.max_lines,
                        set_ellipsize: pango::EllipsizeMode::End,
                        set_visible: !model.body.is_empty(),
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
    }

    fn init(
        (mut notification_init, mut config): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut timeout_overridden = false;
        if let Some(app_override) = config
            .app_overrides
            .iter()
            .find(|app_override| app_override.app_name == notification_init.app_name)
        {
            (config, timeout_overridden) = config.clone().overridden(app_override.clone());
        }

        let mut timeout = if notification_init.expire_timeout == -1 || timeout_overridden {
            config.timeout
        } else {
            notification_init.expire_timeout as u32
        };

        let default_action_index = notification_init
            .actions
            .iter()
            .enumerate()
            .find_map(|(i, (key, _))| if key == DEFAULT_ACTION { Some(i) } else { None });

        let default_action = default_action_index.map(|i| notification_init.actions.remove(i).1);

        let id = notification_init.id;

        let mut actions_factory: FactoryVecDeque<ActionButton> = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(
                sender.output_sender(),
                glib::clone!(move |output| {
                    NotificationOutput::ActionInvoked { id, action: output }
                }),
            );

        // If notification has 2 or more actions alongside a default
        // disable timeout
        //
        // Odds are the notification wants some user input (looking at you blueman)
        if notification_init.actions.len() >= 2 {
            timeout = 0;
        }

        for (action, display) in notification_init.actions {
            info!("Action added for notification: {}, {}", action, display);
            actions_factory.guard().push_back((action, display));
        }

        if timeout > 0 {
            sender.oneshot_command(async move {
                portable_async_sleep::async_sleep(Duration::from_secs(timeout as u64)).await;
                NotificationInput::Close(NotificationCloseReason::Expired)
            });
        }

        let icon = if let Some(data) = notification_init.image_data {
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
        } else if let Some(path) = notification_init.image_path {
            NotificationIcon::Path(path)
        } else if !notification_init.app_icon.is_empty() {
            NotificationIcon::Name(notification_init.app_icon)
        } else {
            NotificationIcon::None
        };

        let model = Self {
            offset: config.margin + notification_init.offset,
            config,
            icon,
            default_action,
            actions_factory,
            id: notification_init.id,
            app_name: notification_init.app_name,
            summary: notification_init.summary,
            // Remove all newlines to make sure GTK can properly truncate the label
            // TODO: Configurable, figure out a better way to do this
            body: notification_init.body.replace('\n', " "),
            urgency: notification_init.urgency.unwrap_or_default(),
        };
        let action_buttons = model.actions_factory.widget();

        let widgets = view_output!();

        match &model.icon {
            NotificationIcon::Path(path) => widgets.icon.set_from_file(Some(path)),
            NotificationIcon::Name(name) => widgets.icon.set_icon_name(Some(name)),
            NotificationIcon::Data(texture) => widgets.icon.set_paintable(Some(texture)),
            NotificationIcon::None => widgets.icon.set_visible(false),
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            NotificationInput::ChangeOffset(offset) => {
                self.offset = self.config.margin + offset;
            }
            NotificationInput::Close(reason) => {
                root.set_visible(false);
                root.close();
                sender
                    .output(NotificationOutput::Closed {
                        id: self.id,
                        reason,
                    })
                    .unwrap();
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &<Self as Component>::Root,
    ) {
        sender.input(message);
    }
}
