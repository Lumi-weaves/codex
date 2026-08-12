use crate::error_code::internal_error;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_http_client::HttpClient;
use codex_http_client::HttpClientBuilder;
use hmac::Hmac;
use hmac::Mac;
use reqwest::Method;
use reqwest::StatusCode;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use uuid::Uuid;

const RUNTIME_RECORD_LIMIT: u64 = 16 * 1024;
const CAPABILITY_TTL_MS: u64 = 10_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePortState {
    pid: u32,
    port: u16,
    hostname: Option<String>,
    model_workbench_capability_secret: String,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    service: String,
    pid: u32,
    port: u16,
}

struct WorkbenchTarget {
    pid: u32,
    port: u16,
    host: String,
    secret: String,
}

pub(super) async fn request_workbench(
    method: Method,
    path: &str,
    body: Vec<u8>,
    expected_revision: Option<u64>,
) -> Result<reqwest::Response, JSONRPCErrorError> {
    let target = read_target()?;
    let client = direct_client()?;
    let base_url = format!("http://{}:{}", target.host, target.port);
    verify_health(&client, &base_url, &target).await?;

    let nonce = URL_SAFE_NO_PAD.encode(Sha256::digest(Uuid::new_v4().as_bytes()));
    let content_sha256 = URL_SAFE_NO_PAD.encode(Sha256::digest(&body));
    let expires_at = now_millis()?.saturating_add(CAPABILITY_TTL_MS);
    let capability = create_capability(
        &target.secret,
        &nonce,
        &method,
        path,
        target.pid,
        target.port,
        expires_at,
        &content_sha256,
    )?;

    let mut request = client
        .request(method, format!("{base_url}{path}"))
        .header("x-opencodex-workbench-expected-pid", target.pid.to_string())
        .header("x-opencodex-workbench-nonce", nonce)
        .header("x-opencodex-workbench-expires-at", expires_at.to_string())
        .header("x-opencodex-workbench-content-sha256", content_sha256)
        .header("x-opencodex-workbench-capability", capability)
        .timeout(Duration::from_secs(4));
    if !body.is_empty() {
        request = request
            .header("content-type", "application/json")
            .body(body);
    }
    if let Some(revision) = expected_revision {
        request = request.header("if-match", format!("\"{revision}\""));
    }
    request
        .send()
        .await
        .map_err(|_| internal_error("OpenCodex Model Workbench is unavailable"))
}

async fn verify_health(
    client: &HttpClient,
    base_url: &str,
    target: &WorkbenchTarget,
) -> Result<(), JSONRPCErrorError> {
    let response = client
        .get(format!("{base_url}/healthz"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map_err(|_| internal_error("OpenCodex runtime identity is unavailable"))?;
    if response.status() != StatusCode::OK {
        return Err(internal_error("OpenCodex runtime identity is unavailable"));
    }
    let bytes = super::model_workbench_processor::read_bounded(response).await?;
    let health: HealthResponse = serde_json::from_slice(&bytes)
        .map_err(|_| internal_error("OpenCodex runtime identity is invalid"))?;
    if health.service != "opencodex" || health.pid != target.pid || health.port != target.port {
        return Err(internal_error("OpenCodex runtime identity changed"));
    }
    Ok(())
}

fn read_target() -> Result<WorkbenchTarget, JSONRPCErrorError> {
    let home = opencodex_home()?;
    reject_unsafe_directory(&home)?;
    let path = home.join("runtime-port.json");
    let bytes = read_protected_runtime_record(&path)?;
    let runtime: RuntimePortState = serde_json::from_slice(&bytes)
        .map_err(|_| internal_error("OpenCodex runtime record is invalid"))?;
    if runtime.pid == 0
        || runtime.port == 0
        || !is_base64url_256(&runtime.model_workbench_capability_secret)
    {
        return Err(internal_error(
            "OpenCodex Model Workbench capability is unavailable",
        ));
    }
    let host = loopback_host(runtime.hostname.as_deref())?;
    Ok(WorkbenchTarget {
        pid: runtime.pid,
        port: runtime.port,
        host,
        secret: runtime.model_workbench_capability_secret,
    })
}

fn opencodex_home() -> Result<PathBuf, JSONRPCErrorError> {
    if let Some(raw) = std::env::var_os("OPENCODEX_HOME") {
        let path = PathBuf::from(raw);
        if path.as_os_str().is_empty() || !path.is_absolute() {
            return Err(internal_error("OPENCODEX_HOME must be an absolute path"));
        }
        return Ok(path);
    }
    dirs::home_dir()
        .map(|home| home.join(".opencodex"))
        .ok_or_else(|| internal_error("OpenCodex home is unavailable"))
}

fn reject_unsafe_directory(path: &Path) -> Result<(), JSONRPCErrorError> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|_| internal_error("OpenCodex protected state is unavailable"))?;
        if metadata.file_type().is_symlink() {
            return Err(internal_error("OpenCodex protected state path is unsafe"));
        }
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| internal_error("OpenCodex protected state is unavailable"))?;
    if !metadata.is_dir() {
        return Err(internal_error("OpenCodex protected state path is unsafe"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(internal_error(
                "OpenCodex protected state permissions are unsafe",
            ));
        }
    }
    Ok(())
}

fn read_protected_runtime_record(path: &Path) -> Result<Vec<u8>, JSONRPCErrorError> {
    use std::fs::OpenOptions;
    use std::io::Read;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|_| internal_error("OpenCodex runtime record is unavailable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| internal_error("OpenCodex runtime record is unavailable"))?;
    if !metadata.is_file() || metadata.len() > RUNTIME_RECORD_LIMIT {
        return Err(internal_error("OpenCodex runtime record is invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(internal_error(
                "OpenCodex protected state permissions are unsafe",
            ));
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(RUNTIME_RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| internal_error("OpenCodex runtime record is unavailable"))?;
    if bytes.len() as u64 > RUNTIME_RECORD_LIMIT {
        return Err(internal_error("OpenCodex runtime record is invalid"));
    }
    Ok(bytes)
}

pub(super) fn loopback_host(hostname: Option<&str>) -> Result<String, JSONRPCErrorError> {
    match hostname.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("0.0.0.0") | Some("::") | Some("[::]") | Some("127.0.0.1")
        | Some("localhost") => Ok("127.0.0.1".to_string()),
        Some("::1") | Some("[::1]") => Ok("[::1]".to_string()),
        Some(_) => Err(internal_error(
            "OpenCodex Model Workbench requires a loopback runtime",
        )),
    }
}

pub(super) fn direct_client() -> Result<HttpClient, JSONRPCErrorError> {
    HttpClientBuilder::new()
        .without_request_logging()
        .without_redirects()
        .connect_timeout(Duration::from_secs(2))
        .build_direct()
        .map_err(|_| internal_error("OpenCodex local transport is unavailable"))
}

fn now_millis() -> Result<u64, JSONRPCErrorError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .map_err(|_| internal_error("System clock is unavailable"))
}

pub(super) fn is_base64url_256(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_capability(
    secret: &str,
    nonce: &str,
    method: &Method,
    path: &str,
    pid: u32,
    port: u16,
    expires_at: u64,
    content_sha256: &str,
) -> Result<String, JSONRPCErrorError> {
    if pid == 0
        || port == 0
        || expires_at == 0
        || !is_base64url_256(secret)
        || !is_base64url_256(nonce)
        || !is_base64url_256(content_sha256)
    {
        return Err(internal_error(
            "OpenCodex Model Workbench capability is unavailable",
        ));
    }
    let payload = format!(
        "opencodex-model-workbench-v1\n{nonce}\n{method}\n{path}\n{pid}\n{port}\n{expires_at}\n{content_sha256}"
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| internal_error("OpenCodex Model Workbench capability is unavailable"))?;
    mac.update(payload.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}
