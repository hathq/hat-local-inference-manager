# hat-local-inference-manager

Request local model-resource operations through a bounded role invocation.

## What you can do

- Validate model-operation bindings and placements.
- Coordinate worker lifecycle through the declared supervisor.

## Current scope

A real model must be acquired, used and stopped before production readiness is claimed. Small-data demonstrations are not model-execution evidence.

Package distribution is not activated by this documentation. Use the checked-in source and the declared dependency versions; published availability must be verified separately.

## Getting started

Install Rust 1.97 or newer and make the declared dependencies available. Use the configured private registry when a dependency is not distributed publicly. Run from this repository:

```sh
cargo test --locked
```

## Documentation and source

[Interface reference](docs/interface-reference.md)

[Usage guide](docs/getting-started.md)

[Implementation and public interfaces](src) · [Verification cases](tests) · [Contributing](CONTRIBUTING.md) · [Security reporting](SECURITY.md) · [License](LICENSE) · [Attribution notices](NOTICE)
