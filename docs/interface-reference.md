# hat-local-inference-manager interface reference

Use the [usage guide](getting-started.md) for the first steps. This reference preserves the current interface details and operational limits. Run command examples from the repository root, after preparing the exact declared dependencies and registered configuration.

## Acceptance

- Exact request, artifact digest, operation and binding must match.
- A real model must be acquired, started, used and stopped through the released
  consumer before deployment is marked verified. The Zixcel small-data demo is
  not model-execution evidence.
- Cancellation, lease expiry, concurrent invocations and restart must not leave
  orphan processes, duplicate effects or an incorrect completed state.
- Missing delivery/runtime configuration remains an explicit unavailable result.

Use the owner-local registry configuration for `cargo test --locked --offline`.
Do not use cross-repository source patches to satisfy missing artifacts.
