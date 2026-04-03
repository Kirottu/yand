use std::{
    cell::RefCell, collections::HashMap, fmt::Display, fs, path::PathBuf, rc::Rc, time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
use gtk::{gdk, prelude::*};
use gtk4::{self as gtk, gio, glib};
use log::error;
use notification::{Notification, NotificationOutput};
use relm4::{ComponentBuilder, Sender, prelude::*};
use serde::Deserialize;

use crate::notification::{
    ImageData, NotificationCloseReason, NotificationInit, NotificationInput,
};

mod notification;

const INTERFACE_XML: &str = r#"
<node>
    <interface name="org.freedesktop.Notifications">
        <method name="GetCapabilities">
            <arg type="as" name="capabilities" direction="out"/>
        </method>
        <method name="Notify">
            <arg type="s" name="app_name" direction="in"/> 
            <arg type="u" name="replaces_id" direction="in"/>
            <arg type="s" name="app_icon" direction="in"/>
            <arg type="s" name="summary" direction="in"/>
            <arg type="s" name="body" direction="in"/>
            <arg type="as" name="actions" direction="in"/>
            <arg type="a{sv}" name="hints" direction="in"/>
            <arg type="i" name="expire_timeout" direction="in"/>
            <arg type="u" name="id" direction="out"/>
        </method>
        <method name="CloseNotification">
            <arg type="u" name="id" direction="in"/>
        </method>
        <method name="GetServerInformation">
            <arg type="s" name="name" direction="out"/>
            <arg type="s" name="vendor" direction="out"/>
            <arg type="s" name="version" direction="out"/>
            <arg type="s" name="spec_version" direction="out"/>
        </method>

        <signal name="NotificationClosed">
            <arg type="u" name="id"/>
            <arg type="u" name="reason"/>
        </signal>
        <signal name="ActionInvoked">
            <arg type="u" name="id"/>
            <arg type="s" name="action_key"/>
        </signal>
        <signal name="ActivationToken">
            <arg type="u" name="id"/>
            <arg type="s" name="activation_token"/>
        </signal>
    </interface>
    <interface name="com.kirottu.Yand">
        <method name="Reload"/>
        <method name="SetOffset">
            <arg type="i" name="offset" direction="in"/>
        </method>
        <property type="s" name="NotificationLevel" access="readwrite"/>
    </interface>
</node>
"#;
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATIONS_IFACE: &str = "org.freedesktop.Notifications";
const CONTROL_PATH: &str = "/com/kirottu/Yand";
const CONTROL_IFACE: &str = "com.kirottu.Yand";

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the notification daemon
    Daemon,
    /// Reload config and style files
    Reload,
    /// Manage notification level
    Level {
        /// Set the notification level to this value
        level: Option<NotificationLevel>,
    },
    /// Set a vertical offset for notifications temporarily.
    /// Useful for making sure notifications align with possibly dynamic UI elements.
    SetOffset { offset: i32 },
}
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

impl From<ConfigLayer> for gtk4_layer_shell::Layer {
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
    margin_anchor: i32,
    margin_side: i32,
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
            spacing: 20,
            margin_side: 10,
            margin_anchor: 10,
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

#[derive(Debug, glib::Variant)]
struct NotifyArgs {
    app_name: String,
    replaces_id: u32,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, glib::Variant>,
    expire_timeout: i32,
}

impl NotifyArgs {
    fn into_notification_init(self, id: u32) -> NotificationInit {
        let actions = self
            .actions
            .chunks_exact(2)
            .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
            .collect();

        let mut init = NotificationInit {
            id,
            app_name: self.app_name,
            app_icon: self.app_icon,
            summary: self.summary,
            body: self.body,
            actions,
            expire_timeout: self.expire_timeout,
            action_icons: None,
            image_data: None,
            urgency: None,
            image_path: None,
            resident: None,
        };

        for (key, value) in self.hints {
            match key.as_str() {
                "action-icons" => init.action_icons = FromVariant::from_variant(&value),
                "image-data" => {
                    let data: Option<(i32, i32, i32, bool, i32, i32, Vec<u8>)> =
                        FromVariant::from_variant(&value);
                    if let Some((
                        width,
                        height,
                        rowstride,
                        has_alpha,
                        _bits_per_sample,
                        _channels,
                        data,
                    )) = data
                    {
                        init.image_data = Some(ImageData {
                            width,
                            height,
                            rowstride,
                            has_alpha,
                            data,
                        });
                    }
                }
                "image-path" => init.image_path = FromVariant::from_variant(&value),
                "resident" => init.resident = FromVariant::from_variant(&value),
                "urgency" => init.urgency = u8::from_variant(&value).map(Into::into),
                _ => (),
            }
        }

        init
    }
}

#[derive(Debug, glib::Variant)]
struct CloseNotificationArgs {
    id: u32,
}

#[derive(Debug)]
enum NotificationMethod {
    GetCapabilities,
    Notify(NotifyArgs),
    CloseNotification(CloseNotificationArgs),
    GetServerInformation,
}

impl DBusMethodCall for NotificationMethod {
    fn parse_call(
        _obj_path: &str,
        _interface: Option<&str>,
        method: &str,
        params: glib::Variant,
    ) -> Result<Self, glib::Error> {
        match method {
            "GetCapabilities" => Ok(Some(Self::GetCapabilities)),
            "Notify" => Ok(params.get::<NotifyArgs>().map(Self::Notify)),
            "CloseNotification" => Ok(params
                .get::<CloseNotificationArgs>()
                .map(Self::CloseNotification)),
            "GetServerInformation" => Ok(Some(Self::GetServerInformation)),
            _ => Err(glib::Error::new(
                gio::DBusError::UnknownMethod,
                "No such method",
            )),
        }
        .and_then(|p| {
            p.ok_or_else(|| glib::Error::new(gio::DBusError::InvalidArgs, "Invalid parameters"))
        })
    }
}

#[derive(Debug, glib::Variant)]
struct SetOffsetArgs {
    offset: i32,
}

enum ControlMethod {
    Reload,
    SetOffset(SetOffsetArgs),
}

impl DBusMethodCall for ControlMethod {
    fn parse_call(
        _obj_path: &str,
        _interface: Option<&str>,
        method: &str,
        params: glib::Variant,
    ) -> Result<Self, glib::Error> {
        match method {
            "Reload" => Ok(Some(Self::Reload)),
            "SetOffset" => Ok(params.get::<SetOffsetArgs>().map(Self::SetOffset)),
            _ => Err(glib::Error::new(
                gio::DBusError::UnknownMethod,
                "No such method",
            )),
        }
        .and_then(|p| {
            p.ok_or_else(|| glib::Error::new(gio::DBusError::InvalidArgs, "Invalid parameters"))
        })
    }
}

#[derive(Debug, Clone, Copy, glib::Variant, ValueEnum, Default)]
pub enum NotificationLevel {
    #[default]
    Normal,
    Dnd,
}

impl Display for NotificationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotificationLevel::Normal => f.write_str("normal"),
            NotificationLevel::Dnd => f.write_str("dnd"),
        }
    }
}

// Daemon side state per every notification
struct NotificationState {
    id: u32,
    sender: Sender<NotificationInput>,
    window: gtk::Window,
}

struct DaemonState {
    config: Config,
    config_path: PathBuf,
    style_path: PathBuf,
    css_provider: gtk::CssProvider,

    notification_level: NotificationLevel,
    notifications: Vec<NotificationState>,
    // The ID for the next notification that will be created
    next_id: u32,
    // A temporary extra offset managed with IPC. Useful for making sure notifications
    // align with dynamically placed panels
    offset: i32,
}

impl DaemonState {
    fn reload(&mut self) {
        let display = gdk::Display::default().unwrap();
        self.config = if let Ok(str) = fs::read_to_string(&self.config_path) {
            toml::from_str::<Config>(&str).unwrap_or_else(|why| {
                error!("Failed to parse config file: {}", why);
                Config::default()
            })
        } else {
            Config::default()
        };

        gtk::style_context_remove_provider_for_display(&display, &self.css_provider);

        self.css_provider = gtk::CssProvider::new();

        if let Ok(str) = fs::read_to_string(&self.style_path) {
            self.css_provider.load_from_string(&str);
        } else {
            self.css_provider
                .load_from_string(include_str!("../res/style.css"));
        }
        gtk::style_context_add_provider_for_display(
            &display,
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        log::info!("Config reloaded");

        self.recalculate_offsets();
    }

    // Before this is called, the notifications vector should be "clean"
    fn recalculate_offsets(&self) {
        let mut offset = self.offset;
        for state in &self.notifications {
            state.sender.emit(NotificationInput::ChangeOffset(offset));
            offset += self.config.spacing + state.window.height();
        }
    }
}

fn main() {
    let args = Args::parse();
    let flags = if matches!(args.command, Command::Daemon) {
        gio::ApplicationFlags::IS_SERVICE
    } else {
        Default::default()
    };

    colog::init();

    let app = gtk::Application::new(Some(NOTIFICATIONS_IFACE), flags);
    app.register(Option::<&gio::Cancellable>::None).unwrap();

    let dbus_conn = app.dbus_connection().unwrap();

    let node_info = gio::DBusNodeInfo::for_xml(INTERFACE_XML).unwrap();

    let control_iface = node_info.lookup_interface(CONTROL_IFACE).unwrap();

    let control_proxy = gio::DBusProxy::new_sync(
        &dbus_conn,
        gio::DBusProxyFlags::empty(),
        Some(&control_iface),
        Some(NOTIFICATIONS_IFACE),
        CONTROL_PATH,
        CONTROL_IFACE,
        Option::<&gio::Cancellable>::None,
    )
    .unwrap();

    match args.command {
        Command::SetOffset { offset } => {
            control_proxy
                .call_sync(
                    "SetOffset",
                    Some(&(offset,).to_variant()),
                    gio::DBusCallFlags::NONE,
                    100,
                    Option::<&gio::Cancellable>::None,
                )
                .unwrap();
            app.run_with_args(&Vec::<String>::new());
        }
        Command::Level { level } => {
            let property_proxy = gio::DBusProxy::new_sync(
                &dbus_conn,
                gio::DBusProxyFlags::empty(),
                Some(&control_iface),
                Some(NOTIFICATIONS_IFACE),
                CONTROL_PATH,
                "org.freedesktop.DBus.Properties",
                Option::<&gio::Cancellable>::None,
            )
            .unwrap();
            match level {
                Some(level) => {
                    property_proxy
                        .call_sync(
                            "Set",
                            Some(&(level,).to_variant()),
                            gio::DBusCallFlags::NONE,
                            100,
                            Option::<&gio::Cancellable>::None,
                        )
                        .unwrap();
                }
                None => {
                    let level: (NotificationLevel,) = FromVariant::from_variant(
                        &property_proxy
                            .call_sync(
                                "Get",
                                None,
                                gio::DBusCallFlags::NONE,
                                100,
                                Option::<&gio::Cancellable>::None,
                            )
                            .unwrap(),
                    )
                    .unwrap();
                    println!("{}", level.0);
                }
            }
            app.run_with_args(&Vec::<String>::new());
        }
        Command::Reload => {
            control_proxy
                .call_sync(
                    "Reload",
                    None,
                    gio::DBusCallFlags::NONE,
                    100,
                    Option::<&gio::Cancellable>::None,
                )
                .unwrap();
            app.run_with_args(&Vec::<String>::new());
        }
        Command::Daemon => {
            let notification_iface = node_info.lookup_interface(NOTIFICATIONS_IFACE).unwrap();
            let _hold_guard = app.hold();

            let dirs = xdg::BaseDirectories::with_prefix("yand");

            let config_path = dirs
                .place_config_file("config.toml")
                .expect("Failed to create config directory");
            let style_path = dirs
                .get_config_file("style.css")
                .expect("Failed to get style path");

            let state = Rc::new(RefCell::new(DaemonState {
                config_path,
                style_path,
                css_provider: gtk::CssProvider::new(),
                config: Config::default(),
                notifications: Vec::new(),
                next_id: 1,
                notification_level: NotificationLevel::default(),
                offset: 0,
            }));

            state.borrow_mut().reload();

            dbus_conn
                .register_object(NOTIFICATIONS_PATH, &notification_iface)
                .typed_method_call::<NotificationMethod>()
                .invoke(glib::clone!(
                    #[weak_allow_none]
                    app,
                    #[strong]
                    state,
                    move |conn, _sender, method, invocation| {
                        let app = app.unwrap();
                        notification_handler(app, state.clone(), conn, method, invocation);
                    }
                ))
                .build()
                .unwrap();

            dbus_conn
                .register_object("/com/kirottu/Yand", &control_iface)
                .property(glib::clone!(
                    #[strong]
                    state,
                    move |_conn, _sender, _path, _interface, name| {
                        match name {
                            "NotificationLevel" => state.borrow().notification_level.to_variant(),
                            _ => ().to_variant(),
                        }
                    }
                ))
                .set_property(glib::clone!(
                    #[strong]
                    state,
                    move |_conn, _sender, _path, _interface, name, val| {
                        match name {
                            "NotificationLevel" => {
                                if let Some(level) = NotificationLevel::from_variant(&val) {
                                    state.borrow_mut().notification_level = level;
                                    true
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        }
                    }
                ))
                .typed_method_call::<ControlMethod>()
                .invoke(glib::clone!(
                    #[strong]
                    state,
                    move |_conn, _sender, method, invocation| {
                        match method {
                            ControlMethod::Reload => {
                                state.borrow_mut().reload();
                                invocation.return_value(None);
                            }
                            ControlMethod::SetOffset(args) => {
                                state.borrow_mut().offset = args.offset;
                                state.borrow().recalculate_offsets();
                                invocation.return_value(None);
                            }
                        }
                    }
                ))
                .build()
                .unwrap();

            log::info!("Starting Yand");

            app.run_with_args(&Vec::<String>::new());
        }
    }
}

fn notification_handler(
    app: gtk::Application,
    state: Rc<RefCell<DaemonState>>,
    conn: gio::DBusConnection,
    method: NotificationMethod,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        NotificationMethod::GetCapabilities => {
            invocation.return_value(Some(
                &(vec![
                    "action-icons",
                    "actions",
                    "body",
                    "body-markup",
                    "icon-static",
                ],)
                    .to_variant(),
            ));
        }
        NotificationMethod::Notify(args) => {
            let mut _state = state.borrow_mut();
            let id = if args.replaces_id == 0 {
                let id = _state.next_id;
                _state.next_id += 1;
                id
            } else {
                args.replaces_id
            };
            log::info!("Notification {id} received: {}", args.summary);

            match _state.notification_level {
                NotificationLevel::Normal => {
                    let init = args.into_notification_init(id);

                    glib::idle_add_local_once(glib::clone!(
                        #[strong]
                        state,
                        move || {
                            state.borrow().recalculate_offsets();
                        }
                    ));

                    let builder = ComponentBuilder::<Notification>::default();
                    let connector = builder.launch((init, _state.config.clone()));

                    let mut controller = connector.connect_receiver(glib::clone!(
                        #[strong]
                        state,
                        #[strong]
                        conn,
                        move |sender, message| match message {
                            NotificationOutput::Closed { id, reason } => {
                                log::info!("Notification {id} closed: {reason:?}");
                                let mut _state = state.borrow_mut();

                                _state
                                    .notifications
                                    .retain(|notification| notification.id != id);

                                glib::idle_add_local_once(glib::clone!(
                                    #[strong]
                                    state,
                                    move || {
                                        state.borrow().recalculate_offsets();
                                    }
                                ));
                                conn.emit_signal(
                                    None,
                                    NOTIFICATIONS_PATH,
                                    NOTIFICATIONS_IFACE,
                                    "NotificationClosed",
                                    Some(&(id, u32::from(reason)).to_variant()),
                                )
                                .unwrap();

                                // These need to be periodically cleared, and when all notifications have been closed it is
                                // an excellent time to do so
                                if _state.notifications.is_empty() {
                                    relm4::runtime_util::shutdown_all();
                                }
                            }
                            NotificationOutput::ActionInvoked { id, action } => {
                                log::info!("Notification {id} action invoked: {action}");
                                sender
                                    .send(notification::NotificationInput::Close(
                                        NotificationCloseReason::DismissedByUser,
                                    ))
                                    .unwrap();

                                // Does not work right now, and does some weird stuff
                                // let display = gdk::Display::default().unwrap();
                                // let ctx = display.app_launch_context();
                                // if let Some(token) =
                                //     ctx.startup_notify_id(Option::<&gio::AppInfo>::None, &[])
                                // {
                                //     log::info!("{token}");
                                //     conn.emit_signal(
                                //         None,
                                //         NOTIFICATIONS_PATH,
                                //         NOTIFICATIONS_IFACE,
                                //         "ActivationToken",
                                //         Some(&(id, token.to_string()).to_variant()),
                                //     )
                                //     .unwrap();
                                // }

                                conn.emit_signal(
                                    None,
                                    NOTIFICATIONS_PATH,
                                    NOTIFICATIONS_IFACE,
                                    "ActionInvoked",
                                    Some(&(id, action).to_variant()),
                                )
                                .unwrap();
                            }
                        }
                    ));

                    let window = controller.widget();
                    app.add_window(window);
                    window.set_visible(true);

                    _state.notifications.push(NotificationState {
                        id,
                        sender: controller.sender().clone(),
                        window: window.clone(),
                    });

                    controller.detach_runtime();
                }
                NotificationLevel::Dnd => {
                    // Send an event regarding the closure after a little bit
                    glib::timeout_add_once(Duration::from_millis(100), move || {
                        conn.emit_signal(
                            None,
                            NOTIFICATIONS_PATH,
                            NOTIFICATIONS_IFACE,
                            "NotificationClosed",
                            Some(&(id, u32::from(NotificationCloseReason::Undefined)).to_variant()),
                        )
                        .unwrap();
                    });
                }
            }
            invocation.return_value(Some(&(id,).to_variant()));
        }
        NotificationMethod::CloseNotification(close_notification_args) => {
            if let Some(notification) = state
                .borrow()
                .notifications
                .iter()
                .find(|n| n.id == close_notification_args.id)
            {
                notification.sender.emit(NotificationInput::Close(
                    NotificationCloseReason::DismissedByApp,
                ));
            }
            invocation.return_value(None);
        }
        NotificationMethod::GetServerInformation => {
            invocation.return_value(Some(
                &("Yand", "Kirottu", env!("CARGO_PKG_VERSION"), "1.3").to_variant(),
            ));
        }
    }
}
