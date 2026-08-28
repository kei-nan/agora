mod account_pin;
mod commands;
mod rpc;
mod vector_search;

use account_pin::AccountPinStore;
use commands::{agent, auth, chain};
use auth::{PendingSessions, SessionStore};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PendingSessions::new())
        .manage(SessionStore::new())
        .setup(|app| {
            // Local trust-on-first-use pin store for the QR-auth callback's nullifier ->
            // AccountId lookup (see account_pin.rs's module doc). This is the real, narrower
            // mitigation for the finding that `lookup_registered_account` (commands/chain.rs)
            // trusts an unauthenticated RPC response with no state-proof verification: every
            // resolved account is checked against this store before a session can be minted.
            // Persisted under this install's app data dir so a pin survives an app restart —
            // an in-memory-only pin would hand an attacker a fresh TOFU race on every launch.
            let base_dir = app.path().app_data_dir().unwrap_or_else(|e| {
                eprintln!(
                    "[account_pin] could not resolve app data dir ({e}); falling back to the \
                     system temp dir for the QR-auth account pin store"
                );
                std::env::temp_dir()
            });
            app.manage(AccountPinStore::load(base_dir.join("account_pins.json")));
            Ok(())
        })
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            chain::chain_status,
            chain::fetch_proposals,
            chain::fetch_laws,
            chain::fetch_treasury,
            chain::fetch_department_budgets,
            chain::fetch_rulings,
            chain::fetch_oracle_council_info,
            chain::fetch_oracle_pending_approvals,
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
