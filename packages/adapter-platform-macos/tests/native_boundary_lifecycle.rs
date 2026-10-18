//! Native-only pairing of the safe public adapter API with real native JSON allocation,
//! parsing and release. The executable starts on the process main thread so native
//! dispatch does not depend on libtest worker threads or a running AppKit event loop.
//!
//! Miri's separate `--lib` boundary suite models the native effects; this executable
//! deliberately calls real Foundation/dispatch/allocator entry points and is excluded
//! from that suite. Missing process-local registrations never inspect user windows,
//! request Accessibility permission or register a real AX observer.

#![forbid(unsafe_code)]

use adapter_platform_macos::MacOsPlatform;
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    model::{ExtractionQuality, WindowIdentity},
    observation::{Observation, ObservationRequest},
    selection::TargetSelection,
    ExtractionLimits, PlatformError,
};
use std::num::NonZeroU64;
use uuid::Uuid;

fn main() {
    assert!(objc2::MainThreadMarker::new().is_some());
    let platform = MacOsPlatform;
    // Every executable has its own empty registration registry. These fixed IDs
    // cannot select or address an operation in another application process.
    let operation_id = Uuid::from_u128(1);
    let identity = WindowIdentity {
        window_id: 1,
        bundle_id: "native-boundary-lifecycle-fixture".into(),
        pid: 1,
    };
    let limits = ExtractionLimits {
        max_nodes: 8,
        max_text_bytes: 64,
        max_resource_refs: 2,
        max_resource_uri_bytes: 64,
        max_total_resource_uri_bytes: 128,
    };
    const CYCLES: usize = 4;
    for _ in 0..CYCLES {
        // The unavailable payload is allocated by LensCopyJSONString and parsed
        // and released by the real production NativeJson owner.
        let result = platform
            .extract(
                ExtractionTarget::Registered {
                    operation_id,
                    identity: identity.clone(),
                },
                limits,
            )
            .expect("missing registration returns a native unavailable payload");
        assert_eq!(result.quality, ExtractionQuality::Unavailable);
        assert!(result.resolved_window.is_none());
        assert!(result.nodes.is_empty());
        assert!(!result.diagnostics.is_empty());

        // The native start-failure payload and started=false exercise the real
        // JSON deallocator and the Rust callback-context cleanup together.
        let request = ObservationRequest {
            operation_id,
            context_id: Uuid::from_u128(2),
            source_registration_id: Uuid::from_u128(3),
            observer_epoch: NonZeroU64::new(1).expect("finite nonzero epoch"),
            identity: identity.clone(),
        };
        assert!(matches!(
            platform.observe(request),
            Err(PlatformError::Operation(_))
        ));

        // Public release is fail-closed at the absent Rust registry. The native
        // release implementation is covered separately by the native fixtures.
        assert!(matches!(
            platform.release_target(operation_id, identity.window_id),
            Err(PlatformError::Operation(_))
        ));
        assert!(matches!(
            platform.release_operation(operation_id),
            Err(PlatformError::Operation(_))
        ));
    }
    println!(
        "Native Rust/C boundary pairing: {CYCLES} cycles passed (unavailable extraction, observation start failure, absent public release)"
    );
}
