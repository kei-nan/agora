mod commands;
mod rpc;

use commands::{agent, auth, chain};
use auth::{PendingSessions, SessionStore};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PendingSessions::new())
        .manage(SessionStore::new())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            chain::chain_status,
            chain::fetch_proposals,
            chain::fetch_laws,
            chain::fetch_treasury,
            chain::fetch_department_budgets,
            chain::fetch_rulings,
            chain::fetch_ipfs_content,
            chain::auth_verify_nullifier,
            chain::fetch_legislature_data,
            chain::fetch_elections_data,
            chain::fetch_anticorruption_data,
            chain::chain_submit_extrinsic,
            auth::auth_generate_challenge,
            auth::auth_poll_session,
            auth::auth_start_callback_server,
            agent::agent_ask,
        ])
        .run(tauri::generate_context!())
        .expect("error while running democracy chain desktop");
}
