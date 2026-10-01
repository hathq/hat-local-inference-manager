# Local Inference Manager HAT

An optional HAT exposing local model-resource operations through the common HAT
invocation boundary. Zixcel owns model catalogs, artifact validation and runtime
operations; Crowsi owns delivery and authorized egress. This HAT does not own the
language, user credentials, model weights or provider implementation.

`hat-local-inference-manager-worker` accepts exact binding, placement and state
arguments. Its supervisor owns the process lock, bounded worker concurrency,
heartbeat and shutdown. Role count does not imply one model process per role.

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
