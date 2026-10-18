#![deny(clippy::all)]

//! SLIM Core — domain types, invariants, and service trait definitions.
//!
//! This crate contains no I/O or database dependencies. It defines the
//! vocabulary types shared across all other SLIM crates and the trait
//! interfaces (ports) that storage and application layers implement.

pub mod error;
pub mod label;
pub mod model;
pub mod service;
