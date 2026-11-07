//! Handler REST cho màn Cài đặt thống nhất (M29 mục 8.1): bảng thiết lập, secret
//! slot và kiểm tra kết nối.
//!
//! # Ranh giới bảo mật
//!
//! * `GET /api/settings` + `GET /api/secrets`: **mọi user đã đăng nhập** — chỉ đọc,
//!   và payload không có giá trị secret (chỉ cờ `set`/`source`, tên biến đã công khai
//!   trong `bean.toml`).
//! * `PATCH /api/settings`, `POST /api/settings/test/{target}`, `PUT/DELETE
//!   /api/secrets/{name}`: **chỉ admin** khi RBAC bật (mẫu `set_tool_enabled`) —
//!   sửa cấu hình và đốt key là thay đổi bề mặt vận hành.
//! * Ghi/xoá secret yêu cầu **step-up mật khẩu** (D19.5) qua
//!   [`crate::auth::AuthService::verify_step_up`] — dùng chung rate-limiter với
//!   `login`, session bị đánh cắp không đủ để đọc/xoá key.
//! * Ghi thiết lập bắt buộc audit (bất biến của `SettingsService`): không có audit
//!   log thì trả `503 audit_unavailable` — không bao giờ ghi âm thầm. Audit **không
//!   bao giờ** chứa giá trị (chỉ tên khoá + nguồn cũ→mới).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Json;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_security::{AuditLog, entry_now};
use bean_settings::{
    ProbeOutcome, SecretSlot, SecretSlotRegistry, SettingApply, SettingKind, SettingSource,
    SettingsError,
};
use bean_types::config::SpeechProviderKind;
use secrecy::SecretString;

use crate::api_types::{
    SecretDeleteRequest, SecretMutateResponse, SecretPutRequest, SecretSlotDto,
    SecretsListResponse, SettingDto, SettingsListResponse, SettingsPatchRequest,
    SettingsPatchResponse, SettingsTestResponse,
};
use crate::auth::AuthError;

use super::rest_voice::effective_voice_config;
use super::{ApiFailure, ApiResult, WebState, require_user};

// ────────────────── GET /api/settings — bảng thiết lập (chỉ đọc) ──────────────────

/// `GET /api/settings` — toàn bộ bảng thiết lập với giá trị hiệu lực + nguồn.
///
/// Đọc qua `SettingsService` (tự load `bean.toml` + state files từ đĩa) nên luôn
/// khớp với những gì CLI `bean config get` thấy — UI không tự giữ bản sao schema.
pub(super) async fn settings_list(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SettingsListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let service = bean_settings::SettingsService::new(&state.config, state.router.config_path());
    let settings = bean_settings::SETTINGS
        .iter()
        .map(|entry| {
            let read = service.get(entry);
            SettingDto {
                key: entry.path.to_string(),
                value: read.value,
                source: source_str(read.source).to_string(),
                tier: entry.tier.short().to_string(),
                apply: entry.apply.label().to_string(),
                kind: kind_str(entry.kind).to_string(),
                editable_in_ui: !entry.tier.cli_only(),
                desc_vi: entry.desc_vi.to_string(),
                desc_en: entry.desc_en.to_string(),
            }
        })
        .collect();
    Ok(Json(SettingsListResponse {
        settings,
        writable: state.router.config_path().is_some(),
    }))
}

/// `PATCH /api/settings` — ghi một khoá tầng A; tầng B trả `403 cli_only` kèm đúng
/// lệnh CLI thay thế. Giá trị là chuỗi, parse bằng `parse_value` theo kiểu của khoá.
pub(super) async fn settings_patch(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<SettingsPatchRequest>,
) -> ApiResult<Json<SettingsPatchResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let Some(entry) = bean_settings::lookup(&request.key) else {
        return Err(unknown_setting_failure(&request.key));
    };
    if entry.tier.cli_only() {
        return Err(ApiFailure::new(
            StatusCode::FORBIDDEN,
            "cli_only",
            format!(
                "`{}` là khoá tầng B (bảo mật) — chỉ sửa được qua CLI tương tác: \
                 `bean config set {} <giá trị>`",
                entry.path, entry.path
            ),
        ));
    }
    let Some(config_path) = state.router.config_path() else {
        return Err(config_readonly());
    };
    let value = bean_settings::parse_value(entry, &request.value).map_err(|error| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_value",
            error.to_string(),
        )
    })?;
    let audit = required_audit(&state)?;
    let service = bean_settings::SettingsService::new(&state.config, Some(config_path));
    let outcome = service
        .set(entry, value, &audit, &user_id)
        .map_err(map_settings_error)?;
    Ok(Json(SettingsPatchResponse {
        key: entry.path.to_string(),
        apply: entry.apply.label().to_string(),
        source: source_str(outcome.new_source).to_string(),
        restart_required: entry.apply == SettingApply::Restart,
    }))
}

// ────────────────── GET/PUT/DELETE /api/secrets — slot secret ──────────────────

/// `GET /api/secrets` — slot dẫn xuất từ cấu hình, **không có giá trị**; nguồn theo
/// thứ tự D19.3 (env thắng `secrets.env`), kèm cờ bị che cho bẫy K28.
pub(super) async fn secrets_list(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SecretsListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let registry = SecretSlotRegistry::from_config(&state.config);
    let snapshot = state.router.secret_snapshot();
    let slots = registry
        .slots()
        .iter()
        .map(|slot| slot_dto(slot, snapshot.as_ref()))
        .collect();
    Ok(Json(SecretsListResponse { slots }))
}

/// `PUT /api/secrets/{name}` — lưu giá trị vào `secrets.env` sau step-up.
pub(super) async fn secret_put(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
    Path(name): Path<String>,
    Json(request): Json<SecretPutRequest>,
) -> ApiResult<Json<SecretMutateResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    step_up(&state, &request.password, peer_ip(&peer))?;
    let (registry, name) = resolve_slot(&state, &name)?;
    let store = secret_store_of(&state)?;
    let audit = required_audit(&state)?;
    store
        .set(&registry, &name, SecretString::from(request.value))
        .map_err(map_settings_error)?;
    record_secret_audit(&audit, &user_id, "secret.set", &name);
    Ok(Json(SecretMutateResponse {
        ok: true,
        name,
        source: "secrets_env".to_string(),
    }))
}

/// `DELETE /api/secrets/{name}` — xoá giá trị khỏi `secrets.env` sau step-up.
pub(super) async fn secret_delete(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
    Path(name): Path<String>,
    Json(request): Json<SecretDeleteRequest>,
) -> ApiResult<Json<SecretMutateResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    step_up(&state, &request.password, peer_ip(&peer))?;
    let (registry, name) = resolve_slot(&state, &name)?;
    let store = secret_store_of(&state)?;
    let audit = required_audit(&state)?;
    let existed = store.remove(&registry, &name).map_err(map_settings_error)?;
    record_secret_audit(&audit, &user_id, "secret.delete", &name);
    Ok(Json(SecretMutateResponse {
        ok: true,
        name,
        source: if existed { "secrets_env" } else { "unset" }.to_string(),
    }))
}

// ────────────────── POST /api/settings/test/{target} — probe ──────────────────

/// `POST /api/settings/test/{target}` — kiểm tra kết nối **trước** khi bật, dùng
/// chung probe với `bean doctor --online` nên kết luận luôn khớp. Không gate cờ
/// `enabled`: ý chính của nút này là test cấu hình còn chưa bật.
pub(super) async fn settings_test(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(target): Path<String>,
) -> ApiResult<Json<SettingsTestResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let target = target.trim().to_string();
    let outcome = match target.as_str() {
        "llm" => test_llm(&state).await,
        "voice" => test_voice(&state).await,
        "telegram" => test_telegram(&state).await,
        "search" => test_search(&state).await,
        _ => {
            return Err(ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown_target",
                "target chỉ nhận `llm` | `voice` | `telegram` | `search`",
            ));
        }
    };
    Ok(Json(SettingsTestResponse {
        target,
        level: outcome.level.as_str().to_string(),
        message: outcome.message,
        hint: outcome.hint,
    }))
}

/// Probe LLM: dựng lại provider từ snapshot cấu hình runtime (cùng nguồn `/model`)
/// với key resolve theo D19.3, rồi ping `max_tokens = 1`.
async fn test_llm(state: &WebState) -> ProbeOutcome {
    let runtime = match state.router.llm_config() {
        Ok(runtime) => runtime,
        Err(error) => {
            return ProbeOutcome::error(format!("không đọc được cấu hình LLM runtime: {error}"));
        }
    };
    // Clone seed rồi áp snapshot runtime — `resolve_secrets_with` chỉ đọc `[llm]`.
    let mut candidate = state.config.clone();
    candidate.llm.provider = runtime.provider;
    candidate.llm.model = runtime.model;
    candidate.llm.allowed_models = runtime.allowed_models;
    candidate.llm.api_key_env = runtime.api_key_env;
    candidate.llm.base_url = runtime.base_url;
    candidate.llm.max_tokens = runtime.max_tokens;
    let snapshot = state.router.secret_snapshot();
    let lookup = bean_settings::env_first_lookup(snapshot.as_ref());
    let secrets = match candidate.resolve_secrets_with(lookup) {
        Ok(secrets) => secrets,
        Err(error) => {
            return ProbeOutcome::error(format!("không resolve được secret LLM: {error}"))
                .with_hint(format!("`bean secret set {}`", candidate.llm.api_key_env));
        }
    };
    let provider = match bean_llm::build_provider(&candidate.llm, secrets.llm_api_key) {
        Ok(provider) => provider,
        Err(error) => return ProbeOutcome::error(format!("không dựng được provider LLM: {error}")),
    };
    bean_settings::probe_llm(
        provider.as_ref(),
        candidate.llm.provider.as_str(),
        &candidate.llm.model,
    )
    .await
}

/// Probe ElevenLabs trên đúng cấu hình hiệu lực (`voice-state.json` thắng seed) —
/// provider `local` không có gì để kiểm qua mạng nên trả `warn` kèm hướng dẫn.
async fn test_voice(state: &WebState) -> ProbeOutcome {
    let effective = effective_voice_config(state);
    if effective.provider != SpeechProviderKind::Api {
        return ProbeOutcome::warn(
            "voice: provider `local` không kiểm tra được qua HTTP — dùng `bean doctor` \
             để kiểm tra binary/model whisper/piper",
        );
    }
    // `resolve_voice_api_key_with` là method của `Config` — dựng bản seed với đúng
    // phần `[voice]` hiệu lực (state file thắng seed) rồi resolve theo D19.3.
    let mut candidate = state.config.clone();
    candidate.voice = effective;
    let snapshot = state.router.secret_snapshot();
    let lookup = bean_settings::env_first_lookup(snapshot.as_ref());
    let env_name = candidate.voice.api_key_env.clone();
    match candidate.resolve_voice_api_key_with(&lookup) {
        Ok(Some(key)) => bean_settings::probe_voice_api(&candidate.voice.api_base_url, &key).await,
        Ok(None) => ProbeOutcome::error(format!("voice api: chưa có key ({env_name})"))
            .with_hint(format!("`bean secret set {env_name}`")),
        Err(error) => ProbeOutcome::error(format!("voice api: đọc key thất bại: {error}"))
            .with_hint(format!("`bean secret set {env_name}`")),
    }
}

/// Probe Telegram `getMe` — token resolve theo D19.3, mọi lỗi đã redact URL.
async fn test_telegram(state: &WebState) -> ProbeOutcome {
    let snapshot = state.router.secret_snapshot();
    let lookup = bean_settings::env_first_lookup(snapshot.as_ref());
    match state.config.resolve_telegram_token_with(&lookup) {
        Ok(Some(token)) => bean_settings::probe_telegram(&token).await,
        Ok(None) => ProbeOutcome::error(format!(
            "telegram: chưa có token ({})",
            state.config.telegram.token_env
        ))
        .with_hint(format!(
            "`bean secret set {}`",
            state.config.telegram.token_env
        )),
        Err(error) => ProbeOutcome::error(format!("telegram: đọc token thất bại: {error}")),
    }
}

/// Probe web search; **không** gate `tools.enabled` — test trước khi bật là ý chính
/// nên key resolve thủ công thay vì `resolve_web_search_api_key_with` (hàm đó trả
/// `None` khi nhóm `web` tắt).
async fn test_search(state: &WebState) -> ProbeOutcome {
    let search = &state.config.tools.web_search;
    let key = if search.provider.requires_api_key() {
        let snapshot = state.router.secret_snapshot();
        let lookup = bean_settings::env_first_lookup(snapshot.as_ref());
        match lookup(&search.api_key_env) {
            Some(value) => Some(SecretString::from(value)),
            None => {
                return ProbeOutcome::error(format!(
                    "web_search: chưa có key ({})",
                    search.api_key_env
                ))
                .with_hint(format!("`bean secret set {}`", search.api_key_env));
            }
        }
    } else {
        None
    };
    bean_settings::probe_search(search.provider, search.base_url.as_deref(), key.as_ref()).await
}

// ────────────────── Helpers dùng chung ──────────────────

/// Gate admin cho mọi mutation — cùng quy tắc với toggle tool (mẫu `rest_audit_tools`).
pub(super) fn require_admin(state: &WebState, user_id: &str) -> Result<(), ApiFailure> {
    let permissions = state.config.permissions_for(user_id);
    if state.config.rbac_enabled() && permissions.role != "admin" {
        return Err(ApiFailure::new(
            StatusCode::FORBIDDEN,
            "admin_required",
            "chỉ admin được sửa cài đặt",
        ));
    }
    Ok(())
}

/// IP thật từ `ConnectInfo` (không tin header `X-Forwarded-For`) — fallback loopback
/// cho test `oneshot`, cùng mẫu với `login`.
pub(super) fn peer_ip(peer: &Option<Extension<ConnectInfo<SocketAddr>>>) -> IpAddr {
    peer.as_ref().map_or(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        |Extension(ConnectInfo(peer))| peer.ip(),
    )
}

/// Step-up mật khẩu; lỗi ánh xạ thẳng sang HTTP, chi tiết lỗi auth giữ kín.
pub(super) fn step_up(state: &WebState, password: &str, ip: IpAddr) -> Result<(), ApiFailure> {
    state
        .auth
        .verify_step_up(password, ip, state.audit.as_deref())
        .map_err(|error| match error {
            AuthError::InvalidCredentials => ApiFailure::new(
                StatusCode::UNAUTHORIZED,
                "invalid_password",
                "mật khẩu không đúng",
            ),
            AuthError::RateLimited(seconds) => ApiFailure::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("quá nhiều lần thử; thử lại sau {seconds} giây"),
            ),
            _ => ApiFailure::internal(),
        })
}

/// Slot phải được khai báo trong cấu hình hiện tại — `secrets.env` không thể trở
/// thành đường chèn biến môi trường tuỳ ý (`PATH`, `LD_PRELOAD`…).
fn resolve_slot(state: &WebState, name: &str) -> Result<(SecretSlotRegistry, String), ApiFailure> {
    let name = name.trim().to_string();
    let registry = SecretSlotRegistry::from_config(&state.config);
    if registry.slot(&name).is_none() {
        return Err(ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_slot",
            format!("`{name}` không phải secret slot đã khai báo — xem `bean secret list`"),
        ));
    }
    Ok((registry, name))
}

fn secret_store_of(state: &WebState) -> Result<Arc<bean_settings::SecretStore>, ApiFailure> {
    state.router.secret_store().cloned().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "secret_unavailable",
            "tiến trình không có kho secret — cấu hình key qua biến môi trường",
        )
    })
}

/// Audit log **bắt buộc** cho mutation: dùng log của state, hoặc tự mở từ đĩa
/// (`data.dir/audit` — cùng vị trí `serve` đã mở); không có gì cả ⇒ 503, không ghi
/// âm thầm (M29 mục 4.3).
pub(super) fn required_audit(state: &WebState) -> Result<Arc<AuditLog>, ApiFailure> {
    if let Some(audit) = state.audit.as_ref() {
        return Ok(Arc::clone(audit));
    }
    let dir = state.config.data.dir.join("audit");
    AuditLog::open(&dir).map(Arc::new).map_err(|error| {
        tracing::warn!(error = %error, "không mở được audit log — từ chối mutation cài đặt");
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "audit_unavailable",
            "không mở được audit log — từ chối ghi (audit là bắt buộc)",
        )
    })
}

/// Audit cho ghi/xoá secret: chỉ **tên slot**, không bao giờ giá trị (mục 15.8).
/// Caller đã mở được log qua [`required_audit`] — ở đây chỉ ghi, lỗi log không che
/// mất kết quả đã thành công của thao tác chính.
fn record_secret_audit(audit: &AuditLog, user_id: &str, event: &'static str, name: &str) {
    let mut entry = entry_now(0, "web", event, &serde_json::json!({ "slot": name }));
    entry.ok = Some(true);
    entry.decision = "allow";
    entry.decided_by = user_id.to_string();
    if let Err(error) = audit.record(&entry) {
        tracing::warn!(error = %error, "không ghi được audit secret");
    }
}

/// Từ chối khoá lạ với đúng gợi ý của CLI `bean config set` (mẫu `config_cmd`).
pub(super) fn unknown_setting_failure(key: &str) -> ApiFailure {
    let looks_like_env_name = !key.contains('.')
        && key.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    let message = if looks_like_env_name {
        format!(
            "`{key}` không phải khoá thiết lập — trông giống tên biến môi trường: dùng \
             `bean secret set {key}` để lưu giá trị (xem `bean secret list`)"
        )
    } else {
        format!(
            "`{key}` không phải khoá thiết lập — xem `bean config list`. Cấu trúc TOML phức tạp \
             ([[roles]], [[infra_scope]], [[mcp_clients]], agent.user_roles…) không có trong bảng: \
             sửa trực tiếp bean.toml rồi restart"
        )
    };
    ApiFailure::new(StatusCode::UNPROCESSABLE_ENTITY, "unknown_setting", message)
}

/// Lỗi `SettingsError` → HTTP: sai tham số/cấu hình bị từ chối là 422 kèm lý do;
/// lỗi đĩa/audit là 500 (thông điệp chỉ vào log, không lộ đường dẫn hệ thống).
pub(super) fn map_settings_error(error: SettingsError) -> ApiFailure {
    match &error {
        SettingsError::UnknownSetting { .. }
        | SettingsError::InvalidSetting { .. }
        | SettingsError::UnknownSlot { .. }
        | SettingsError::InvalidValue { .. }
        | SettingsError::ConfigRejected { .. } => ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "settings_rejected",
            error.to_string(),
        ),
        _ => {
            tracing::warn!(error = %error, "ghi thiết lập/secret thất bại");
            ApiFailure::internal()
        }
    }
}

pub(super) fn config_readonly() -> ApiFailure {
    ApiFailure::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "config_readonly",
        "server không biết đường dẫn bean.toml — không thể ghi thiết lập",
    )
}

/// DTO một slot với nguồn theo D19.3 — env thắng file, cả hai cùng có ⇒ `shadowed_by_env`.
fn slot_dto(slot: &SecretSlot, snapshot: Option<&bean_settings::SecretSnapshot>) -> SecretSlotDto {
    let env_set = std::env::var(&slot.name).is_ok_and(|value| !value.trim().is_empty());
    let file_set = snapshot.is_some_and(|snapshot| snapshot.contains(&slot.name));
    SecretSlotDto {
        name: slot.name.clone(),
        label: slot.used_by.join(", "),
        group: slot_group_str(slot.group).to_string(),
        set: env_set || file_set,
        source: if env_set {
            "env"
        } else if file_set {
            "secrets_env"
        } else {
            "unset"
        }
        .to_string(),
        shadowed_by_env: env_set && file_set,
    }
}

const fn slot_group_str(group: bean_settings::SlotGroup) -> &'static str {
    match group {
        bean_settings::SlotGroup::Llm => "llm",
        bean_settings::SlotGroup::Voice => "voice",
        bean_settings::SlotGroup::Telegram => "telegram",
        bean_settings::SlotGroup::Search => "search",
        bean_settings::SlotGroup::Billing => "billing",
        bean_settings::SlotGroup::Marketing => "marketing",
    }
}

/// Chuỗi máy cho nguồn giá trị — khác `label()` hiển thị ("mặc định" → `default`).
const fn source_str(source: SettingSource) -> &'static str {
    match source {
        SettingSource::Default => "default",
        SettingSource::BeanToml => "bean.toml",
        SettingSource::SettingsState => "settings-state.json",
    }
}

const fn kind_str(kind: SettingKind) -> &'static str {
    match kind {
        SettingKind::Bool => "bool",
        SettingKind::U32 => "u32",
        SettingKind::U64 => "u64",
        SettingKind::F64 => "f64",
        SettingKind::Str => "str",
        SettingKind::OptStr => "opt_str",
        SettingKind::StrList => "str_list",
        SettingKind::I64List => "i64_list",
    }
}
