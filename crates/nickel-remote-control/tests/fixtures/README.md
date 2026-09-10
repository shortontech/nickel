These certificate and private-key files are public test fixtures for the local HTTPS transport
test. They identify only `localhost` and `127.0.0.1`. Never use them for a running Nickel session.

The test installs the fixture certificate into its own in-memory trust store, verifies the hostname,
compares the advertised SHA-256 fingerprint, and checks that listener shutdown releases its port.
