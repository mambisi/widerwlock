# Wide (Partitioned) Read Write Lock

[![Latest version](https://img.shields.io/crates/v/widerwlock.svg)](https://crates.io/crates/widerwlock)
[![Documentation](https://docs.rs/widerwlock/badge.svg)](https://docs.rs/widerwlock)
![Minimum rustc version](https://img.shields.io/badge/rustc-1.28+-yellow.svg)

This crate implements an enterprise-grade read-write lock, typically used for large data structures.
The lock is internally 8x partitioned, which allows it to scale much better with a large number of readers,
as compared to `std::sync::RwLock`.

Even though this is a large lock, it has better performance characteristic for uncontended single reader
or single writer lock than `std::sync::RwLock`.

The lock uses a contiguous 576 byte heap area to store its state, so it's not a light-weight lock.
If you have a complex data structure that holds a GB of data, this would be an appropriate lock.

An interesting feature of this lock, beside its performance, is its Read->Write upgrade mechanics. The `ReadGuard` allows an
upgrade to a write-lock and informs the user whether the upgrade succeeded atomically or not. This enables
the following pattern:
- Read to see if the data structure is in a particular state (e.g. contains a value).
  - if not, upgrade to a write lock
  - if upgrade was not atomic, perform the (potentially expensive) read again
  - write to the structure if needed

- [Documentation](https://docs.rs/widerwlock)

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
widerwlock = "0.5"
```

and this to your crate root:

```rust
extern crate widerwlock;
```

## Rust Version Support

The minimum supported Rust version is 1.28 due to use of allocator api.