# Public HAT build and release template

Adopted by the eight explicitly designated public HAT repositories. Each owner
builds and tests its own Linux x86_64 worker with the locked Cargo graph. The HAT
version comes only from `hat.package.json`; `hat/v<version>` is the release tag.
A main update or PR produces an immutable Actions artifact, not a GitHub Release.

The bundle contains the tracked public source, licenses/notices and the worker.
`hat.package.json`, `release.json` and `SHA256SUMS` bind identity, exact source and
bytes. This is build evidence, not a signed installation catalog or permission to
execute. Source visibility does not imply that a module is currently released.

Before publication is enabled, configure the owner's native `hat-public`
Environment with reviewers and a `hat/v*` tag policy. Set `HAT_RELEASE_ENABLED`
to `true` only after reviewing the first candidate's public scope and hashes.
Publication fails unless the tag matches the descriptor, resolves current main,
main CI is successful, numeric repository identity/public visibility match, and
native review is configured. The approved job retrieves the exact build artifact
ID, attests its bytes and creates a release without replacing an existing release
or creating a missing tag. No registry token or long-lived signing key is used.

The Registry may index these stable releases. Hatter must still verify its pinned
signed catalog and exact package bytes before inert installation. Artifact
attestation does not replace catalog signing, grant permissions, start a worker,
activate a role or execute downloaded scripts. Linux build availability does not
claim Windows/macOS or immutable installed-product acceptance.

No release-enabling variable, tag, secret, Environment, DNS or deployment is
created by this source preparation. Runtime/signature acceptance remains #264.
