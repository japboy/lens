//! Canonical projection re-export. The projection itself is domain-owned.
//!
//! This module once carried an application-owned observation, delivery and Agent
//! authority state machine. Live synchronisation is implemented in `observation` instead,
//! and every type declared here had become unreachable, so only the re-export remains.

pub use domain::projection::*;
