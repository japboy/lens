//! Native-only public receipt rejection and Legacy allocator pairing. The executable starts on the process
//! main thread so dispatch does not depend on libtest worker threads or AppKit.
//!
//! Miri's separate `--lib` boundary suite models the native effects; this executable
//! is excluded from that suite. Missing registrations reject before native AX;
//! an explicit Legacy diagnostic uses invalid PID -1, never a user process.
//! Native code inspects trust without requesting it and returns an owned unavailable
//! JSON response through the production Rust decoder and matching native deallocator.
//! No test registers a real AX observer or selects a user window.

#![forbid(unsafe_code)]

use adapter_platform_macos::MacOsPlatform;
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    authority::{ReadSequence, TargetAuthority, TargetReadKey, TargetReceipt},
    model::{Bounds, ExtractionQuality, SelectedWindow, WindowIdentity, WindowObservableFacts},
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
    let target = TargetAuthority {
        operation_id,
        receipt: TargetReceipt::try_from(Uuid::from_u128(4)).unwrap(),
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
        // This explicitly separate diagnostic still crosses the real C allocator,
        // Rust NativeJson parser and NativeJson::drop -> lens_free_string boundary.
        // -1 is not a process ID: the native Legacy resolver addresses only this
        // invalid process, with no global enumeration or registered-path fallback.
        let legacy = platform
            .extract(
                ExtractionTarget::Legacy(SelectedWindow {
                    identity: WindowIdentity {
                        window_id: 1,
                        bundle_id: "native-boundary-lifecycle-fixture".into(),
                        pid: -1,
                    },
                    facts: WindowObservableFacts {
                        title: "native-boundary-lifecycle-fixture".into(),
                        application_name: "Fixture".into(),
                        frame: Bounds {
                            x: 0.0,
                            y: 0.0,
                            width: 10.0,
                            height: 10.0,
                        },
                    },
                }),
                limits,
            )
            .expect("invalid-process diagnostic returns owned native unavailable JSON");
        assert_eq!(legacy.quality, ExtractionQuality::Unavailable);
        assert!(legacy.resolved_window.is_none());
        assert!(legacy.nodes.is_empty());
        assert!(!legacy.diagnostics.is_empty());
        // A missing opaque registration is rejected before entering native AX.
        let result = platform.extract(
            ExtractionTarget::Registered(TargetReadKey {
                target,
                sequence: ReadSequence::FIRST,
            }),
            limits,
        );
        assert!(matches!(result, Err(PlatformError::Operation(_))));

        // Observation also rejects the absent receipt before native start.
        let request = ObservationRequest {
            target,
            context_id: Uuid::from_u128(2),
            source_registration_id: Uuid::from_u128(3),
            observer_epoch: NonZeroU64::new(1).expect("finite nonzero epoch"),
        };
        assert!(matches!(
            platform.observe(request),
            Err(PlatformError::Operation(_))
        ));

        // Public release is fail-closed at the absent Rust registry. The native
        // release implementation is covered separately by the native fixtures.
        assert!(matches!(
            platform.release_target(target),
            Err(PlatformError::Operation(_))
        ));
        assert!(matches!(
            platform.release_operation(operation_id),
            Err(PlatformError::Operation(_))
        ));
    }
    println!(
        "Native public boundary: {CYCLES} cycles passed (Legacy real allocator pairing; absent receipts rejected for extraction, observation and release)"
    );
}
