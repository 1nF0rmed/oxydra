use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State, rejection::JsonRejection};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml_edit::{
    Array, DocumentMut, InlineTable, Item, Table, TableLike, Value, value as toml_value_item,
};
use types::{
    AgentConfig, RunnerControl, RunnerControlResponse, RunnerGlobalConfig, RunnerUserConfig,
};

use super::response::{ApiError, ErrorResponse, ok_response};
use super::state::WebState;
use crate::send_control_to_daemon_async;

const SECRET_SENTINEL: &str = "__UNCHANGED__";
const BACKUP_KEEP_COUNT: usize = 10;

#[derive(Debug, Serialize)]
struct PatchConfigResponse {
    changed_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_path: Option<String>,
    restart_required: bool,
}

#[derive(Debug, Serialize)]
struct ValidateConfigResponse {
    valid: bool,
    changed_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    user_id: String,
    config_path: String,
}

#[derive(Debug, Serialize)]
struct CreateUserResponse {
    user_id: String,
    config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteUserQuery {
    #[serde(default)]
    delete_config_file: bool,
}

#[derive(Debug, Serialize)]
struct DeleteUserResponse {
    user_id: String,
    deleted_config_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_path: Option<String>,
}

#[derive(Debug)]
struct PreparedPatch<T> {
    rendered_toml: String,
    _typed_config: std::marker::PhantomData<T>,
    changed_fields: Vec<String>,
    file_exists: bool,
}

/// `PATCH /api/v1/config/runner` — Apply JSON merge patch to runner config.
pub async fn patch_runner_config(
    State(state): State<Arc<WebState>>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let patch = match parse_json_payload(payload) {
        Ok(patch) => patch,
        Err(error) => return error.into_response(),
    };

    let prepared = match prepare_patch::<RunnerGlobalConfig, _>(&state.config_path, &patch, |cfg| {
        cfg.validate()
    }) {
        Ok(prepared) => prepared,
        Err(error) => return error.into_response(),
    };

    let backup_path = match persist_patch(&state.config_path, &prepared) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };

    let restart_required = any_registered_daemon_running(&state).await;

    ok_response(PatchConfigResponse {
        changed_fields: prepared.changed_fields,
        backup_path: backup_path.map(|path| path.display().to_string()),
        restart_required,
    })
    .into_response()
}

/// `POST /api/v1/config/runner/validate` — Validate runner patch without write.
pub async fn validate_runner_config(
    State(state): State<Arc<WebState>>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    validate_config_patch::<RunnerGlobalConfig, _>(&state.config_path, payload, |cfg| {
        cfg.validate()
    })
    .into_response()
}

/// `PATCH /api/v1/config/agent` — Apply JSON merge patch to workspace agent config.
pub async fn patch_agent_config(
    State(state): State<Arc<WebState>>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let patch = match parse_json_payload(payload) {
        Ok(patch) => patch,
        Err(error) => return error.into_response(),
    };
    let agent_path = state.config_dir().join("agent.toml");

    let prepared = match prepare_patch::<AgentConfig, _>(&agent_path, &patch, |cfg| cfg.validate())
    {
        Ok(prepared) => prepared,
        Err(error) => return error.into_response(),
    };

    let backup_path = match persist_patch(&agent_path, &prepared) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };

    let restart_required = any_registered_daemon_running(&state).await;

    ok_response(PatchConfigResponse {
        changed_fields: prepared.changed_fields,
        backup_path: backup_path.map(|path| path.display().to_string()),
        restart_required,
    })
    .into_response()
}

/// `POST /api/v1/config/agent/validate` — Validate agent patch without write.
pub async fn validate_agent_config(
    State(state): State<Arc<WebState>>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let agent_path = state.config_dir().join("agent.toml");
    validate_config_patch::<AgentConfig, _>(&agent_path, payload, |cfg| cfg.validate())
        .into_response()
}

/// `PATCH /api/v1/config/users/{user_id}` — Apply JSON merge patch to user config.
pub async fn patch_user_config(
    State(state): State<Arc<WebState>>,
    AxumPath(user_id): AxumPath<String>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let patch = match parse_json_payload(payload) {
        Ok(patch) => patch,
        Err(error) => return error.into_response(),
    };

    let global_config = state.latest_global_config_or_cached();
    let Some(registration) = global_config.users.get(&user_id) else {
        return not_found(format!("User `{user_id}` is not registered")).into_response();
    };
    let user_path = state.resolve_user_config_path(&registration.config_path);

    let prepared =
        match prepare_patch::<RunnerUserConfig, _>(&user_path, &patch, |cfg| cfg.validate()) {
            Ok(prepared) => prepared,
            Err(error) => return error.into_response(),
        };

    let backup_path = match persist_patch(&user_path, &prepared) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };

    let restart_required = user_daemon_running(&state, &user_id).await;

    ok_response(PatchConfigResponse {
        changed_fields: prepared.changed_fields,
        backup_path: backup_path.map(|path| path.display().to_string()),
        restart_required,
    })
    .into_response()
}

/// `POST /api/v1/config/users/{user_id}/validate` — Validate user patch without write.
pub async fn validate_user_config(
    State(state): State<Arc<WebState>>,
    AxumPath(user_id): AxumPath<String>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let global_config = state.latest_global_config_or_cached();
    let Some(registration) = global_config.users.get(&user_id) else {
        return not_found(format!("User `{user_id}` is not registered")).into_response();
    };

    let user_path = state.resolve_user_config_path(&registration.config_path);
    validate_config_patch::<RunnerUserConfig, _>(&user_path, payload, |cfg| cfg.validate())
        .into_response()
}

/// `POST /api/v1/config/users` — Register a user and create a default user config file.
pub async fn create_user(
    State(state): State<Arc<WebState>>,
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> impl IntoResponse {
    let raw_json = match parse_json_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return error.into_response(),
    };
    let request: CreateUserRequest = match serde_json::from_value(raw_json) {
        Ok(request) => request,
        Err(error) => {
            return invalid_request(format!("invalid user create payload: {error}"))
                .into_response();
        }
    };

    let user_id = match validate_user_id(&request.user_id) {
        Ok(user_id) => user_id.to_owned(),
        Err(error) => return invalid_request(error).into_response(),
    };
    if request.config_path.trim().is_empty() {
        return invalid_request("`config_path` must not be empty").into_response();
    }

    let runner_snapshot = state.latest_global_config_or_cached();
    if runner_snapshot.users.contains_key(&user_id) {
        return invalid_request(format!("User `{user_id}` is already registered")).into_response();
    }

    let user_config_path = state.resolve_user_config_path(request.config_path.trim());
    if user_config_path.exists() {
        return invalid_request(format!(
            "User config file already exists at `{}`",
            user_config_path.display()
        ))
        .into_response();
    }

    let user_parent = match user_config_path.parent() {
        Some(parent) => parent,
        None => {
            return config_write_failed(format!(
                "cannot determine parent directory for `{}`",
                user_config_path.display()
            ))
            .into_response();
        }
    };

    if let Err(error) = fs::create_dir_all(user_parent) {
        return config_write_failed(format!(
            "failed to create user config directory `{}`: {error}",
            user_parent.display()
        ))
        .into_response();
    }

    let user_toml = match toml::to_string_pretty(&RunnerUserConfig::default()) {
        Ok(content) => content,
        Err(error) => {
            return config_write_failed(format!(
                "failed to serialize default user config: {error}"
            ))
            .into_response();
        }
    };

    if let Err(error) = atomic_write(&user_config_path, &user_toml) {
        return config_write_failed(format!(
            "failed to create user config `{}`: {error}",
            user_config_path.display()
        ))
        .into_response();
    }

    let runner_patch = json!({
        "users": {
            user_id.clone(): {
                "config_path": request.config_path.trim(),
            }
        }
    });
    let prepared_runner =
        match prepare_patch::<RunnerGlobalConfig, _>(&state.config_path, &runner_patch, |cfg| {
            cfg.validate()
        }) {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = fs::remove_file(&user_config_path);
                return error.into_response();
            }
        };

    let runner_backup = match persist_patch(&state.config_path, &prepared_runner) {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&user_config_path);
            return error.into_response();
        }
    };

    ok_response(CreateUserResponse {
        user_id,
        config_path: request.config_path.trim().to_owned(),
        backup_path: runner_backup.map(|path| path.display().to_string()),
    })
    .into_response()
}

/// `DELETE /api/v1/config/users/{user_id}` — Unregister a user.
pub async fn delete_user(
    State(state): State<Arc<WebState>>,
    AxumPath(user_id): AxumPath<String>,
    Query(query): Query<DeleteUserQuery>,
) -> impl IntoResponse {
    let runner_snapshot = state.latest_global_config_or_cached();
    let Some(registration) = runner_snapshot.users.get(&user_id) else {
        return not_found(format!("User `{user_id}` is not registered")).into_response();
    };

    let user_config_path = state.resolve_user_config_path(&registration.config_path);
    let runner_patch = json!({
        "users": {
            user_id.clone(): JsonValue::Null,
        }
    });

    let prepared_runner =
        match prepare_patch::<RunnerGlobalConfig, _>(&state.config_path, &runner_patch, |cfg| {
            cfg.validate()
        }) {
            Ok(prepared) => prepared,
            Err(error) => return error.into_response(),
        };

    let runner_backup = match persist_patch(&state.config_path, &prepared_runner) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };

    let mut deleted_config_file = false;
    if query.delete_config_file && user_config_path.exists() {
        if let Err(error) = fs::remove_file(&user_config_path) {
            if let Some(ref backup_path) = runner_backup {
                let _ = fs::copy(backup_path, &state.config_path);
            }
            return config_write_failed(format!(
                "failed to remove user config `{}`: {error}",
                user_config_path.display()
            ))
            .into_response();
        }
        deleted_config_file = true;
    }

    ok_response(DeleteUserResponse {
        user_id,
        deleted_config_file,
        backup_path: runner_backup.map(|path| path.display().to_string()),
    })
    .into_response()
}

/// Creates a timestamped backup sibling file for the given path.
pub fn backup_file(path: &Path) -> Result<PathBuf, std::io::Error> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_owned());
    let backup_name = format!("{file_name}.bak.{timestamp}.{}", std::process::id());
    let backup_path = path.with_file_name(backup_name);
    fs::copy(path, &backup_path)?;
    Ok(backup_path)
}

/// Keeps only the latest `keep` backup files for `path`.
pub fn prune_backups(path: &Path, keep: usize) -> Result<(), std::io::Error> {
    let parent = match path.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_owned());
    let prefix = format!("{file_name}.bak.");

    let mut backups: Vec<PathBuf> = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();

    backups.sort_by(|a, b| {
        let a_name = a
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let b_name = b
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        b_name.cmp(&a_name)
    });

    for stale_backup in backups.into_iter().skip(keep) {
        fs::remove_file(stale_backup)?;
    }

    Ok(())
}

/// Writes content atomically by writing a sibling temp file then renaming.
pub fn atomic_write(path: &Path, content: &str) -> Result<(), std::io::Error> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let tmp_name = format!(
        "{}.tmp.{}.{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config".to_owned()),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let tmp_path = parent.join(tmp_name);

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp_path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    drop(file);

    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    if let Ok(dir_file) = OpenOptions::new().read(true).open(parent) {
        let _ = dir_file.sync_all();
    }

    Ok(())
}

/// Applies RFC 7396 JSON merge patch semantics onto a TOML document.
pub fn apply_json_merge_patch(
    document: &mut DocumentMut,
    patch: &JsonValue,
    secret_sentinel: &str,
) -> Result<(), String> {
    let patch_obj = patch
        .as_object()
        .ok_or_else(|| "patch payload must be a JSON object".to_owned())?;
    apply_patch_to_table(document.as_table_mut(), patch_obj, secret_sentinel)
}

/// Computes changed dot-paths between two JSON values.
pub fn compute_changed_fields(before: &JsonValue, after: &JsonValue) -> Vec<String> {
    let mut changed = Vec::new();
    diff_values(before, after, "", &mut changed);
    changed.sort();
    changed.dedup();
    changed
}

fn apply_patch_to_table(
    table: &mut dyn TableLike,
    patch_obj: &JsonMap<String, JsonValue>,
    secret_sentinel: &str,
) -> Result<(), String> {
    for (key, patch_value) in patch_obj {
        if matches!(patch_value, JsonValue::String(value) if value == secret_sentinel) {
            continue;
        }

        match patch_value {
            JsonValue::Null => {
                table.remove(key);
            }
            JsonValue::Object(nested) => {
                if table
                    .get(key)
                    .is_some_and(|existing| !existing.is_table_like())
                {
                    table.insert(key, Item::Table(Table::new()));
                }
                if table.get(key).is_none() {
                    table.insert(key, Item::Table(Table::new()));
                }

                let nested_table = table
                    .get_mut(key)
                    .and_then(Item::as_table_like_mut)
                    .ok_or_else(|| format!("failed to prepare table at key `{key}`"))?;
                apply_patch_to_table(nested_table, nested, secret_sentinel)?;
            }
            _ => {
                let patched_item = json_to_item(patch_value)?;
                if let Some(existing) = table.get_mut(key) {
                    *existing = patched_item;
                } else {
                    table.insert(key, patched_item);
                }
            }
        }
    }

    Ok(())
}

fn json_to_item(json_value: &JsonValue) -> Result<Item, String> {
    let converted = json_to_toml_value(json_value)?;
    Ok(toml_value_item(converted))
}

fn json_to_toml_value(value: &JsonValue) -> Result<Value, String> {
    match value {
        JsonValue::Null => Err("null values are only valid as object field removals".to_owned()),
        JsonValue::Bool(value) => Ok(Value::from(*value)),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(Value::from(value))
            } else if let Some(value) = number.as_u64() {
                let converted = i64::try_from(value)
                    .map_err(|_| format!("unsigned integer `{value}` exceeds TOML i64 range"))?;
                Ok(Value::from(converted))
            } else if let Some(value) = number.as_f64() {
                Ok(Value::from(value))
            } else {
                Err(format!("unsupported numeric value `{number}`"))
            }
        }
        JsonValue::String(value) => Ok(Value::from(value.clone())),
        JsonValue::Array(elements) => {
            let mut array = Array::new();
            for element in elements {
                array.push(json_to_toml_value(element)?);
            }
            Ok(Value::from(array))
        }
        JsonValue::Object(fields) => {
            let mut table = InlineTable::new();
            for (field, value) in fields {
                table.insert(field, json_to_toml_value(value)?);
            }
            Ok(Value::from(table))
        }
    }
}

fn diff_values(before: &JsonValue, after: &JsonValue, path: &str, changed: &mut Vec<String>) {
    if before == after {
        return;
    }

    match (before, after) {
        (JsonValue::Object(before_obj), JsonValue::Object(after_obj)) => {
            let keys: BTreeSet<&str> = before_obj
                .keys()
                .map(String::as_str)
                .chain(after_obj.keys().map(String::as_str))
                .collect();

            for key in keys {
                let next_path = if path.is_empty() {
                    key.to_owned()
                } else {
                    format!("{path}.{key}")
                };
                match (before_obj.get(key), after_obj.get(key)) {
                    (Some(before_value), Some(after_value)) => {
                        diff_values(before_value, after_value, &next_path, changed)
                    }
                    _ => changed.push(next_path),
                }
            }
        }
        (JsonValue::Array(before_arr), JsonValue::Array(after_arr)) => {
            if before_arr.len() != after_arr.len() {
                changed.push(path_or_root(path));
                return;
            }

            for (index, (before_value, after_value)) in
                before_arr.iter().zip(after_arr.iter()).enumerate()
            {
                let next_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                diff_values(before_value, after_value, &next_path, changed);
            }
        }
        _ => changed.push(path_or_root(path)),
    }
}

fn path_or_root(path: &str) -> String {
    if path.is_empty() {
        "$".to_owned()
    } else {
        path.to_owned()
    }
}

fn prepare_patch<T, E>(
    path: &Path,
    patch: &JsonValue,
    validate: impl Fn(&T) -> Result<(), E>,
) -> Result<PreparedPatch<T>, ErrorResponse>
where
    T: Default + serde::de::DeserializeOwned + serde::Serialize,
    E: std::fmt::Display,
{
    if !patch.is_object() {
        return Err(invalid_request("patch payload must be a JSON object"));
    }

    let (existing_toml, file_exists) = if path.exists() {
        let content = fs::read_to_string(path).map_err(|error| {
            config_write_failed(format!(
                "failed to read config `{}`: {error}",
                path.display()
            ))
        })?;
        (content, true)
    } else {
        let default_toml = toml::to_string_pretty(&T::default()).map_err(|error| {
            config_write_failed(format!(
                "failed to render default config for patching: {error}"
            ))
        })?;
        (default_toml, false)
    };

    let before_typed: T = toml::from_str(&existing_toml).map_err(|error| {
        config_write_failed(format!(
            "failed to parse existing config `{}` before patch: {error}",
            path.display()
        ))
    })?;
    let before_json = serde_json::to_value(&before_typed).map_err(|error| {
        config_write_failed(format!(
            "failed to convert existing config `{}` to json: {error}",
            path.display()
        ))
    })?;

    let mut document = existing_toml.parse::<DocumentMut>().map_err(|error| {
        config_write_failed(format!(
            "failed to parse config document `{}`: {error}",
            path.display()
        ))
    })?;

    apply_json_merge_patch(&mut document, patch, SECRET_SENTINEL).map_err(invalid_request)?;

    let rendered_toml = document.to_string();
    let typed_config: T = toml::from_str(&rendered_toml).map_err(|error| {
        config_validation_failed(format!(
            "patched config for `{}` is not valid TOML for this schema: {error}",
            path.display()
        ))
    })?;
    validate(&typed_config).map_err(|error| {
        config_validation_failed(format!(
            "config validation failed for `{}`: {error}",
            path.display()
        ))
    })?;

    let after_json = serde_json::to_value(&typed_config).map_err(|error| {
        config_write_failed(format!(
            "failed to convert patched config `{}` to json: {error}",
            path.display()
        ))
    })?;
    let changed_fields = compute_changed_fields(&before_json, &after_json);

    Ok(PreparedPatch {
        rendered_toml,
        _typed_config: std::marker::PhantomData,
        changed_fields,
        file_exists,
    })
}

fn validate_config_patch<T, E>(
    path: &Path,
    payload: Result<Json<JsonValue>, JsonRejection>,
    validate: impl Fn(&T) -> Result<(), E>,
) -> Result<super::response::ApiResponse<ValidateConfigResponse>, ErrorResponse>
where
    T: Default + serde::de::DeserializeOwned + serde::Serialize,
    E: std::fmt::Display,
{
    let patch = parse_json_payload(payload)?;
    let prepared = prepare_patch::<T, E>(path, &patch, validate)?;
    Ok(ok_response(ValidateConfigResponse {
        valid: true,
        changed_fields: prepared.changed_fields,
    }))
}

fn persist_patch<T>(
    path: &Path,
    prepared: &PreparedPatch<T>,
) -> Result<Option<PathBuf>, ErrorResponse> {
    if prepared.changed_fields.is_empty() {
        return Ok(None);
    }

    let backup = if prepared.file_exists {
        Some(backup_file(path).map_err(|error| {
            config_write_failed(format!(
                "failed to create backup for `{}`: {error}",
                path.display()
            ))
        })?)
    } else {
        None
    };

    if let Err(error) = atomic_write(path, &prepared.rendered_toml) {
        if let Some(ref backup_path) = backup {
            let _ = fs::copy(backup_path, path);
        }
        return Err(config_write_failed(format!(
            "failed to write config `{}`: {error}",
            path.display()
        )));
    }

    if backup.is_some() {
        prune_backups(path, BACKUP_KEEP_COUNT).map_err(|error| {
            config_write_failed(format!(
                "failed to prune backups for `{}`: {error}",
                path.display()
            ))
        })?;
    }

    Ok(backup)
}

fn parse_json_payload(
    payload: Result<Json<JsonValue>, JsonRejection>,
) -> Result<JsonValue, ErrorResponse> {
    payload
        .map(|Json(value)| value)
        .map_err(|error| invalid_request(format!("invalid JSON payload: {error}")))
}

fn validate_user_id(user_id: &str) -> Result<&str, String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err(format!(
            "invalid user_id `{trimmed}`: value must not be empty and cannot contain `/`, `\\`, or `..`"
        ));
    }
    Ok(trimmed)
}

async fn any_registered_daemon_running(state: &WebState) -> bool {
    let global_config = state.latest_global_config_or_cached();
    for user_id in global_config.users.keys() {
        if user_daemon_running(state, user_id).await {
            return true;
        }
    }
    false
}

async fn user_daemon_running(state: &WebState, user_id: &str) -> bool {
    let socket_path = state.control_socket_path(user_id);
    if !socket_path.exists() {
        return false;
    }
    matches!(
        send_control_to_daemon_async(&socket_path, &RunnerControl::HealthCheck).await,
        Ok(RunnerControlResponse::HealthStatus(_))
    )
}

fn invalid_request(message: impl Into<String>) -> ErrorResponse {
    ApiError::with_status(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn not_found(message: impl Into<String>) -> ErrorResponse {
    ApiError::with_status(StatusCode::NOT_FOUND, "not_found", message)
}

fn config_validation_failed(message: impl Into<String>) -> ErrorResponse {
    ApiError::with_status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "config_validation_failed",
        message,
    )
}

fn config_write_failed(message: impl Into<String>) -> ErrorResponse {
    ApiError::with_status(
        StatusCode::INTERNAL_SERVER_ERROR,
        "config_write_failed",
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    fn with_api_headers(builder: axum::http::request::Builder) -> axum::http::request::Builder {
        builder
            .header("host", "127.0.0.1:9400")
            .header("content-type", "application/json")
    }

    #[test]
    fn merge_patch_preserves_comments_and_secret_sentinel() {
        let mut document = r#"
# keep this comment
workspace_root = "old"

[web]
auth_token = "super-secret"
"#
        .parse::<DocumentMut>()
        .unwrap();

        let patch = json!({
            "workspace_root": "new",
            "web": {
                "auth_token": SECRET_SENTINEL
            }
        });

        apply_json_merge_patch(&mut document, &patch, SECRET_SENTINEL).unwrap();
        let rendered = document.to_string();

        assert!(rendered.contains("# keep this comment"));
        assert!(rendered.contains("workspace_root = \"new\""));
        assert!(rendered.contains("auth_token = \"super-secret\""));
    }

    #[test]
    fn changed_fields_include_nested_paths() {
        let before = json!({
            "runtime": {
                "max_turns": 8,
                "turn_timeout_secs": 60
            }
        });
        let after = json!({
            "runtime": {
                "max_turns": 12,
                "turn_timeout_secs": 60,
                "max_cost": 5.0
            }
        });

        let changed = compute_changed_fields(&before, &after);
        assert!(changed.contains(&"runtime.max_turns".to_owned()));
        assert!(changed.contains(&"runtime.max_cost".to_owned()));
        assert!(!changed.contains(&"runtime.turn_timeout_secs".to_owned()));
    }

    #[tokio::test]
    async fn patch_runner_config_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("runner.toml");
        let state = Arc::new(WebState::new(
            RunnerGlobalConfig::default(),
            config_path.clone(),
            "127.0.0.1:9400".to_owned(),
        ));
        let app = crate::web::build_router(state);

        let request = with_api_headers(
            Request::builder()
                .method(Method::PATCH)
                .uri("/api/v1/config/runner"),
        )
        .body(Body::from(
            json!({ "workspace_root": "custom/workspaces" }).to_string(),
        ))
        .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let written = fs::read_to_string(&config_path).unwrap();
        assert!(written.contains("workspace_root = \"custom/workspaces\""));
    }

    #[tokio::test]
    async fn validate_agent_config_rejects_invalid_runtime_limits() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("runner.toml");
        fs::write(
            &config_path,
            toml::to_string_pretty(&RunnerGlobalConfig::default()).unwrap(),
        )
        .unwrap();
        let state = Arc::new(WebState::new(
            RunnerGlobalConfig::default(),
            config_path,
            "127.0.0.1:9400".to_owned(),
        ));
        let app = crate::web::build_router(state);

        let request = with_api_headers(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/config/agent/validate"),
        )
        .body(Body::from(
            json!({ "runtime": { "max_turns": 0 } }).to_string(),
        ))
        .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_and_delete_user_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("runner.toml");
        fs::write(
            &config_path,
            toml::to_string_pretty(&RunnerGlobalConfig::default()).unwrap(),
        )
        .unwrap();
        let state = Arc::new(WebState::new(
            RunnerGlobalConfig::default(),
            config_path.clone(),
            "127.0.0.1:9400".to_owned(),
        ));
        let app = crate::web::build_router(state);

        let create_request = with_api_headers(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/config/users"),
        )
        .body(Body::from(
            json!({ "user_id": "alice", "config_path": "users/alice.toml" }).to_string(),
        ))
        .unwrap();
        let create_response = app.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::OK);

        let user_config_path = dir.path().join("users/alice.toml");
        assert!(user_config_path.exists());

        let delete_request = with_api_headers(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/v1/config/users/alice?delete_config_file=true"),
        )
        .body(Body::from("{}"))
        .unwrap();
        let delete_response = app.oneshot(delete_request).await.unwrap();
        assert_eq!(delete_response.status(), StatusCode::OK);
        assert!(!user_config_path.exists());
    }
}
