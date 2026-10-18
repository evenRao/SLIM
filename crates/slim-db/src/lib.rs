#![deny(clippy::all)]

//! SLIM Database — SQLite repositories, migrations, and transaction logic.

pub mod connection;
pub mod migrate;
pub mod repo;
