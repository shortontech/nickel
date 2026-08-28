use std::{
    collections::HashSet,
    env,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{CodexError, REQUIRED_PROFILE};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    Installed,
    Bundled,
    Explicit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub source: CandidateSource,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendChoice {
    Automatic,
    Installed,
    Bundled,
    Path(PathBuf),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Compatibility {
    pub candidate: Candidate,
    pub version: Option<String>,
    pub executable_sha256: Option<String>,
    pub compatible: bool,
    pub reason: String,
    pub additive_methods: Vec<String>,
    pub protocol_profile_sha256: String,
    pub generated_schema_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub selected: Option<Candidate>,
    pub probes: Vec<Compatibility>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProbeLimits {
    pub command_timeout: Duration,
    pub handshake_timeout: Duration,
}

impl Default for ProbeLimits {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Selector {
    bundled: Option<PathBuf>,
    limits: ProbeLimits,
}

impl Selector {
    pub fn new(bundled: Option<PathBuf>) -> Self {
        Self {
            bundled,
            limits: ProbeLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: ProbeLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn platform_default() -> Self {
        let bundled = env::var_os("NICKEL_BUNDLED_CODEX")
            .map(PathBuf::from)
            .or_else(|| {
                env::current_exe().ok().and_then(|exe| {
                    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
                    exe.parent()
                        .map(|parent| parent.join("runtime/codex").join(name))
                })
            });
        Self::new(bundled)
    }

    pub fn discover(&self, path: Option<&OsString>) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        if let Some(installed) =
            find_on_path(path.unwrap_or(&env::var_os("PATH").unwrap_or_default()))
        {
            candidates.push(Candidate {
                source: CandidateSource::Installed,
                path: installed,
            });
        }
        if let Some(path) = &self.bundled {
            candidates.push(Candidate {
                source: CandidateSource::Bundled,
                path: path.clone(),
            });
        }
        deduplicate(candidates)
    }

    pub fn select(&self, choice: BackendChoice) -> Selection {
        self.select_with_path(choice, None)
    }

    pub fn select_with_path(&self, choice: BackendChoice, path: Option<&OsString>) -> Selection {
        let all = self.discover(path);
        let automatic = matches!(choice, BackendChoice::Automatic);
        let candidates = match choice {
            BackendChoice::Automatic => all,
            BackendChoice::Installed => all
                .into_iter()
                .filter(|candidate| candidate.source == CandidateSource::Installed)
                .collect(),
            BackendChoice::Bundled => all
                .into_iter()
                .filter(|candidate| candidate.source == CandidateSource::Bundled)
                .collect(),
            BackendChoice::Path(path) => vec![Candidate {
                source: CandidateSource::Explicit,
                path,
            }],
        };
        let mut probes = Vec::new();
        let mut selected = None;
        for candidate in candidates {
            let result = self.probe(candidate.clone());
            let compatible = result.compatible;
            probes.push(result);
            if compatible {
                selected = Some(candidate);
                break;
            }
            if !automatic {
                break;
            }
        }
        Selection { selected, probes }
    }

    pub fn probe(&self, candidate: Candidate) -> Compatibility {
        match self.probe_inner(&candidate) {
            Ok((version, digest, additive_methods, schema_digest)) => Compatibility {
                candidate,
                version: Some(version),
                executable_sha256: digest,
                compatible: true,
                reason: "required schema and initialize handshake accepted".into(),
                additive_methods,
                protocol_profile_sha256: profile_digest(),
                generated_schema_sha256: Some(schema_digest),
            },
            Err(error) => Compatibility {
                candidate,
                version: None,
                executable_sha256: None,
                compatible: false,
                reason: error.to_string(),
                additive_methods: Vec::new(),
                protocol_profile_sha256: profile_digest(),
                generated_schema_sha256: None,
            },
        }
    }

    fn probe_inner(
        &self,
        candidate: &Candidate,
    ) -> Result<(String, Option<String>, Vec<String>, String), CodexError> {
        if !candidate.path.is_file() {
            return Err(CodexError::Unavailable(format!(
                "candidate does not exist: {}",
                candidate.path.display()
            )));
        }
        let version = run_bounded(&candidate.path, &["--version"], self.limits.command_timeout)?;
        let version = version.lines().next().unwrap_or_default().trim().to_owned();
        if version.is_empty() {
            return Err(CodexError::Incompatible("empty version identity".into()));
        }
        let schema_dir = tempfile::Builder::new()
            .prefix("nickel-codex-schema")
            .tempdir()?;
        let output = Command::new(&candidate.path)
            .args(["app-server", "generate-json-schema", "--out"])
            .arg(schema_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        wait_child(output, self.limits.command_timeout)?;
        let (additive_methods, schema_digest) = validate_schema(schema_dir.path())?;
        initialize_handshake(&candidate.path, self.limits.handshake_timeout)?;
        let digest = fs::read(&candidate.path).ok().map(|bytes| {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            format!("{:x}", hasher.finalize())
        });
        Ok((version, digest, additive_methods, schema_digest))
    }
}

fn profile_digest() -> String {
    let mut hasher = Sha256::new();
    hasher.update(REQUIRED_PROFILE.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn find_on_path(path: &OsString) -> Option<PathBuf> {
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    env::split_paths(path)
        .filter(|entry| !entry.as_os_str().is_empty())
        .map(|entry| entry.join(name))
        .find(|candidate| candidate.is_file())
}

fn deduplicate(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            let identity =
                fs::canonicalize(&candidate.path).unwrap_or_else(|_| candidate.path.clone());
            seen.insert(identity)
        })
        .collect()
}

fn run_bounded(path: &Path, args: &[&str], timeout: Duration) -> Result<String, CodexError> {
    let mut child = Command::new(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            if !output.status.success() {
                return Err(CodexError::Incompatible(
                    String::from_utf8_lossy(&output.stderr)
                        .chars()
                        .take(4096)
                        .collect(),
                ));
            }
            return String::from_utf8(output.stdout)
                .map_err(|_| CodexError::Protocol("command stdout is not UTF-8".into()));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CodexError::Timeout("candidate command timed out".into()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_child(mut child: std::process::Child, timeout: Duration) -> Result<(), CodexError> {
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let stderr = child
                .stderr
                .take()
                .map(|mut stream| {
                    let mut text = String::new();
                    let _ = std::io::Read::read_to_string(&mut stream, &mut text);
                    text
                })
                .unwrap_or_default();
            return Err(CodexError::Incompatible(
                stderr.chars().take(4096).collect(),
            ));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CodexError::Timeout("schema generation timed out".into()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_schema(directory: &Path) -> Result<(Vec<String>, String), CodexError> {
    let profile: Value = serde_json::from_str(REQUIRED_PROFILE)?;
    let groups = [
        ("client_requests", "ClientRequest.json"),
        ("client_notifications", "ClientNotification.json"),
        ("server_requests", "ServerRequest.json"),
        ("server_notifications", "ServerNotification.json"),
    ];
    let mut additions = Vec::new();
    let mut schema_hasher = Sha256::new();
    for (group, filename) in groups {
        let schema = fs::read_to_string(directory.join(filename)).map_err(|error| {
            CodexError::Incompatible(format!("missing generated {filename}: {error}"))
        })?;
        schema_hasher.update(filename.as_bytes());
        schema_hasher.update(schema.as_bytes());
        let schema: Value = serde_json::from_str(&schema)?;
        let mut actual = HashSet::new();
        collect_methods(&schema, None, &mut actual);
        let required: HashSet<_> = profile[group]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for method in &required {
            if !actual.contains(*method) {
                return Err(CodexError::Incompatible(format!(
                    "required method absent from schema: {method}"
                )));
            }
        }
        additions.extend(
            actual
                .into_iter()
                .filter(|method| !required.contains(method.as_str())),
        );
    }
    additions.sort();
    additions.dedup();
    Ok((additions, format!("{:x}", schema_hasher.finalize())))
}

fn collect_methods(value: &Value, parent: Option<&str>, methods: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                collect_methods(value, Some(key), methods);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_methods(value, parent, methods);
            }
        }
        Value::String(string)
            if matches!(parent, Some("const" | "enum" | "methods"))
                && (parent == Some("methods")
                    || string.contains('/')
                    || matches!(string.as_str(), "initialize" | "initialized" | "error")) =>
        {
            methods.insert(string.clone());
        }
        _ => {}
    }
}

fn initialize_handshake(path: &Path, timeout: Duration) -> Result<(), CodexError> {
    let mut child = Command::new(path)
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CodexError::Protocol("missing stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CodexError::Protocol("missing stdout".into()))?;
    writeln!(
        stdin,
        "{}",
        json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "nickel-probe", "version": env!("CARGO_PKG_VERSION")}}})
    )?;
    stdin.flush()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let outcome = rx.recv_timeout(timeout);
    let _ = child.kill();
    let _ = child.wait();
    let line =
        outcome.map_err(|_| CodexError::Timeout("initialize handshake timed out".into()))??;
    let response: Value = serde_json::from_str(line.trim())?;
    if response.get("id") != Some(&Value::from(1)) || response.get("error").is_some() {
        return Err(CodexError::Incompatible(
            "initialize response rejected".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_deduplicates_installed_and_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory
            .path()
            .join(if cfg!(windows) { "codex.exe" } else { "codex" });
        fs::write(&executable, b"fixture").unwrap();
        let selector = Selector::new(Some(executable.clone()));
        let path = env::join_paths([directory.path()]).unwrap();
        assert_eq!(
            selector.discover(Some(&path)),
            vec![Candidate {
                source: CandidateSource::Installed,
                path: executable,
            }]
        );
    }

    #[test]
    fn explicit_missing_candidate_fails_without_fallback() {
        let selection =
            Selector::new(None).select(BackendChoice::Path(PathBuf::from("/missing/codex")));
        assert!(selection.selected.is_none());
        assert_eq!(selection.probes.len(), 1);
        assert_eq!(
            selection.probes[0].candidate.source,
            CandidateSource::Explicit
        );
        assert_eq!(
            selection.probes[0].candidate.path,
            PathBuf::from("/missing/codex")
        );
        assert!(!selection.probes[0].compatible);
        assert!(selection.probes[0].reason.contains("does not exist"));
    }
}
