//! Bounded stdio bridge. Connection watches are transport presence, never lease renewal.
use reqwest::{
    Client, Response, Url,
    header::{HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    error::Error,
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const LIMIT: usize = 96 * 1024 * 1024;
const REQUEST_LIMIT: usize = 1024 * 1024;
const CONCURRENT_REQUESTS: usize = 4;
const WATCH_PROTOCOL: &str = "2026-07-28";

#[derive(Clone)]
struct Bridge {
    client: Client,
    url: Url,
}
impl Bridge {
    fn configured() -> Result<Self> {
        let url = Url::parse(&env::var("NICKEL_MCP_URL")?)?;
        validate_url(&url)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-nickel-client",
            HeaderValue::from_str(&env::var("NICKEL_MCP_CLIENT")?)?,
        );
        let mut token =
            HeaderValue::from_str(&format!("Bearer {}", env::var("NICKEL_MCP_TOKEN")?))?;
        token.set_sensitive(true);
        headers.insert("authorization", token);
        headers.insert(
            "accept",
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        let mut builder = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5));
        if url.scheme() == "https" {
            let path = env::var("NICKEL_MCP_CA_FILE")?;
            let mut pem = Vec::new();
            std::fs::File::open(path)?
                .take(65537)
                .read_to_end(&mut pem)?;
            if pem.len() > 65536 {
                return Err("CA file exceeds limit".into());
            }
            builder = trust_certificate(builder, &pem)?;
        }
        Ok(Self {
            client: builder.build()?,
            url,
        })
    }
    async fn post(&self, body: &Value, protocol: &str, session: Option<&str>) -> Result<Response> {
        let mut request = self
            .client
            .post(self.url.clone())
            .header("MCP-Protocol-Version", protocol)
            .json(body);
        if let Some(method) = body["method"].as_str() {
            request = request.header("Mcp-Method", method);
        }
        if let Some(name) = body["params"]["name"].as_str() {
            request = request.header("Mcp-Name", name);
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        Ok(request.send().await?.error_for_status()?)
    }
    async fn watch(&self, number: u64) -> Result<Response> {
        let body = json!({"jsonrpc":"2.0","id":number,"method":"tools/call","params":{
            "name":"client_connection","arguments":{"action":"watch"},"_meta":{
                "progressToken":number,
                "io.modelcontextprotocol/protocolVersion":WATCH_PROTOCOL,
                "io.modelcontextprotocol/clientInfo":{"name":"nickel-mcp-client","version":"0.1.0"},
                "io.modelcontextprotocol/clientCapabilities":{}}}});
        let mut response = self.post(&body, WATCH_PROTOCOL, None).await?;
        let mut parser = Events::default();
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(chunk) = response.chunk().await? {
                for event in parser.push(&chunk)? {
                    if event["method"] == "notifications/progress"
                        && event["params"]["progressToken"] == number
                        && event["params"]["message"] == "Connection watch active"
                    {
                        return Ok::<_, Box<dyn Error + Send + Sync>>(());
                    }
                    if event.get("id").is_some() {
                        return Err("connection watch rejected".into());
                    }
                }
            }
            Err("connection watch ended before readiness".into())
        })
        .await??;
        Ok(response)
    }
}
fn trust_certificate(
    builder: reqwest::ClientBuilder,
    pem: &[u8],
) -> Result<reqwest::ClientBuilder> {
    Ok(builder
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_pem(pem)?))
}
fn validate_url(url: &Url) -> Result<()> {
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("endpoint must not contain credentials, query, or fragment".into());
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("remote endpoints require verified HTTPS".into());
    }
    Ok(())
}

/// Incremental SSE framing with a fixed upper bound, including unterminated lines.
#[derive(Default)]
struct Events {
    pending: Vec<u8>,
    scanned: usize,
    data: Vec<u8>,
}
impl Events {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        if self.pending.len() + self.data.len() + bytes.len() > LIMIT {
            return Err("SSE frame exceeds limit".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut result = Vec::new();
        while let Some(relative) = self.pending[self.scanned..]
            .iter()
            .position(|b| *b == b'\n')
        {
            let end = self.scanned + relative;
            self.scanned = 0;
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let line = line
                .strip_suffix(b"\n")
                .unwrap()
                .strip_suffix(b"\r")
                .unwrap_or(&line[..line.len() - 1]);
            if line.is_empty() {
                if !self.data.is_empty() {
                    result.push(serde_json::from_slice(&self.data)?);
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix(b"data:") {
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data
                    .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            }
        }
        self.scanned = self.pending.len();
        Ok(result)
    }
}
async fn maintain(bridge: Bridge, response: Response) -> Result<()> {
    maintain_every(bridge, response, Duration::from_secs(25)).await
}
async fn maintain_every(bridge: Bridge, mut response: Response, interval: Duration) -> Result<()> {
    let mut sequence = 2;
    loop {
        let rotate = tokio::time::sleep(interval);
        tokio::pin!(rotate);
        loop {
            tokio::select! {
                _ = &mut rotate => break,
                chunk = tokio::time::timeout(Duration::from_secs(5), response.chunk()) => {
                    if chunk??.is_none() { return Err("connection watch ended".into()); }
                }
            }
        }
        // Retain the old stream until its replacement is owner-acknowledged.
        let replacement = bridge.watch(sequence).await?;
        response = replacement;
        sequence = sequence.checked_add(1).ok_or("watch sequence exhausted")?;
    }
}
async fn forward(
    bridge: Bridge,
    body: Value,
    protocol: String,
    session: Option<String>,
    out: mpsc::Sender<Value>,
) -> Result<Option<String>> {
    let mut response = bridge.post(&body, &protocol, session.as_deref()).await?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|h| h.to_str().map(str::to_owned))
        .transpose()?;
    if response.status() == reqwest::StatusCode::ACCEPTED
        || response.status() == reqwest::StatusCode::NO_CONTENT
    {
        return Ok(session);
    }
    let sse = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.starts_with("text/event-stream"));
    let mut bytes = Vec::new();
    let mut events = Events::default();
    while let Some(chunk) = response.chunk().await? {
        if sse {
            for value in events.push(&chunk)? {
                out.send(value).await?;
            }
        } else {
            if bytes.len() + chunk.len() > LIMIT {
                return Err("HTTP response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
    }
    if !sse && !bytes.is_empty() {
        out.send(serde_json::from_slice(&bytes)?).await?;
    }
    Ok(session)
}
async fn read_message(input: &mut (impl tokio::io::AsyncBufRead + Unpin)) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf().await?;
        if available.is_empty() {
            return Ok(line);
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        if line.len() + count > REQUEST_LIMIT {
            return Err("stdio message exceeds limit".into());
        }
        line.extend_from_slice(&available[..count]);
        input.consume(count);
        if line.last() == Some(&b'\n') {
            return Ok(line);
        }
    }
}
type PendingRequests = Arc<Mutex<HashMap<String, (Arc<()>, Option<tokio::task::AbortHandle>)>>>;
#[derive(Clone)]
struct RequestKey {
    id: String,
    generation: Arc<()>,
}
fn forget(pending: &PendingRequests, key: &RequestKey) {
    let mut entries = pending.lock().unwrap();
    if entries
        .get(&key.id)
        .is_some_and(|(generation, _)| Arc::ptr_eq(generation, &key.generation))
    {
        entries.remove(&key.id);
    }
}
struct Incoming {
    body: Value,
    key: Option<RequestKey>,
}
async fn ingest(
    mut input: impl tokio::io::AsyncBufRead + Unpin,
    messages: mpsc::Sender<Incoming>,
    pending: PendingRequests,
) -> Result<()> {
    loop {
        let line = read_message(&mut input).await?;
        if line.is_empty() {
            return Ok(());
        }
        let body: Value = serde_json::from_slice(&line)?;
        if body["method"] == "notifications/cancelled" {
            if let Some(id) = body["params"].get("requestId") {
                let handle = pending
                    .lock()
                    .map_err(|_| "pending requests unavailable")?
                    .remove(&id.to_string());
                if let Some((_, Some(handle))) = handle {
                    handle.abort();
                }
            }
            continue;
        }
        let key = body
            .get("id")
            .filter(|_| body.get("method").is_some() && body["method"] != "initialize")
            .map(|id| RequestKey {
                id: id.to_string(),
                generation: Arc::new(()),
            });
        if let Some(key) = &key {
            let mut requests = pending.lock().map_err(|_| "pending requests unavailable")?;
            if requests.contains_key(&key.id) {
                return Err("duplicate in-flight request ID".into());
            }
            requests.insert(key.id.clone(), (key.generation.clone(), None));
        }
        // Never wait behind saturated tools: EOF/cancellation must remain observable.
        // Overflow closes presence rather than retaining authority behind unbounded input.
        messages
            .try_send(Incoming { body, key })
            .map_err(|_| "stdio request queue exhausted")?;
    }
}
fn completed(
    result: std::result::Result<
        std::result::Result<Result<Option<String>>, tokio::time::error::Elapsed>,
        tokio::task::JoinError,
    >,
) -> Result<()> {
    match result {
        Err(error) if error.is_cancelled() => Ok(()),
        other => {
            other???;
            Ok(())
        }
    }
}
async fn run() -> Result<()> {
    serve(
        Bridge::configured()?,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}
async fn serve(
    bridge: Bridge,
    source: impl tokio::io::AsyncBufRead + Unpin + Send + 'static,
    mut stdout: impl tokio::io::AsyncWrite + Unpin + Send + 'static,
) -> Result<()> {
    let response = bridge.watch(1).await?;
    let mut watch = tokio::spawn(maintain(bridge.clone(), response));
    let (out, mut incoming) = mpsc::channel::<Value>(1);
    let mut writer = tokio::spawn(async move {
        while let Some(value) = incoming.recv().await {
            let mut line = serde_json::to_vec(&value)?;
            line.push(b'\n');
            stdout.write_all(&line).await?;
            stdout.flush().await?;
        }
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });
    let pending = PendingRequests::default();
    let (messages, mut input) = mpsc::channel::<Incoming>(8);
    let mut reader = tokio::spawn(ingest(source, messages, pending.clone()));
    let operation = async {
        let mut protocol = "2025-06-18".to_owned();
        let mut session = None;
        let mut client_info = json!({"name":"nickel-mcp-client","version":"0.1.0"});
        let mut capabilities = json!({});
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let incoming = loop {
                tokio::select! {
                    incoming = input.recv() => match incoming { Some(incoming) => break incoming, None => return std::future::pending::<Result<()>>().await },
                    result = tasks.join_next(), if !tasks.is_empty() => { completed(result.ok_or("request task unavailable")?)?; }
                }
            };
            let Incoming { mut body, key } = incoming;
            if body["method"] == "initialize" {
                client_info = body["params"]["clientInfo"].clone();
                capabilities = body["params"]["capabilities"].clone();
                protocol = body["params"]["protocolVersion"]
                    .as_str()
                    .ok_or("missing protocol version")?
                    .to_owned();
                session = tokio::time::timeout(
                    Duration::from_secs(10),
                    forward(bridge.clone(), body, protocol.clone(), None, out.clone()),
                )
                .await??;
                if let Some(key) = key {
                    forget(&pending, &key);
                }
            } else {
                if body.get("id").is_some() && body.get("method").is_some() {
                    let params = body
                        .as_object_mut()
                        .ok_or("invalid request")?
                        .entry("params")
                        .or_insert_with(|| json!({}));
                    let meta = params
                        .as_object_mut()
                        .ok_or("invalid params")?
                        .entry("_meta")
                        .or_insert_with(|| json!({}));
                    let meta = meta.as_object_mut().ok_or("invalid metadata")?;
                    meta.insert(
                        "io.modelcontextprotocol/protocolVersion".into(),
                        json!(protocol),
                    );
                    meta.insert(
                        "io.modelcontextprotocol/clientInfo".into(),
                        client_info.clone(),
                    );
                    meta.insert(
                        "io.modelcontextprotocol/clientCapabilities".into(),
                        capabilities.clone(),
                    );
                }
                while tasks.len() >= CONCURRENT_REQUESTS {
                    completed(tasks.join_next().await.ok_or("request task unavailable")?)?;
                }
                if key.as_ref().is_some_and(|key| {
                    !pending
                        .lock()
                        .unwrap()
                        .get(&key.id)
                        .is_some_and(|(generation, _)| Arc::ptr_eq(generation, &key.generation))
                }) {
                    continue; // Cancelled before admission: never issue an upstream request.
                }
                let cleanup = pending.clone();
                let cleanup_key = key.clone();
                let bridge = bridge.clone();
                let protocol = protocol.clone();
                let session = session.clone();
                let out = out.clone();
                let handle = tasks.spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(90),
                        forward(bridge, body, protocol, session, out),
                    )
                    .await;
                    if let Some(key) = cleanup_key {
                        forget(&cleanup, &key);
                    }
                    result
                });
                if let Some(key) = key {
                    let mut requests = pending.lock().unwrap();
                    if let Some((generation, slot)) = requests
                        .get_mut(&key.id)
                        .filter(|(generation, _)| Arc::ptr_eq(generation, &key.generation))
                    {
                        let _ = generation;
                        *slot = Some(handle);
                    } else {
                        handle.abort();
                    }
                }
                while let Some(result) = tasks.try_join_next() {
                    completed(result)?;
                }
            }
        }
    };
    let result = tokio::select! {
        result = operation => result,
        result = &mut watch => result.unwrap_or_else(|_| Err("watch task failed".into())),
        result = &mut writer => result.unwrap_or_else(|_| Err("writer task failed".into())),
        result = &mut reader => result.unwrap_or_else(|_| Err("reader task failed".into())),
    };
    watch.abort();
    writer.abort();
    reader.abort();
    result
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        // Deliberately omit error sources: HTTP and JSON errors can contain payloads or URLs.
        eprintln!(
            "Nickel MCP connection ended; verify endpoint, credentials, trust, and local approval."
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_requires_tls_and_no_url_secrets() {
        for good in [
            "http://127.0.0.1:123/mcp",
            "http://[::1]/mcp",
            "https://example.com/mcp",
        ] {
            assert!(validate_url(&Url::parse(good).unwrap()).is_ok());
        }
        for bad in [
            "http://example.com/mcp",
            "http://localhost/mcp",
            "https://user:secret@example.com/mcp",
            "https://example.com/mcp?token=secret",
        ] {
            assert!(validate_url(&Url::parse(bad).unwrap()).is_err());
        }
    }
    #[test]
    fn fragmented_sse_handles_crlf_and_comments() {
        let mut events = Events::default();
        assert!(
            events
                .push(b": keepalive\r\ndata: {\"jsonrpc\":\"2.0\",\r\n")
                .unwrap()
                .is_empty()
        );
        let values = events.push(b"data: \"id\":1}\r\n\r\n").unwrap();
        assert_eq!(values, vec![json!({"jsonrpc":"2.0","id":1})]);
    }
    #[test]
    fn unbounded_sse_line_is_rejected() {
        let mut events = Events::default();
        assert!(events.push(&vec![b'x'; LIMIT]).unwrap().is_empty());
        assert!(events.push(b"x").is_err());
    }
    #[tokio::test]
    async fn replacement_is_ready_before_old_watch_closes() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use tokio::{io::AsyncReadExt, net::TcpListener, sync::Notify};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bridge = Bridge {
            client: Client::builder().no_proxy().build().unwrap(),
            url: Url::parse(&format!("http://{}/mcp", listener.local_addr().unwrap())).unwrap(),
        };
        let replacement_arrived = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first_closed = Arc::new(AtomicBool::new(false));
        let server = tokio::spawn({
            let replacement_arrived = replacement_arrived.clone();
            let release = release.clone();
            let first_closed = first_closed.clone();
            async move {
                let mut handlers = tokio::task::JoinSet::new();
                for number in 1..=2 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let replacement_arrived = replacement_arrived.clone();
                    let release = release.clone();
                    let first_closed = first_closed.clone();
                    handlers.spawn(async move {
                        let mut request = vec![]; let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") { stream.read_exact(&mut byte).await.unwrap(); request.push(byte[0]); }
                        let header = String::from_utf8(request).unwrap();
                        let len: usize = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|value| value.trim().parse().unwrap())).unwrap();
                        let mut body = vec![0;len]; stream.read_exact(&mut body).await.unwrap();
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(request["params"]["arguments"]["action"], "watch");
                        if number == 2 { replacement_arrived.notify_one(); release.notified().await; }
                        let event = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":number,"message":"Connection watch active","progress":0}});
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 1000000\r\n\r\ndata: {event}\n\n").as_bytes()).await.unwrap();
                        let _ = stream.read(&mut byte).await;
                        if number == 1 { first_closed.store(true, Ordering::SeqCst); }
                    });
                }
                while handlers.join_next().await.is_some() {}
            }
        });
        let first = bridge.watch(1).await.unwrap();
        let maintenance = tokio::spawn(maintain_every(bridge, first, Duration::from_millis(25)));
        tokio::time::timeout(Duration::from_secs(2), replacement_arrived.notified())
            .await
            .unwrap();
        assert!(!first_closed.load(Ordering::SeqCst));
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !first_closed.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        maintenance.abort();
        let _ = maintenance.await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn explicit_certificate_and_matching_hostname_are_both_required() {
        const CERT: &[u8] =
            include_bytes!("../../nickel-remote-control/tests/fixtures/localhost-cert.pem");
        const KEY: &[u8] =
            include_bytes!("../../nickel-remote-control/tests/fixtures/localhost-key.pem");
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem(CERT.to_vec(), KEY.to_vec())
            .await
            .unwrap();
        let handle = axum_server::Handle::new();
        let server = tokio::spawn(
            axum_server::from_tcp_rustls(socket, tls)
                .unwrap()
                .handle(handle.clone())
                .serve(
                    axum::Router::new()
                        .route("/", axum::routing::get(|| async { "ok" }))
                        .into_make_service(),
                ),
        );
        let trusted = trust_certificate(Client::builder().no_proxy(), CERT)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            trusted
                .get(format!("https://localhost:{}/", address.port()))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        let untrusted = Client::builder()
            .no_proxy()
            .tls_built_in_root_certs(false)
            .build()
            .unwrap();
        assert!(
            untrusted
                .get(format!("https://localhost:{}/", address.port()))
                .send()
                .await
                .is_err()
        );
        let wrong_host = trust_certificate(
            Client::builder()
                .no_proxy()
                .resolve("wrong.example", address),
            CERT,
        )
        .unwrap()
        .build()
        .unwrap();
        assert!(
            wrong_host
                .get(format!("https://wrong.example:{}/", address.port()))
                .send()
                .await
                .is_err()
        );
        handle.shutdown();
        server.await.unwrap().unwrap();
    }
    async fn saturated_transport(cancel: bool) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bridge = Bridge {
            client: Client::builder().no_proxy().build().unwrap(),
            url: Url::parse(&format!("http://{}/mcp", listener.local_addr().unwrap())).unwrap(),
        };
        let started = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let watch_closed = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn({
            let started = started.clone();
            let closed = closed.clone();
            let watch_closed = watch_closed.clone();
            async move {
                let mut handlers = tokio::task::JoinSet::new();
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let started = started.clone();
                    let closed = closed.clone();
                    let watch_closed = watch_closed.clone();
                    handlers.spawn(async move {
                        let mut request = vec![]; let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") { stream.read_exact(&mut byte).await.unwrap(); request.push(byte[0]); }
                        let header = String::from_utf8(request).unwrap();
                        let len: usize = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|value| value.trim().parse().unwrap())).unwrap();
                        let mut body = vec![0; len]; stream.read_exact(&mut body).await.unwrap();
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        if body["params"]["name"] == "client_connection" {
                            let event = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":1,"message":"Connection watch active","progress":0}});
                            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 1000000\r\n\r\ndata: {event}\n\n").as_bytes()).await.unwrap();
                            let _ = stream.read(&mut byte).await;
                            watch_closed.fetch_add(1, Ordering::SeqCst);
                        } else {
                            started.fetch_add(1, Ordering::SeqCst);
                            // No response: occupy an actual outbound HTTP request until cancelled.
                            let _ = stream.read(&mut byte).await;
                            closed.fetch_add(1, Ordering::SeqCst);
                        }
                    });
                }
            }
        });
        let (mut source, input) = tokio::io::duplex(65536);
        let adapter = tokio::spawn(serve(bridge, BufReader::new(input), tokio::io::sink()));
        for id in 1..=4 {
            source.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"long_running","arguments":{}}})).as_bytes()).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) != 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        source.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"queued","arguments":{}}})).as_bytes()).await.unwrap();
        if cancel {
            source.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":2}}\n").await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while started.load(Ordering::SeqCst) != 5 || closed.load(Ordering::SeqCst) != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                watch_closed.load(Ordering::SeqCst),
                0,
                "cancelling one request must retain connection presence"
            );
        }
        source.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), adapter)
            .await
            .expect("EOF blocked behind saturated tool requests")
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while watch_closed.load(Ordering::SeqCst) != 1
                || closed.load(Ordering::SeqCst) != if cancel { 5 } else { 4 }
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(started.load(Ordering::SeqCst), if cancel { 5 } else { 4 });
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn eof_closes_presence_with_four_active_and_fifth_queued() {
        saturated_transport(false).await;
    }
    #[tokio::test]
    async fn cancellation_bypasses_saturation_and_drops_only_its_http_request() {
        saturated_transport(true).await;
    }
    #[test]
    fn retired_request_cannot_remove_a_reused_id() {
        let pending = PendingRequests::default();
        let old = RequestKey {
            id: "1".into(),
            generation: Arc::new(()),
        };
        let fresh = Arc::new(());
        pending
            .lock()
            .unwrap()
            .insert("1".into(), (fresh.clone(), None));
        forget(&pending, &old);
        assert!(Arc::ptr_eq(&pending.lock().unwrap()["1"].0, &fresh));
    }

    #[tokio::test]
    async fn queued_cancellation_does_not_cancel_a_reused_request_id() {
        let pending = PendingRequests::default();
        let (send, mut receive) = mpsc::channel(8);
        let source = b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":5}}\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n";
        ingest(&source[..], send, pending.clone()).await.unwrap();
        let old = receive.recv().await.unwrap().key.unwrap();
        let fresh = receive.recv().await.unwrap().key.unwrap();
        assert!(!Arc::ptr_eq(
            &pending.lock().unwrap()[&old.id].0,
            &old.generation
        ));
        forget(&pending, &old);
        assert!(Arc::ptr_eq(
            &pending.lock().unwrap()[&fresh.id].0,
            &fresh.generation
        ));
    }
}
