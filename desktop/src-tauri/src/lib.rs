mod commands;

use commands::{agent, auth, chain};
use auth::PendingSessions;
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PendingSessions(Mutex::new(HashMap::new())))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            chain::chain_status,
            chain::fetch_proposals,
            chain::fetch_laws,
            chain::fetch_treasury,
            chain::fetch_rulings,
            auth::auth_generate_challenge,
            auth::auth_poll_session,
            agent::agent_ask,
        ])
        .run(tauri::generate_context!())
        .expect("error while running democracy chain desktop");
}
