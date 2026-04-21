use std::{fmt::Display, time::Duration};

use gtk::{gdk, glib, pango, prelude::*};
use gtk4 as gtk;
use gtk4_layer_shell::LayerShell;
use log::info;
use relm4::prelude::*;

use crate::{Config, ConfigOverrides};

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

#[derive(Debug, Default, Clone, Copy)]
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
    pub image_data: Option<ImageData>,
    pub image_path: Option<String>,
    pub resident: Option<bool>,
    pub urgency: Option<Urgency>,
    // Extra data
    // pub offset: i32,
}

impl NotificationInit {
    fn icon(&self) -> NotificationIcon {
        if let Some(data) = &self.image_data {
            let format = if data.has_alpha {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            };
            let tex = gdk::MemoryTexture::new(
                data.width,
                data.height,
                format,
                &glib::Bytes::from_owned(data.data.clone()),
                data.rowstride as usize,
            );
            NotificationIcon::Data(tex.into())
        } else if let Some(path) = &self.image_path {
            if let Ok((path, _)) = glib::filename_from_uri(path) {
                NotificationIcon::Path(path.to_string_lossy().to_string())
            } else {
                NotificationIcon::Path(path.clone())
            }
        } else if !self.app_icon.is_empty() {
            // The spec allows for URIs in the app_icon field, but GTK is not a fan of them. So we must commit this
            // atrocity
            if let Ok((path, _)) = glib::filename_from_uri(&self.app_icon) {
                NotificationIcon::Path(path.to_string_lossy().to_string())
            } else {
                NotificationIcon::Name(self.app_icon.clone())
            }
        } else {
            NotificationIcon::None
        }
    }

    fn default_action(&mut self) -> Option<String> {
        let default_action_index = self
            .actions
            .iter()
            .enumerate()
            .find_map(|(i, (key, _))| if key == DEFAULT_ACTION { Some(i) } else { None });

        default_action_index.map(|i| self.actions.remove(i).1)
    }

    /// Determine the timeout that should be used.
    ///
    /// Must be called after `Self::default_action` to make sure the action Vec is representative of what
    /// is shown to users
    fn timeout(&self, config: &Config, overrides: &ConfigOverrides) -> Option<u32> {
        // If notification has 2 or more actions alongside a default
        // disable timeout
        //
        // Odds are the notification wants some user input (looking at you blueman)
        if self.actions.len() >= 2 {
            None
        } else if self.expire_timeout == -1 || overrides.timeout {
            Some(config.timeout)
        } else {
            Some(self.expire_timeout as u32)
        }
        .and_then(|timeout| if timeout == 0 { None } else { Some(timeout) })
    }
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
    Replace(Box<<Notification as Component>::Init>),
}

#[derive(Debug)]
pub struct Notification {
    pub id: u32,
    app_name: String,
    summary: String,
    body: String,
    urgency: Urgency,

    // Watched variables
    offset: i32,
    opacity: f64,

    config: Config,

    icon_widget: gtk::Image,
    actions_factory: FactoryVecDeque<ActionButton>,
    default_action: Option<String>,
    /// The ID to the glib timeout for possible cancellation during a replace event
    timeout_source_id: Option<glib::SourceId>,
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
            set_margin: (gtk4_layer_shell::Edge::Right, model.config.margin_side),
            #[watch]
            set_margin: (gtk4_layer_shell::Edge::Top, model.offset),
            set_namespace: Some("yand"),
            #[watch]
            set_opacity: model.opacity,
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
                #[watch]
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
                    #[watch]
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
                        #[local_ref]
                        icon_widget -> gtk::Image {
                            #[watch]
                            set_pixel_size: model.config.icon_size,
                            set_css_classes: &["icon"],
                        },
                    },

                    attach[1, 0, 1, 1] = &gtk::Label {
                        #[watch]
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
        (mut notification_init, config): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (config, overrides) = config.clone().overridden(&notification_init.app_name);

        let id = notification_init.id;

        // If a factory already exists for the purposes of a notification replacement, use it
        let mut actions_factory: FactoryVecDeque<ActionButton> = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(
                sender.output_sender(),
                glib::clone!(move |output| {
                    NotificationOutput::ActionInvoked { id, action: output }
                }),
            );

        let default_action = notification_init.default_action();

        let icon = notification_init.icon();

        for (action, display) in notification_init.actions.clone() {
            info!("Action added for notification: {}, {}", action, display);
            actions_factory.guard().push_back((action, display));
        }

        let icon_widget = gtk::Image::new();

        let mut model = Self {
            offset: config.margin_anchor,
            // Opacity is set to 0 initially to make sure the window isn't visible before the correct position has been configured
            opacity: 0.0,
            config,
            icon_widget: icon_widget.clone(),
            default_action,
            actions_factory,
            id: notification_init.id,
            app_name: notification_init.app_name.clone(),
            summary: notification_init.summary.clone(),
            // Remove all newlines to make sure GTK can properly truncate the label
            // TODO: Configurable, figure out a better way to do this
            body: notification_init.body.replace('\n', " "),
            urgency: notification_init.urgency.unwrap_or_default(),
            timeout_source_id: None,
        };

        model.set_timeout(&notification_init, &overrides, sender.clone());
        model.set_icon(icon);

        let action_buttons = model.actions_factory.widget();

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            NotificationInput::ChangeOffset(offset) => {
                self.offset = self.config.margin_anchor + offset;
                self.opacity = 1.0;
            }
            NotificationInput::Close(reason) => {
                // For some reason, this fixes things.
                root.set_visible(false);
                root.close();
                sender
                    .output(NotificationOutput::Closed {
                        id: self.id,
                        reason,
                    })
                    .unwrap();
            }
            NotificationInput::Replace(init) => {
                let (mut notification_init, config) = *init;
                let icon = notification_init.icon();
                let default_action = notification_init.default_action();
                let (config, overrides) = config.clone().overridden(&notification_init.app_name);
                self.set_timeout(&notification_init, &overrides, sender);

                self.actions_factory.guard().clear();
                for (action, display) in notification_init.actions {
                    info!("Action added for notification: {}, {}", action, display);
                    self.actions_factory.guard().push_back((action, display));
                }

                self.set_icon(icon);
                self.default_action = default_action;
                self.config = config;
                self.summary = notification_init.summary;
                self.body = notification_init.body;
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

impl Notification {
    fn set_timeout(
        &mut self,
        notification_init: &NotificationInit,
        overrides: &ConfigOverrides,
        sender: ComponentSender<Self>,
    ) {
        // Cancel existing timeout
        if let Some(source_id) = self.timeout_source_id.take() {
            source_id.remove();
        }
        if let Some(timeout) = notification_init.timeout(&self.config, overrides) {
            let source_id = glib::timeout_add_local_once(
                Duration::from_secs(timeout as u64),
                glib::clone!(
                    #[strong]
                    sender,
                    move || {
                        // This will error out if the notification was already closed for some other reason,
                        // so we just discard the error
                        let _ = sender
                            .input_sender()
                            .send(NotificationInput::Close(NotificationCloseReason::Expired));
                    }
                ),
            );
            self.timeout_source_id = Some(source_id);
        }
    }

    fn set_icon(&self, icon: NotificationIcon) {
        match &icon {
            NotificationIcon::Path(path) => self.icon_widget.set_from_file(Some(path)),
            NotificationIcon::Name(name) => self.icon_widget.set_icon_name(Some(name)),
            NotificationIcon::Data(texture) => self.icon_widget.set_paintable(Some(texture)),
            NotificationIcon::None => self.icon_widget.set_visible(false),
        }
    }
}
