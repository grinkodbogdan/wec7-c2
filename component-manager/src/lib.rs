//! Flare component manager core.
//!
//! This crate holds the policy logic of the component manager as plain,
//! host-testable `no_std` code. Today it is used at build time (to validate
//! boot compositions and generate the kernel's boot table) and by the
//! developer tooling; in milestone M2 the same code runs inside the
//! user-space component manager.
//!
//! * [`manifest`] — what a component declares about itself.
//! * [`resolve`] — wiring required interfaces to providers, start order,
//!   dependents.
//! * [`lifecycle`] — the component state machine.
//! * [`swap`] — planning a hot-swap / restart of a component.
//! * [`binding`] — matching described hardware to driver components.
//! * [`boot`] — boot compositions: which components start and with exactly
//!   which capabilities.

#![no_std]

extern crate alloc;

pub mod binding;
pub mod boot;
pub mod lifecycle;
pub mod manifest;
pub mod resolve;
pub mod swap;
pub mod version;

pub use manifest::Manifest;
