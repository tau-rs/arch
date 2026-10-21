//! Adapters. `http` drives the domain; the rest are driven by it and talk to externals.

pub mod carrier;
pub mod email;
pub mod http;
pub mod memory;
pub mod postgres;
pub mod stripe;
