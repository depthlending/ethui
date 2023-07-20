use std::path::PathBuf;

use iron_db::DB;
use iron_types::ui_events::UIReceiver;
use iron_types::{ui_events, UIEvent, UISender};
use tauri::{
    AppHandle, Builder, CustomMenuItem, GlobalWindowEvent, Manager, SystemTray, SystemTrayEvent,
    SystemTrayMenu, SystemTrayMenuItem, WindowBuilder, WindowEvent, WindowUrl,
};
#[cfg(not(target_os = "linux"))]
use tauri::{Menu, Submenu, WindowMenuEvent};
use tauri_plugin_window_state::{AppHandleExt, Builder as windowStatePlugin, StateFlags};
use tokio::sync::mpsc;

use crate::error::AppResult;

pub struct IronApp {
    app: tauri::App,
    //pub db: DB,
}

impl IronApp {
    pub async fn build() -> AppResult<Self> {
        let (snd, rcv) = mpsc::unbounded_channel();

        let tray = Self::build_tray();

        let mut builder = Builder::default()
            .plugin(windowStatePlugin::default().build())
            .invoke_handler(tauri::generate_handler![
                iron_settings::commands::settings_get,
                iron_settings::commands::settings_set,
                iron_settings::commands::settings_set_dark_mode,
                iron_settings::commands::settings_set_alias,
                iron_settings::commands::settings_get_alias,
                iron_networks::commands::networks_get_list,
                iron_networks::commands::networks_get_current,
                iron_networks::commands::networks_set_list,
                iron_networks::commands::networks_set_current,
                iron_networks::commands::networks_reset,
                iron_db::commands::db_get_transactions,
                iron_db::commands::db_get_contracts,
                iron_db::commands::db_get_erc20_balances,
                iron_db::commands::db_get_native_balance,
                iron_ws::commands::ws_get_all_peers,
                iron_wallets::commands::wallets_get_all,
                iron_wallets::commands::wallets_get_current,
                iron_wallets::commands::wallets_get_current_address,
                iron_wallets::commands::wallets_set_list,
                iron_wallets::commands::wallets_create,
                iron_wallets::commands::wallets_update,
                iron_wallets::commands::wallets_remove,
                iron_wallets::commands::wallets_set_current_wallet,
                iron_wallets::commands::wallets_set_current_path,
                iron_wallets::commands::wallets_get_wallet_addresses,
                iron_wallets::commands::wallets_get_mnemonic_addresses,
                iron_dialogs::commands::dialog_get_payload,
                iron_dialogs::commands::dialog_send,
                iron_dialogs::commands::dialog_finish,
                iron_forge::commands::foundry_get_abi,
                iron_sync_alchemy::commands::alchemy_fetch_erc20_balances,
                iron_sync_alchemy::commands::alchemy_fetch_native_balance,
                iron_sync_alchemy::commands::alchemy_fetch_transactions,
                iron_rpc::commands::rpc_send_transaction,
            ])
            .setup(|app| {
                let handle = app.handle();

                tauri::async_runtime::spawn(async move {
                    event_listener(handle, rcv).await;
                });

                #[cfg(feature = "debug")]
                if std::env::var("IRON_OPEN_DEVTOOLS").is_ok() {
                    let window = app.get_window("main").unwrap();
                    window.open_devtools();
                }

                Ok(())
            })
            .on_window_event(on_window_event);

        #[cfg(not(target_os = "macos"))]
        {
            builder = builder
                .system_tray(tray)
                .on_system_tray_event(on_system_tray_event);
        }

        #[cfg(not(target_os = "linux"))]
        {
            let menu = Self::build_menu();
            builder = builder.menu(menu).on_menu_event(on_menu_event)
        }

        let app = builder
            .build(tauri::generate_context!())
            .expect("error while running tauri application");

        let db = DB::connect(&resource(&app, "db.sqlite3")).await?;
        init(&app, &db, snd).await?;

        app.manage(db);
        let res = Self { app };

        Ok(res)
    }

    pub fn run(self) {
        self.app.run(|_, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
    }

    #[cfg(not(target_os = "linux"))]
    fn build_menu() -> Menu {
        let details = CustomMenuItem::new("details".to_string(), "Details");
        let transactions = CustomMenuItem::new("transactions".to_string(), "Transactions");
        let balances = CustomMenuItem::new("balances".to_string(), "Balances");
        let contracts = CustomMenuItem::new("contracts".to_string(), "Contracts");
        let connections = CustomMenuItem::new("connections".to_string(), "Connections");
        let go_submenu = Submenu::new(
            "Go",
            Menu::new()
                .add_item(details)
                .add_item(transactions)
                .add_item(balances)
                .add_item(contracts)
                .add_item(connections),
        );

        Menu::os_default("Iron").add_submenu(go_submenu)
    }

    fn build_tray() -> tauri::SystemTray {
        let quit = CustomMenuItem::new("quit".to_string(), "Quit");
        let hide = CustomMenuItem::new("hide".to_string(), "Hide");
        let show = CustomMenuItem::new("show".to_string(), "Show");
        let tray_menu = SystemTrayMenu::new()
            .add_item(show)
            .add_item(hide)
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(quit);

        SystemTray::new().with_menu(tray_menu)
    }
}

#[cfg(not(target_os = "linux"))]
fn on_menu_event(event: WindowMenuEvent) {
    match event.menu_item_id() {
        "quit" => {
            std::process::exit(0);
        }
        "close" => {
            event.window().close().unwrap();
        }
        path => {
            event.window().emit("go", path).unwrap();
        }
    }
}

async fn init(app: &tauri::App, db: &DB, snd: UISender) -> AppResult<()> {
    iron_dialogs::init(snd.clone());
    iron_settings::init(resource(app, "settings.json")).await;
    iron_ws::init(snd.clone()).await;
    iron_wallets::init(resource(app, "wallets.json"), snd.clone()).await;
    iron_networks::init(resource(app, "networks.json"), snd.clone()).await;
    iron_forge::init().await?;
    iron_sync_alchemy::init(db.clone(), snd.clone()).await;
    iron_sync_anvil::init(db.clone(), snd.clone());

    Ok(())
}

fn on_window_event(event: GlobalWindowEvent) {
    if let WindowEvent::CloseRequested {
        #[cfg(not(target_os = "linux"))]
        api,
        ..
    } = event.event()
    {
        let window = event.window();
        let app = window.app_handle();
        let _ = app.save_window_state(StateFlags::all());

        #[cfg(target_os = "macos")]
        {
            app.hide().unwrap();
            api.prevent_close();
        }
    }
}

fn on_system_tray_event(app: &AppHandle, event: SystemTrayEvent) {
    use SystemTrayEvent::*;

    match event {
        MenuItemClick { id, .. } => match id.as_str() {
            "quit" => app.exit(0),
            "hide" => app.get_window("main").unwrap().hide().unwrap(),
            "show" => show_main_window(app),
            _ => {}
        },
        DoubleClick { .. } => show_main_window(app),
        _ => {}
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        w.show().unwrap()
    } else {
        tauri::WindowBuilder::new(app, "main", tauri::WindowUrl::App("index.html".into()))
            .build()
            .unwrap();
    }
}

async fn event_listener(handle: AppHandle, mut rcv: UIReceiver) {
    while let Some(msg) = rcv.recv().await {
        use UIEvent::*;

        match msg {
            Notify(msg) => {
                // forward directly to main window
                // if window is not open, just ignore them
                if let Some(window) = handle.get_window("main") {
                    window.emit(msg.label(), &msg).unwrap();
                }
            }

            DialogOpen(ui_events::DialogOpen {
                label,
                title,
                url,
                w,
                h,
            }) => {
                WindowBuilder::new(&handle, label, WindowUrl::App(url.into()))
                    .min_inner_size(w, h)
                    .max_inner_size(w, h)
                    .title(title)
                    .build()
                    .unwrap();
            }

            DialogClose(ui_events::DialogClose { label }) => {
                if let Some(window) = handle.get_window(&label) {
                    window.close().unwrap();
                }
            }

            DialogSend(ui_events::DialogSend {
                label,
                event_type,
                payload,
            }) => {
                handle
                    .get_window(&label)
                    .unwrap()
                    .emit(&event_type, &payload)
                    .unwrap();
            }
        }
    }
}

fn resource(app: &tauri::App, resource: &str) -> PathBuf {
    app.path_resolver()
        .resolve_resource(resource)
        .unwrap_or_else(|| panic!("failed to resolve resource {}", resource))
}