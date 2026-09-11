# Nickel MCP stdio adapter

`nickel-mcp-client` lets a stdio MCP host use Nickel's authenticated HTTP service while the adapter maintains connection presence during quiet periods. Build with `cargo build -p nickel-mcp-client`. Configure the host to execute this binary and supply these environment variables through its private configuration:

| Variable | Meaning |
| --- | --- |
| `NICKEL_MCP_URL` | Complete endpoint, such as `http://127.0.0.1:42639/mcp` or `https://desktop.example:42639/mcp` |
| `NICKEL_MCP_CLIENT` | Issued client identity |
| `NICKEL_MCP_TOKEN` | Issued bearer capability; keep private |
| `NICKEL_MCP_CA_FILE` | Required for HTTPS: PEM certificate to trust for this host |

Obtain the identity and capability through Nickel's local identity/pairing workflow. The adapter does not grant permissions. Local approval remains necessary, and watches never renew or extend leases.

For HTTPS, verify the server's SHA-256 host fingerprint with the desktop owner over a trusted channel and set the 64-digit hexadecimal value in `NICKEL_MCP_HOST_FINGERPRINT`. The adapter pins the exact leaf certificate presented in the TLS handshake, including when the endpoint is a numeric IP that is not named in the certificate. It still verifies the server's handshake signature. A different or missing certificate is rejected.

Alternatively, set `NICKEL_MCP_CA_FILE` to a self-signed host certificate or dedicated issuing CA after verifying it with the desktop owner. This mode trusts only that explicit certificate authority, disables built-in public roots, and also requires the endpoint hostname or IP to appear in the certificate. Hostname verification is bypassed only when an exact `NICKEL_MCP_HOST_FINGERPRINT` pin is configured. There is no general insecure certificate override. HTTP is accepted only for a literal loopback address; `http://localhost` is rejected so DNS or hosts-file resolution cannot redirect plaintext credentials; remote URLs must use HTTPS. Redirects and environment HTTP proxies are disabled. URLs cannot contain credentials, queries, or fragments. Stderr reports a generic failure without request, response, credential, or URL details.

The adapter waits for owner-confirmed watch readiness before forwarding stdin. Every 25 seconds it opens a replacement watch and waits for readiness before dropping the old stream. Idle connections therefore remain present without periodic user messages. Loss of the watch, its keepalives, stdin, or a stdout write failure ends the adapter and closes its connections. It deliberately does not silently reconnect after a loss: Nickel must enforce pending-request cancellation, non-resumable lease revocation, and resumable lease policy. Restarting the adapter opens a new watch and can resume only authority Nickel permits.

Transport limits: four concurrent ordinary requests, eight queued stdin messages, one queued output message, 1 MiB per stdio request and 96 MiB per JSON response/SSE event (enough for Nickel’s maximum PNG capture including base64), five seconds for connect/watch readiness and keepalive gaps, ten seconds for initialization, and 90 seconds per ordinary HTTP request. The adapter forwards POST response SSE notifications and JSON-RPC responses; it does not implement a separate GET notification channel or automatic HTTP retries. Tools requiring longer streams need a client that implements Nickel's connection-watch contract directly. This adapter currently targets Nickel's stateless HTTP MCP endpoint; general third-party MCP server compatibility is not claimed.

Stdin ingestion runs independently of saturated tools. EOF closes presence promptly even when all four HTTP slots are occupied and another request is queued. A full stdin queue ends the adapter and closes presence instead of indefinitely delaying EOF or allocating more memory. Duplicate in-flight request IDs are rejected.

`notifications/cancelled` bypasses HTTP admission: queued requests are removed before dispatch, and an active request's HTTP future/response stream is dropped. Nickel's stateless HTTP transport cancels that request's server context on stream loss; an independent cancellation POST cannot address another stateless POST's context, so the adapter does not forward it as if it could. Cancellation does not undo already completed side effects and does not promise cancellation semantics for arbitrary third-party/stateful MCP servers. MCP initialization is not cancellable. Cancelling an ordinary request retains the connection watch and unrelated requests.
