//! ACL commands — set, list, remove.

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_set"))]
pub async fn acl_set(
    state: State<'_, AppState>,
    principal: String,
    resource: String,
    permission: String,
    effect: String,
) -> Result<bool, CommandError> {
    let id = uuid::Uuid::new_v4().to_string();
    state.sqlite.insert_acl(&id, &principal, &resource, &permission, &effect)
        .map(|_| true)
        .map_err(|e| CommandError::db("acl_set", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_list"))]
pub async fn acl_list(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String, String, String, String)>, CommandError> {
    state.sqlite.list_acl()
        .map_err(|e| CommandError::db("acl_list", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_remove"))]
pub async fn acl_remove(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    state.sqlite.remove_acl(&id)
        .map(|_| true)
        .map_err(|e| CommandError::db("acl_remove", &e))
}