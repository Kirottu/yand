use clap::{Parser, Subcommand};
use dbus::ControlProxy;
use zbus::Connection;

// We only need `ControlProxy` from the dbus mod, so a lot of dead code warnings are generated
// for the `yandctl` build otherwise
#[allow(dead_code)]
mod dbus;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Reload config and style files
    Reload,
    /// Manage notification level
    Level {
        /// Set the notification level to this value
        level: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let connection = Connection::session().await.unwrap();
    let proxy = ControlProxy::new(&connection).await.unwrap();
    match args.command {
        Commands::Reload => {
            proxy.reload().await.unwrap();
        }
        Commands::Level { level } => match level {
            Some(level) => proxy.set_notification_level(level).await.unwrap(),
            None => println!("{}", proxy.notification_level().await.unwrap()),
        },
    }
}
