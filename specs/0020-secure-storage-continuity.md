# Secure Storage and Wallet Continuity

## Goal

Make encrypted secret storage a mandatory Nickel session service without invalidating existing application profiles. Switching between Plasma, Nickel X11, and Nickel Wayland must not change the wallet identity seen by applications.

## Platform Contract

- Add a platform-neutral secure-storage adapter for Nickel-owned credentials.
- On Windows, use the existing user profile's DPAPI and Credential Manager facilities. Nickel must not create a competing wallet or move secrets between Windows users or machines.
- On Linux, require a working `org.freedesktop.secrets` provider before declaring the session ready. X11 and Wayland share this service; the selected display backend must not affect wallet selection.
- Record the configured provider and collection identity as durable user-profile state. Discovery must never replace an unavailable or slow provider with an empty one.
- Preserve collection aliases, item attributes, unlock behavior, and the secret values applications previously stored.

Secure storage is required session infrastructure, not an optional capability with a plaintext fallback.

## Session Lifecycle

Nickel must start or connect to the configured provider, unlock the existing collection through an established authentication facility, verify its identity, and only then launch dependent applications. The session must expose a visible locked, unavailable, or setup-required state instead of silently continuing.

Nickel must prevent multiple providers from racing for the Secret Service bus name. It must not retain the login password, launch Chromium or Electron applications with a basic password store, or create a replacement collection merely because the expected provider starts slowly.

Provider failure after login must preserve the configured identity and offer retry or session recovery. It must not cause automatic provider selection.

## Migration

A future Nickel-native Rust provider may be introduced only through an explicit, reversible migration. Migration must:

1. Unlock both providers with user authorization.
2. Copy secrets together with lookup attributes and collection aliases.
3. Verify that every copied item can be retrieved.
4. Switch the configured provider atomically.
5. Retain rollback material until applications have successfully reopened their existing profiles.

Ordinary upgrades and display-backend changes must never initiate migration.

## Verification

- Start Chrome, Chromium, Signal, and another Secret Service client under the existing desktop; then start them under Nickel and confirm sessions and encrypted profiles remain accessible.
- Repeat the test while switching between Nickel Wayland and Nickel X11.
- Test slow, locked, missing, crashed, and conflicting providers without creating an empty substitute or launching dependent applications prematurely.
- Verify Windows secrets remain decryptable across Nickel restarts in the same user profile and fail safely under a different profile.
- Unit-test provider identity persistence, readiness gating, failure recovery, and migration rollback with synthetic adapters.
- Run workspace tests and Clippy with warnings denied.

## Completion

Archive this specification when a user can move an existing application profile into a Nickel session without being logged out, losing encrypted state, or changing its established wallet provider.
