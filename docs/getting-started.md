# Using hat-local-inference-manager

Request local model-resource operations through a bounded role invocation.

## Before you start

A real model must be acquired, used and stopped before production readiness is claimed. Small-data demonstrations are not model-execution evidence.

## First steps

Run from the repository root:

```sh
cargo test --locked
```

## How to assess the result

- Validate model-operation bindings and placements.
- Coordinate worker lifecycle through the declared supervisor.

A passing source-level check establishes only what that check observes. Keep missing configuration, unavailable services and unverified deployment paths visible.

## Continue reading

[Repository overview](../README.md)
