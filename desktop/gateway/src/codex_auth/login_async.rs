use std::collections::HashSet;
use std::fmt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::{Zeroize, Zeroizing};

use super::oauth::{
    declared_response_kind, parse_new_oauth_tokens, response_kind, OAuthErrorCode, OAuthFlowError,
    CODEX_OAUTH_CLIENT_ID, CODEX_OAUTH_ISSUER, CODEX_OAUTH_ORIGINATOR, CODEX_OAUTH_SCOPE,
};
use super::storage::{AuthRepository, AuthStatus, SecretStore, StateStore};
use crate::codex_network::CodexHttpClientFactory;

const CALLBACK_PATH: &str = "/auth/callback";
const CALLBACK_PORTS: &[u16] = &[1455, 1457];
const BROWSER_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CALLBACK_IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CALLBACK_HEAD: usize = 64 * 1024;
const MAX_CALLBACK_REQUESTS: usize = 64;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const CONTROL_RUNNING: u8 = 0;
const CONTROL_CANCELLED: u8 = 1;
const CONTROL_COMMITTING: u8 = 2;
const CONTROL_FINISHED: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncLoginMethod {
    Device,
    Browser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelDisposition {
    Accepted,
    CommitInProgress,
    AlreadyTerminal,
}

impl CancelDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::CommitInProgress => "commit_in_progress",
            Self::AlreadyTerminal => "already_terminal",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoginProgress {
    VerificationRequired {
        verification_url: String,
        user_code: String,
        expires_at_ms: i64,
    },
    Waiting,
    Exchanging,
    Committing,
}

#[derive(Clone, Default)]
pub struct LoginControl {
    state: Arc<AtomicU8>,
}

impl LoginControl {
    pub fn cancel(&self) -> CancelDisposition {
        loop {
            match self.state.load(Ordering::Acquire) {
                CONTROL_RUNNING => {
                    if self
                        .state
                        .compare_exchange(
                            CONTROL_RUNNING,
                            CONTROL_CANCELLED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return CancelDisposition::Accepted;
                    }
                }
                CONTROL_CANCELLED => return CancelDisposition::Accepted,
                CONTROL_COMMITTING => return CancelDisposition::CommitInProgress,
                _ => return CancelDisposition::AlreadyTerminal,
            }
        }
    }

    fn begin_commit(&self) -> Result<(), OAuthFlowError> {
        self.state
            .compare_exchange(
                CONTROL_RUNNING,
                CONTROL_COMMITTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| cancelled_error("cancelled"))
    }

    fn finish(&self) {
        self.state.store(CONTROL_FINISHED, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == CONTROL_CANCELLED
    }

    async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[derive(Clone)]
struct AsyncLoginOptions {
    issuer: String,
    client_id: String,
    callback_ports: Vec<u16>,
    browser_timeout: Duration,
    device_timeout: Duration,
}

impl AsyncLoginOptions {
    fn production() -> Self {
        Self {
            issuer: CODEX_OAUTH_ISSUER.to_string(),
            client_id: CODEX_OAUTH_CLIENT_ID.to_string(),
            callback_ports: CALLBACK_PORTS.to_vec(),
            browser_timeout: BROWSER_TIMEOUT,
            device_timeout: DEVICE_TIMEOUT,
        }
    }
}

pub async fn run_production_login<S, T, F>(
    repository: &AuthRepository<S, T>,
    method: AsyncLoginMethod,
    control: &LoginControl,
    progress: F,
) -> Result<AuthStatus, OAuthFlowError>
where
    S: SecretStore,
    T: StateStore,
    F: Fn(LoginProgress),
{
    let factory = CodexHttpClientFactory::from_environment().map_err(|_| {
        OAuthFlowError::new(
            OAuthErrorCode::OAuthNetwork,
            false,
            "The Codex network route is invalid",
        )
        .at_stage("proxy_config")
    })?;
    let client = factory
        .async_builder()
        .map_err(|_| {
            OAuthFlowError::new(
                OAuthErrorCode::OAuthNetwork,
                false,
                "The Codex network route is invalid",
            )
            .at_stage("proxy_config")
        })?
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|_| {
            OAuthFlowError::new(
                OAuthErrorCode::OAuthNetwork,
                true,
                "The OAuth network client could not be created",
            )
            .at_stage("proxy_config")
        })?;
    let options = AsyncLoginOptions::production();
    let result = match method {
        AsyncLoginMethod::Device => {
            run_device_login(
                repository,
                &client,
                factory.has_proxy(),
                &options,
                control,
                &progress,
            )
            .await
        }
        AsyncLoginMethod::Browser => {
            run_browser_login(
                repository,
                &client,
                factory.has_proxy(),
                &options,
                control,
                &progress,
            )
            .await
        }
    };
    control.finish();
    result
}

#[derive(Serialize)]
struct UserCodeRequest<'a> {
    client_id: &'a str,
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default, deserialize_with = "deserialize_interval")]
    interval: u64,
}

#[derive(Serialize)]
struct TokenPollRequest<'a> {
    device_auth_id: &'a str,
    user_code: &'a str,
}

#[derive(Deserialize)]
struct CodeSuccessResponse {
    authorization_code: String,
    code_challenge: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct DeviceErrorResponse {
    #[serde(default)]
    error: String,
}

fn deserialize_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Number(u64),
        String(String),
    }
    match Value::deserialize(deserializer)? {
        Value::Number(value) => Ok(value),
        Value::String(value) => value.trim().parse().map_err(serde::de::Error::custom),
    }
}

async fn run_device_login<S, T, F>(
    repository: &AuthRepository<S, T>,
    client: &reqwest::Client,
    has_proxy: bool,
    options: &AsyncLoginOptions,
    control: &LoginControl,
    progress: &F,
) -> Result<AuthStatus, OAuthFlowError>
where
    S: SecretStore,
    T: StateStore,
    F: Fn(LoginProgress),
{
    let guard = repository.begin_mutation().map_err(OAuthFlowError::from)?;
    let deadline = tokio::time::Instant::now() + options.device_timeout;
    let issuer = options.issuer.trim_end_matches('/');
    let user_code_endpoint = format!("{issuer}/api/accounts/deviceauth/usercode");
    let user_code_body = serde_json::to_vec(&UserCodeRequest {
        client_id: &options.client_id,
    })
    .map_err(|_| {
        OAuthFlowError::protocol("The device code request is invalid")
            .at_stage("device_code_request")
    })?;
    let response = send_request(
        client
            .post(user_code_endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header("user-agent", crate::config::UPSTREAM_UA)
            .body(user_code_body),
        control,
        has_proxy,
        "device_code_request",
    )
    .await?;
    let response = read_response(response, control, "device_code_request").await?;
    reject_challenge(&response, "device_code_request")?;
    if response.status == 404 && matches!(response.kind, "json" | "empty") {
        return Err(OAuthFlowError::new(
            OAuthErrorCode::DeviceAuthUnavailable,
            false,
            "Device code login is unavailable",
        )
        .at_stage("device_code_request")
        .with_http(Some(response.status), Some(response.kind), Some(false)));
    }
    require_json_success(&response, "device_code_request")?;
    let user_code: UserCodeResponse = serde_json::from_slice(&response.body).map_err(|_| {
        OAuthFlowError::protocol("The device code response is invalid")
            .at_stage("device_code_request")
            .with_http(Some(response.status), Some(response.kind), Some(false))
    })?;
    if user_code.device_auth_id.is_empty()
        || user_code.device_auth_id.len() > 1_024
        || !valid_user_code(&user_code.user_code)
    {
        return Err(
            OAuthFlowError::protocol("The device code response is invalid")
                .at_stage("device_code_request")
                .with_http(Some(response.status), Some(response.kind), Some(false)),
        );
    }
    let interval = user_code.interval.clamp(1, 30);
    let verification_url = format!("{issuer}/codex/device");
    progress(LoginProgress::VerificationRequired {
        verification_url: verification_url.clone(),
        user_code: user_code.user_code.clone(),
        expires_at_ms: now_ms().saturating_add(options.device_timeout.as_millis() as i64),
    });
    progress(LoginProgress::Waiting);

    let token_endpoint = format!("{issuer}/api/accounts/deviceauth/token");
    let code = loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(OAuthFlowError::new(
                OAuthErrorCode::CallbackTimeout,
                true,
                "Device code login timed out",
            )
            .at_stage("device_wait"));
        }
        let poll_body = serde_json::to_vec(&TokenPollRequest {
            device_auth_id: &user_code.device_auth_id,
            user_code: &user_code.user_code,
        })
        .map_err(|_| {
            OAuthFlowError::protocol("The device authorization request is invalid")
                .at_stage("device_wait")
        })?;
        let response = send_request(
            client
                .post(&token_endpoint)
                .header(CONTENT_TYPE, "application/json")
                .header("user-agent", crate::config::UPSTREAM_UA)
                .body(poll_body),
            control,
            has_proxy,
            "device_wait",
        )
        .await?;
        let response = read_response(response, control, "device_wait").await?;
        reject_challenge(&response, "device_wait")?;
        if response.status >= 200 && response.status < 300 {
            if response.kind != "json" {
                return Err(unexpected_content_type(&response, "device_wait"));
            }
            let code: CodeSuccessResponse =
                serde_json::from_slice(&response.body).map_err(|_| {
                    OAuthFlowError::protocol("The device authorization response is invalid")
                        .at_stage("device_wait")
                        .with_http(Some(response.status), Some(response.kind), Some(false))
                })?;
            validate_code_success(&code)?;
            break code;
        }
        if let Ok(error) = serde_json::from_slice::<DeviceErrorResponse>(&response.body) {
            if matches!(
                error.error.as_str(),
                "access_denied" | "authorization_declined"
            ) {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::OAuthDenied,
                    false,
                    "Device code login was denied",
                )
                .at_stage("device_wait")
                .with_http(Some(response.status), Some(response.kind), Some(false)));
            }
            if error.error == "expired_token" {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::CallbackTimeout,
                    true,
                    "Device code login expired",
                )
                .at_stage("device_wait")
                .with_http(Some(response.status), Some(response.kind), Some(false)));
            }
        }
        if !matches!(response.status, 403 | 404) || !matches!(response.kind, "json" | "empty") {
            if response.kind == "html" || response.kind == "other" {
                return Err(unexpected_content_type(&response, "device_wait"));
            }
            return Err(
                OAuthFlowError::protocol("Device authorization was rejected")
                    .at_stage("device_wait")
                    .with_http(Some(response.status), Some(response.kind), Some(false)),
            );
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        cancellable_sleep(
            Duration::from_secs(interval).min(remaining),
            control,
            "device_wait",
        )
        .await?;
    };

    progress(LoginProgress::Exchanging);
    let redirect_uri = format!("{issuer}/deviceauth/callback");
    let tokens = exchange_code(
        client,
        has_proxy,
        options,
        &redirect_uri,
        &code.authorization_code,
        &code.code_verifier,
        control,
    )
    .await?;
    commit_login(repository, &guard, tokens, control, progress)
}

async fn run_browser_login<S, T, F>(
    repository: &AuthRepository<S, T>,
    client: &reqwest::Client,
    has_proxy: bool,
    options: &AsyncLoginOptions,
    control: &LoginControl,
    progress: &F,
) -> Result<AuthStatus, OAuthFlowError>
where
    S: SecretStore,
    T: StateStore,
    F: Fn(LoginProgress),
{
    let guard = repository.begin_mutation().map_err(OAuthFlowError::from)?;
    let deadline = tokio::time::Instant::now() + options.browser_timeout;
    let (listener, port) = bind_callback(&options.callback_ports)?;
    let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
    let pkce = generate_pkce()?;
    let state = generate_state()?;
    let authorization_url = build_authorization_url(
        &options.issuer,
        &options.client_id,
        &redirect_uri,
        &pkce.challenge,
        &state,
    )?;
    open_browser(&authorization_url, control).await?;
    progress(LoginProgress::Waiting);

    let mut request_count = 0_usize;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(OAuthFlowError::new(
                OAuthErrorCode::CallbackTimeout,
                true,
                "Codex sign-in timed out",
            )
            .at_stage("callback_wait"));
        }
        let accepted = tokio::select! {
            _ = control.cancelled() => return Err(cancelled_error("callback_wait")),
            result = tokio::time::timeout(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                listener.accept(),
            ) => result,
        };
        let (mut stream, _) = match accepted {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::CallbackUnavailable,
                    true,
                    "The local OAuth callback stopped unexpectedly",
                )
                .at_stage("callback_wait"))
            }
            Err(_) => {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::CallbackTimeout,
                    true,
                    "Codex sign-in timed out",
                )
                .at_stage("callback_wait"))
            }
        };
        request_count += 1;
        if request_count > MAX_CALLBACK_REQUESTS {
            return Err(
                OAuthFlowError::protocol("Too many OAuth callbacks").at_stage("callback_wait")
            );
        }
        let action = match read_and_parse_callback(&mut stream, port, &state, control).await {
            Ok(action) => action,
            Err(error) if error.code == OAuthErrorCode::AuthCancelled => return Err(error),
            Err(_) => {
                write_callback(&mut stream, 400, "Sign-in request rejected").await;
                continue;
            }
        };
        match action {
            CallbackAction::Ignore(status) => {
                write_callback(&mut stream, status, "Sign-in request rejected").await;
            }
            CallbackAction::Denied => {
                write_callback(&mut stream, 400, "Sign-in was not completed").await;
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::OAuthDenied,
                    false,
                    "Codex sign-in was denied or cancelled",
                )
                .at_stage("callback_wait"));
            }
            CallbackAction::Code(mut code) => {
                progress(LoginProgress::Exchanging);
                let tokens = exchange_code(
                    client,
                    has_proxy,
                    options,
                    &redirect_uri,
                    &code,
                    &pkce.verifier,
                    control,
                )
                .await;
                code.zeroize();
                match tokens {
                    Ok(tokens) => {
                        let result = commit_login(repository, &guard, tokens, control, progress);
                        if result.is_ok() {
                            write_callback(
                                &mut stream,
                                200,
                                "Codex sign-in completed. You can close this window.",
                            )
                            .await;
                        } else {
                            write_callback(
                                &mut stream,
                                500,
                                "Codex sign-in could not be completed safely.",
                            )
                            .await;
                        }
                        return result;
                    }
                    Err(error) => {
                        write_callback(
                            &mut stream,
                            500,
                            "Codex sign-in could not be completed safely.",
                        )
                        .await;
                        return Err(error);
                    }
                }
            }
        }
    }
}

fn commit_login<S, T, F>(
    repository: &AuthRepository<S, T>,
    guard: &super::storage::AuthMutationGuard,
    tokens: super::storage::NewOAuthTokens,
    control: &LoginControl,
    progress: &F,
) -> Result<AuthStatus, OAuthFlowError>
where
    S: SecretStore,
    T: StateStore,
    F: Fn(LoginProgress),
{
    control.begin_commit()?;
    progress(LoginProgress::Committing);
    repository
        .commit_login_guarded(guard, tokens)
        .map_err(|error| OAuthFlowError::from(error).at_stage("keychain_commit"))
}

async fn exchange_code(
    client: &reqwest::Client,
    has_proxy: bool,
    options: &AsyncLoginOptions,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
    control: &LoginControl,
) -> Result<super::storage::NewOAuthTokens, OAuthFlowError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("client_id", &options.client_id)
        .append_pair("code_verifier", verifier)
        .finish();
    let endpoint = format!("{}/oauth/token", options.issuer.trim_end_matches('/'));
    let response = send_request(
        client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("user-agent", crate::config::UPSTREAM_UA)
            .body(body),
        control,
        has_proxy,
        "token_exchange",
    )
    .await?;
    let response = read_response(response, control, "token_exchange").await?;
    reject_challenge(&response, "token_exchange")?;
    require_json_success(&response, "token_exchange")?;
    parse_new_oauth_tokens(&response.body).map_err(|error| {
        error.at_stage("token_exchange").with_http(
            Some(response.status),
            Some(response.kind),
            Some(false),
        )
    })
}

struct ResponseData {
    status: u16,
    kind: &'static str,
    challenge: bool,
    body: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for ResponseData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseData")
            .field("status", &self.status)
            .field("kind", &self.kind)
            .field("challenge", &self.challenge)
            .field("body_len", &self.body.len())
            .finish()
    }
}

async fn send_request(
    request: reqwest::RequestBuilder,
    control: &LoginControl,
    has_proxy: bool,
    stage: &'static str,
) -> Result<reqwest::Response, OAuthFlowError> {
    let result = tokio::select! {
        _ = control.cancelled() => return Err(cancelled_error(stage)),
        result = tokio::time::timeout(REQUEST_TIMEOUT, request.send()) => result,
    };
    match result {
        Ok(Ok(response)) => Ok(response),
        Err(_) => Err(OAuthFlowError::new(
            OAuthErrorCode::OAuthNetwork,
            true,
            "The OAuth request timed out",
        )
        .at_stage(stage)
        .with_transport("timeout")),
        Ok(Err(error)) => {
            let (code, transport) = classify_request_error(&error, has_proxy);
            Err(
                OAuthFlowError::new(code, true, "The OAuth request could not connect")
                    .at_stage(stage)
                    .with_transport(transport),
            )
        }
    }
}

pub(super) fn classify_request_error(
    error: &reqwest::Error,
    has_proxy: bool,
) -> (OAuthErrorCode, &'static str) {
    if error.is_timeout() {
        (OAuthErrorCode::OAuthNetwork, "timeout")
    } else if has_tls_source(error) {
        (OAuthErrorCode::TlsFailed, "tls")
    } else if error.is_connect() && has_proxy {
        (OAuthErrorCode::ProxyConnectFailed, "proxy_connect")
    } else {
        (OAuthErrorCode::OAuthNetwork, "unknown")
    }
}

fn has_tls_source(error: &reqwest::Error) -> bool {
    fn visit(error: &(dyn std::error::Error + 'static), depth: usize) -> bool {
        if depth > 12 || error.downcast_ref::<rustls::Error>().is_some() {
            return depth <= 12;
        }
        if let Some(inner) = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
        {
            if visit(inner, depth + 1) {
                return true;
            }
        }
        error
            .source()
            .is_some_and(|source| visit(source, depth + 1))
    }
    visit(error, 0)
}

async fn read_response(
    mut response: reqwest::Response,
    control: &LoginControl,
    stage: &'static str,
) -> Result<ResponseData, OAuthFlowError> {
    let status = response.status().as_u16();
    let challenge = response
        .headers()
        .get("cf-mitigated")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("challenge"));
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    let declared_kind = declared_response_kind(content_type);
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    let mut body = Zeroizing::new(Vec::new());
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(OAuthFlowError::new(
                OAuthErrorCode::OAuthNetwork,
                true,
                "The OAuth response timed out",
            )
            .at_stage(stage)
            .with_transport("timeout"));
        }
        let chunk = tokio::select! {
            _ = control.cancelled() => return Err(cancelled_error(stage)),
            result = tokio::time::timeout(remaining, response.chunk()) => result,
        };
        match chunk {
            Ok(Ok(Some(chunk))) => {
                if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(OAuthFlowError::protocol("The OAuth response is too large")
                        .at_stage(stage)
                        .with_http(Some(status), Some(declared_kind), Some(challenge)));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Ok(None)) => break,
            Ok(Err(_)) => {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::OAuthNetwork,
                    true,
                    "The OAuth response could not be read",
                )
                .at_stage(stage)
                .with_http(Some(status), Some(declared_kind), Some(challenge))
                .with_transport("http"))
            }
            Err(_) => {
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::OAuthNetwork,
                    true,
                    "The OAuth response timed out",
                )
                .at_stage(stage)
                .with_http(Some(status), Some(declared_kind), Some(challenge))
                .with_transport("timeout"))
            }
        }
    }
    let kind = response_kind(declared_kind, body.is_empty());
    Ok(ResponseData {
        status,
        kind,
        challenge,
        body,
    })
}

fn reject_challenge(response: &ResponseData, stage: &'static str) -> Result<(), OAuthFlowError> {
    if response.challenge {
        Err(OAuthFlowError::new(
            OAuthErrorCode::OAuthChallengeResponse,
            true,
            "The OAuth endpoint returned a challenge response",
        )
        .at_stage(stage)
        .with_http(Some(response.status), Some(response.kind), Some(true)))
    } else {
        Ok(())
    }
}

fn require_json_success(
    response: &ResponseData,
    stage: &'static str,
) -> Result<(), OAuthFlowError> {
    if matches!(response.kind, "html" | "other" | "unknown") {
        return Err(unexpected_content_type(response, stage));
    }
    if !(200..300).contains(&response.status) {
        return Err(OAuthFlowError::protocol("The OAuth request was rejected")
            .at_stage(stage)
            .with_http(Some(response.status), Some(response.kind), Some(false)));
    }
    if response.kind != "json" {
        return Err(unexpected_content_type(response, stage));
    }
    Ok(())
}

fn unexpected_content_type(response: &ResponseData, stage: &'static str) -> OAuthFlowError {
    OAuthFlowError::new(
        OAuthErrorCode::OAuthUnexpectedContentType,
        true,
        "The OAuth endpoint returned an unexpected content type",
    )
    .at_stage(stage)
    .with_http(Some(response.status), Some(response.kind), Some(false))
}

async fn cancellable_sleep(
    duration: Duration,
    control: &LoginControl,
    stage: &'static str,
) -> Result<(), OAuthFlowError> {
    tokio::select! {
        _ = control.cancelled() => Err(cancelled_error(stage)),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}

fn cancelled_error(stage: &'static str) -> OAuthFlowError {
    OAuthFlowError::new(
        OAuthErrorCode::AuthCancelled,
        true,
        "Codex sign-in was cancelled",
    )
    .at_stage(stage)
}

fn validate_code_success(response: &CodeSuccessResponse) -> Result<(), OAuthFlowError> {
    if response.authorization_code.is_empty()
        || response.authorization_code.len() > 16 * 1024
        || response.code_challenge.is_empty()
        || response.code_challenge.len() > 1_024
        || response.code_verifier.len() < 43
        || response.code_verifier.len() > 1_024
    {
        return Err(
            OAuthFlowError::protocol("The device authorization response is invalid")
                .at_stage("device_wait"),
        );
    }
    let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(response.code_verifier.as_bytes()));
    if expected != response.code_challenge {
        return Err(
            OAuthFlowError::protocol("The device authorization PKCE binding is invalid")
                .at_stage("device_wait"),
        );
    }
    Ok(())
}

fn valid_user_code(value: &str) -> bool {
    let Some((left, right)) = value.split_once('-') else {
        return false;
    };
    !value.contains(['\r', '\n'])
        && (4..=8).contains(&left.len())
        && (4..=8).contains(&right.len())
        && left
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        && right
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

struct PkceCodes {
    verifier: Zeroizing<String>,
    challenge: String,
}

fn generate_pkce() -> Result<PkceCodes, OAuthFlowError> {
    let mut random = [0_u8; 64];
    getrandom::getrandom(&mut random).map_err(|_| {
        OAuthFlowError::new(
            OAuthErrorCode::Storage,
            false,
            "Secure random generation failed",
        )
    })?;
    let verifier = Zeroizing::new(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random));
    random.zeroize();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    Ok(PkceCodes {
        verifier,
        challenge,
    })
}

fn generate_state() -> Result<Zeroizing<String>, OAuthFlowError> {
    let mut random = [0_u8; 32];
    getrandom::getrandom(&mut random).map_err(|_| {
        OAuthFlowError::new(
            OAuthErrorCode::Storage,
            false,
            "Secure random generation failed",
        )
    })?;
    let state = Zeroizing::new(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random));
    random.zeroize();
    Ok(state)
}

fn build_authorization_url(
    issuer: &str,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<Zeroizing<String>, OAuthFlowError> {
    let mut url = url::Url::parse(issuer)
        .map_err(|_| OAuthFlowError::protocol("The OAuth issuer is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(OAuthFlowError::protocol("The OAuth issuer is invalid"));
    }
    url.set_path("/oauth/authorize");
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", CODEX_OAUTH_SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", CODEX_OAUTH_ORIGINATOR);
    Ok(Zeroizing::new(url.into()))
}

async fn open_browser(url: &str, control: &LoginControl) -> Result<(), OAuthFlowError> {
    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("/usr/bin/open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                OAuthFlowError::new(
                    OAuthErrorCode::BrowserOpenFailed,
                    true,
                    "The system browser could not be opened",
                )
                .at_stage("browser_open")
            })?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) | Err(_) => {
                    return Err(OAuthFlowError::new(
                        OAuthErrorCode::BrowserOpenFailed,
                        true,
                        "The system browser could not be opened",
                    )
                    .at_stage("browser_open"))
                }
                Ok(None) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(OAuthFlowError::new(
                    OAuthErrorCode::BrowserOpenFailed,
                    true,
                    "The system browser did not return promptly",
                )
                .at_stage("browser_open"));
            }
            tokio::select! {
                _ = control.cancelled() => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(cancelled_error("browser_open"));
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (url, control);
        Err(OAuthFlowError::new(
            OAuthErrorCode::BrowserOpenFailed,
            false,
            "Codex browser login is supported only on macOS",
        )
        .at_stage("browser_open"))
    }
}

fn bind_callback(ports: &[u16]) -> Result<(tokio::net::TcpListener, u16), OAuthFlowError> {
    for port in ports {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", *port)) {
            listener.set_nonblocking(true).map_err(|_| {
                OAuthFlowError::new(
                    OAuthErrorCode::CallbackUnavailable,
                    true,
                    "The local OAuth callback could not be configured",
                )
                .at_stage("callback_wait")
            })?;
            let actual = listener.local_addr().map_err(|_| {
                OAuthFlowError::new(
                    OAuthErrorCode::CallbackUnavailable,
                    true,
                    "The local OAuth callback address could not be read",
                )
                .at_stage("callback_wait")
            })?;
            return tokio::net::TcpListener::from_std(listener)
                .map(|listener| (listener, actual.port()))
                .map_err(|_| {
                    OAuthFlowError::new(
                        OAuthErrorCode::CallbackUnavailable,
                        true,
                        "The local OAuth callback could not be configured",
                    )
                    .at_stage("callback_wait")
                });
        }
    }
    Err(OAuthFlowError::new(
        OAuthErrorCode::CallbackUnavailable,
        true,
        "OAuth callback ports 1455 and 1457 are unavailable",
    )
    .at_stage("callback_wait"))
}

enum CallbackAction {
    Ignore(u16),
    Denied,
    Code(Zeroizing<String>),
}

async fn read_and_parse_callback(
    stream: &mut tokio::net::TcpStream,
    port: u16,
    expected_state: &str,
    control: &LoginControl,
) -> Result<CallbackAction, OAuthFlowError> {
    let deadline = tokio::time::Instant::now() + CALLBACK_IO_TIMEOUT;
    let mut head = Zeroizing::new(Vec::with_capacity(2048));
    let mut chunk = [0_u8; 1024];
    while !head.ends_with(b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(
                OAuthFlowError::protocol("The OAuth callback timed out").at_stage("callback_wait")
            );
        }
        let read = tokio::select! {
            _ = control.cancelled() => return Err(cancelled_error("callback_wait")),
            result = tokio::time::timeout(remaining, stream.read(&mut chunk)) => result,
        };
        let read = read
            .map_err(|_| OAuthFlowError::protocol("The OAuth callback timed out"))?
            .map_err(|_| OAuthFlowError::protocol("The OAuth callback could not be read"))?;
        if read == 0 || head.len().saturating_add(read) > MAX_CALLBACK_HEAD {
            return Err(
                OAuthFlowError::protocol("The OAuth callback is invalid").at_stage("callback_wait")
            );
        }
        head.extend_from_slice(&chunk[..read]);
    }
    parse_callback_head(&head, port, expected_state)
}

fn parse_callback_head(
    head: &[u8],
    port: u16,
    expected_state: &str,
) -> Result<CallbackAction, OAuthFlowError> {
    let text = std::str::from_utf8(head)
        .map_err(|_| OAuthFlowError::protocol("The OAuth callback is invalid"))?;
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| OAuthFlowError::protocol("The OAuth callback is invalid"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if method != "GET" || !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return Err(OAuthFlowError::protocol("The OAuth callback is invalid"));
    }
    let mut host = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(OAuthFlowError::protocol("The OAuth callback is invalid"));
        };
        if name.trim().eq_ignore_ascii_case("host")
            && host.replace(value.trim().to_ascii_lowercase()).is_some()
        {
            return Err(OAuthFlowError::protocol("The OAuth callback is invalid"));
        }
    }
    if !matches!(host.as_deref(), Some(value) if value == format!("localhost:{port}") || value == format!("127.0.0.1:{port}"))
    {
        return Err(OAuthFlowError::protocol(
            "The OAuth callback host is invalid",
        ));
    }
    if !target.starts_with('/') || target.starts_with("//") || target.contains('#') {
        return Err(OAuthFlowError::protocol(
            "The OAuth callback URL is invalid",
        ));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Ok(CallbackAction::Ignore(404));
    }
    let mut seen = HashSet::new();
    let mut callback_state = None;
    let mut code = None;
    let mut denied = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let key = Zeroizing::new(key.into_owned());
        let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        if !seen.insert(digest) {
            return Err(OAuthFlowError::protocol(
                "The OAuth callback contains duplicate fields",
            ));
        }
        let value = Zeroizing::new(value.into_owned());
        match key.as_str() {
            "state" => callback_state = Some(value),
            "code" => code = Some(value),
            "error" => denied = !value.is_empty(),
            _ => {}
        }
    }
    if callback_state.as_deref().map(String::as_str) != Some(expected_state) {
        return Err(OAuthFlowError::protocol(
            "The OAuth callback state is invalid",
        ));
    }
    if denied {
        return Ok(CallbackAction::Denied);
    }
    Ok(CallbackAction::Code(
        code.filter(|value| !value.is_empty())
            .ok_or_else(|| OAuthFlowError::protocol("The OAuth callback code is missing"))?,
    ))
}

async fn write_callback(stream: &mut tokio::net::TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Bad Request",
    };
    let body = format!("<html><body>{message}</body></html>");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::thread;

    type SecretMap = HashMap<(String, String), Vec<u8>>;

    #[derive(Clone, Default)]
    struct MemorySecrets(Arc<Mutex<SecretMap>>);

    impl SecretStore for MemorySecrets {
        fn load(
            &self,
            service: &str,
            account: &str,
        ) -> Result<Option<Vec<u8>>, super::super::storage::StorageError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned())
        }

        fn save(
            &self,
            service: &str,
            account: &str,
            value: &[u8],
        ) -> Result<(), super::super::storage::StorageError> {
            self.0
                .lock()
                .unwrap()
                .insert((service.to_string(), account.to_string()), value.to_vec());
            Ok(())
        }

        fn delete(
            &self,
            service: &str,
            account: &str,
        ) -> Result<(), super::super::storage::StorageError> {
            self.0
                .lock()
                .unwrap()
                .remove(&(service.to_string(), account.to_string()));
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MemoryState(Arc<Mutex<Option<super::super::storage::AuthState>>>);

    impl StateStore for MemoryState {
        fn load(
            &self,
        ) -> Result<Option<super::super::storage::AuthState>, super::super::storage::StorageError>
        {
            Ok(self.0.lock().unwrap().clone())
        }

        fn commit(
            &self,
            state: &super::super::storage::AuthState,
        ) -> Result<(), super::super::storage::StorageError> {
            *self.0.lock().unwrap() = Some(state.clone());
            Ok(())
        }
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let mut random = [0_u8; 8];
            getrandom::getrandom(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "csswitch-login-async-test-{}-{}",
                std::process::id(),
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn repository(root: &TempRoot) -> AuthRepository<MemorySecrets, MemoryState> {
        AuthRepository::new(
            MemorySecrets::default(),
            MemoryState::default(),
            root.0.clone(),
        )
    }

    fn http_response(status: &str, content_type: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n{headers}\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn spawn_sequence_server(responses: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if let Some(head_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let head_end = head_end + 4;
                        let head = String::from_utf8_lossy(&request[..head_end]);
                        let content_length = head
                            .lines()
                            .find_map(|line| {
                                line.split_once(':').and_then(|(name, value)| {
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                            })
                            .unwrap_or(0);
                        if request.len() >= head_end + content_length {
                            break;
                        }
                    }
                }
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{address}"), handle)
    }

    fn options(issuer: String) -> AsyncLoginOptions {
        AsyncLoginOptions {
            issuer,
            client_id: "client-test".into(),
            callback_ports: vec![0],
            browser_timeout: Duration::from_secs(2),
            device_timeout: Duration::from_secs(5),
        }
    }

    fn jwt(claims: serde_json::Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        format!("{header}.{payload}.sig")
    }

    fn direct_client() -> reqwest::Client {
        CodexHttpClientFactory::direct_for_test()
            .async_builder()
            .unwrap()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[test]
    fn user_code_and_pkce_binding_are_strict() {
        assert!(valid_user_code("ABCD-1234"));
        assert!(!valid_user_code("abcd-1234"));
        assert!(!valid_user_code("ABCD1234"));
        let pkce = generate_pkce().unwrap();
        let response = CodeSuccessResponse {
            authorization_code: "code".into(),
            code_challenge: pkce.challenge,
            code_verifier: pkce.verifier.to_string(),
        };
        validate_code_success(&response).unwrap();
    }

    #[test]
    fn cancel_and_commit_use_one_atomic_barrier() {
        let cancel_first = LoginControl::default();
        assert_eq!(cancel_first.cancel(), CancelDisposition::Accepted);
        assert_eq!(
            cancel_first.begin_commit().unwrap_err().code,
            OAuthErrorCode::AuthCancelled
        );

        let commit_first = LoginControl::default();
        commit_first.begin_commit().unwrap();
        assert_eq!(commit_first.cancel(), CancelDisposition::CommitInProgress);
    }

    #[test]
    fn browser_authorization_contract_keeps_pkce_state_and_scope() {
        let url = build_authorization_url(
            CODEX_OAUTH_ISSUER,
            CODEX_OAUTH_CLIENT_ID,
            "http://localhost:1455/auth/callback",
            "challenge",
            "state",
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let fields = parsed
            .query_pairs()
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            fields.get("scope").map(String::as_str),
            Some(CODEX_OAUTH_SCOPE)
        );
        assert_eq!(fields.get("state").map(String::as_str), Some("state"));
        assert_eq!(
            fields.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn device_flow_handles_pending_then_commits_one_generation() {
        let verifier = "v".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let id_token = jwt(serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_account_id": "acct-test"},
            "exp": 2_000_000_000_i64
        }));
        let access_token = jwt(serde_json::json!({
            "chatgpt_account_id": "acct-test",
            "exp": 2_000_000_000_i64
        }));
        let responses = vec![
            http_response(
                "200 OK",
                "application/json",
                "",
                br#"{"device_auth_id":"device-1","user_code":"ABCD-1234","interval":"1"}"#,
            ),
            http_response("403 Forbidden", "application/json", "", b""),
            http_response(
                "200 OK",
                "application/json",
                "",
                serde_json::to_vec(&serde_json::json!({
                    "authorization_code": "authorization-code",
                    "code_challenge": challenge,
                    "code_verifier": verifier,
                }))
                .unwrap()
                .as_slice(),
            ),
            http_response(
                "200 OK",
                "application/json",
                "",
                serde_json::to_vec(&serde_json::json!({
                    "id_token": id_token,
                    "access_token": access_token,
                    "refresh_token": "refresh-test",
                }))
                .unwrap()
                .as_slice(),
            ),
        ];
        let (issuer, server) = spawn_sequence_server(responses);
        let root = TempRoot::new();
        let repository = repository(&root);
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_out = progress.clone();
        let status = run_device_login(
            &repository,
            &direct_client(),
            false,
            &options(issuer),
            &LoginControl::default(),
            &move |event| progress_out.lock().unwrap().push(event),
        )
        .await
        .unwrap();
        assert!(status.authenticated);
        assert_eq!(status.auth_generation, 1);
        assert!(matches!(
            progress.lock().unwrap().first(),
            Some(LoginProgress::VerificationRequired { .. })
        ));
        assert!(matches!(
            progress.lock().unwrap().last(),
            Some(LoginProgress::Committing)
        ));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn device_initial_404_is_unavailable_but_html_poll_is_not_pending() {
        let (issuer, server) = spawn_sequence_server(vec![http_response(
            "404 Not Found",
            "application/json",
            "",
            br#"{}"#,
        )]);
        let root = TempRoot::new();
        let error = run_device_login(
            &repository(&root),
            &direct_client(),
            false,
            &options(issuer),
            &LoginControl::default(),
            &|_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::DeviceAuthUnavailable);
        server.join().unwrap();

        let responses = vec![
            http_response(
                "200 OK",
                "application/json",
                "",
                br#"{"device_auth_id":"device-1","user_code":"ABCD-1234","interval":"1"}"#,
            ),
            http_response(
                "403 Forbidden",
                "text/html; charset=UTF-8",
                "",
                b"<html>challenge</html>",
            ),
        ];
        let (issuer, server) = spawn_sequence_server(responses);
        let root = TempRoot::new();
        let error = run_device_login(
            &repository(&root),
            &direct_client(),
            false,
            &options(issuer),
            &LoginControl::default(),
            &|_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::OAuthUnexpectedContentType);
        assert_eq!(error.response_kind, Some("html"));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cloudflare_header_wins_over_html_and_status() {
        let (issuer, server) = spawn_sequence_server(vec![http_response(
            "403 Forbidden",
            "text/html",
            "cf-mitigated: challenge\r\n",
            b"<html>challenge</html>",
        )]);
        let root = TempRoot::new();
        let error = run_device_login(
            &repository(&root),
            &direct_client(),
            false,
            &options(issuer),
            &LoginControl::default(),
            &|_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::OAuthChallengeResponse);
        assert_eq!(error.challenge_detected, Some(true));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hung_header_slow_body_and_poll_sleep_are_cancellable() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(500));
        });
        let control = LoginControl::default();
        let cancel = control.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let error = send_request(
            direct_client().get(format!("http://{address}/hung")),
            &control,
            false,
            "device_code_request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::AuthCancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 20\r\nconnection: close\r\n\r\n{",
            );
            let _ = stream.flush();
            thread::sleep(Duration::from_secs(2));
        });
        let control = LoginControl::default();
        let response = send_request(
            direct_client().get(format!("http://{address}/slow")),
            &control,
            false,
            "token_exchange",
        )
        .await
        .unwrap();
        let cancel = control.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let error = read_response(response, &control, "token_exchange")
            .await
            .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::AuthCancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();

        let control = LoginControl::default();
        let cancel = control.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let error = cancellable_sleep(Duration::from_secs(30), &control, "device_wait")
            .await
            .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::AuthCancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stable_rustls_source_is_classified_as_tls_failure() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut hello = [0_u8; 1024];
            let _ = stream.read(&mut hello);
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            let _ = stream.flush();
        });
        let error = send_request(
            direct_client().get(format!("https://{address}/tls")),
            &LoginControl::default(),
            false,
            "token_exchange",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::TlsFailed);
        assert_eq!(error.transport_kind, Some("tls"));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unresponsive_connect_proxy_is_cancellable() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let proxy = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            thread::sleep(Duration::from_secs(2));
        });
        let route = csswitch_codex_network::resolve(
            &csswitch_codex_network::CodexNetworkSettings {
                mode: csswitch_codex_network::CodexNetworkMode::Custom,
                proxy_url: format!("http://{proxy}"),
            },
            &csswitch_codex_network::EnvironmentSnapshot::default(),
        )
        .unwrap();
        let client = CodexHttpClientFactory::for_test_route(route)
            .async_builder()
            .unwrap()
            .build()
            .unwrap();
        let control = LoginControl::default();
        let cancel = control.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let error = send_request(
            client.get("https://unresolvable.test:443/device"),
            &control,
            true,
            "device_code_request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::AuthCancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refused_proxy_is_classified_without_exposing_url() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let proxy = listener.local_addr().unwrap();
        drop(listener);
        let route = csswitch_codex_network::resolve(
            &csswitch_codex_network::CodexNetworkSettings {
                mode: csswitch_codex_network::CodexNetworkMode::Custom,
                proxy_url: format!("http://{proxy}"),
            },
            &csswitch_codex_network::EnvironmentSnapshot::default(),
        )
        .unwrap();
        let client = CodexHttpClientFactory::for_test_route(route)
            .async_builder()
            .unwrap()
            .build()
            .unwrap();
        let error = send_request(
            client.get("https://unresolvable.test:443/device"),
            &LoginControl::default(),
            true,
            "device_code_request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::ProxyConnectFailed);
        assert_eq!(error.transport_kind, Some("proxy_connect"));
        assert!(!format!("{error:?} {error}").contains(&proxy.to_string()));
    }
}
