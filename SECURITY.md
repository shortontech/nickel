# Security policy

Nickel is a desktop shell and, on Linux, a Wayland compositor. It handles input,
window management, local session control, capability-constrained shell packages,
and other security-sensitive platform integration. Please report suspected
vulnerabilities privately.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting flow to [report a security
advisory](https://github.com/shortontech/nickel/security/advisories/new).

Include as much of the following as is practical:

- the affected version, commit, platform, and backend;
- the security boundary or capability involved;
- reproduction steps or a minimal proof of concept;
- the impact you believe is possible; and
- any suggested mitigation.

Do not open a public issue for an unpatched vulnerability. Do not include real
credentials, tokens, private user data, or destructive payloads in a report.
Use synthetic test data wherever possible.

The maintainer will acknowledge the report through the private advisory,
investigate it, and coordinate disclosure and a fix based on severity and
available maintainer capacity. Please allow time for a safe fix to reach
supported release channels before publishing details.

## Supported versions

Security fixes target the latest tagged release and the current `master` branch.
Older releases may not receive patches. Because Nickel is under active
development, reporters may be asked to confirm an issue against a current build.

Experimental or incomplete features are still in scope when they cross a trust
boundary, expose protected information, bypass a declared capability, or permit
unauthorized input or platform operations.

## Scope

Examples of security-relevant reports include:

- bypassing plugin capability or package-admission checks;
- escaping the JSX host into unrestricted native or filesystem access;
- unauthorized session control, input injection, window control, or capture;
- disclosure of protected surfaces, credentials, tokens, or private content;
- authentication, authorization, or approval-flow bypasses;
- memory-safety issues reachable through supported inputs; and
- unsafe installer, update, or shell-registration behavior.

Ordinary crashes, unsupported hardware, rendering defects, and performance
problems without a security impact belong in the public issue tracker. See
[SUPPORT.md](SUPPORT.md) for routing guidance.
