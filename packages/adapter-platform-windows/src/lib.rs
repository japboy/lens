#![forbid(unsafe_code)]

//! Opt-in Windows source adapter for the cooperative development fixture.

pub mod marker_oracle;
pub mod selection_cleanup;

#[cfg(all(windows, target_arch = "x86_64"))]
mod fixture;
#[cfg(all(windows, target_arch = "x86_64"))]
mod process;

#[cfg(all(windows, target_arch = "x86_64"))]
pub use fixture::{Fixture, FixturePlatform, FixtureReadProbe};
