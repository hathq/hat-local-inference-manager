# hat-local-inference-manager

Request local model-resource operations through a bounded role invocation.

## What you can do

- Validate model-operation bindings and placements.
- Coordinate worker lifecycle through the declared supervisor.

## Current scope

A real model must be acquired, used and stopped before production readiness is claimed. Small-data demonstrations are not model-execution evidence.

Package distribution is not activated by this documentation. Use the checked-in source and the declared dependency versions; published availability must be verified separately.

## Getting started

Install Rust 1.97 or newer and make the declared dependencies available. Resolve declared third-party dependencies from public registries and source-only managed components from their fixed public Git revision. No private registry or adjacent checkout is part of the supported OSS source path. Run from this repository:

```sh
cargo test --locked
```

## Documentation and source

[Interface reference](docs/interface-reference.md)

[Usage guide](docs/getting-started.md)

[Implementation and public interfaces](src) · [Verification cases](tests) · [Contributing](CONTRIBUTING.md) · [Security reporting](SECURITY.md) · [License](LICENSE) · [Attribution notices](NOTICE)

[OSS responsibility and source completion](docs/oss-source-contract.md) defines the supported package boundary and required acceptance. It does not declare unexecuted source, distribution or production checks complete.

Dependency updates: [repository policy](docs/dependency-updates.md).

[Independent HAT build and release](docs/hat-release.md).

## Operational catalog source contract

The current source package has no linguistic clause frames. Its catalog and
foundation dependency digests follow the declared HatSpec source dependency.
This source change does not replace previously signed release bytes. A new
immutable release identity and consumer acceptance are required before distributing
the changed package; language meaning/adoption remains with sem-lang.
