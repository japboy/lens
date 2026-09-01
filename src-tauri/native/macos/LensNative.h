#ifndef LENS_NATIVE_H
#define LENS_NATIVE_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*LensPickerCallback)(const char *json, void *context);
typedef void (*LensWindowTransitionCallback)(bool completed, void *context);
typedef void (*LensWindowObservationCallback)(const char *json, void *context);

bool lens_accessibility_is_trusted(void);
bool lens_accessibility_request_trust(void);

bool lens_present_window_from_screen_right(void *windowPointer);
bool lens_dismiss_window_to_screen_right(
    void *windowPointer,
    LensWindowTransitionCallback callback,
    void *context
);
bool lens_transition_window_frame(
    void *windowPointer,
    double topLeftDeltaX,
    double topLeftDeltaY,
    double contentWidth,
    double contentHeight,
    LensWindowTransitionCallback callback,
    void *context
);

/* Legacy one-shot picker. It serializes metadata and owns no selected source. */
bool lens_present_window_picker(LensPickerCallback callback, void *context);

/*
 * Operation-scoped picker. A successful callback stores the exact picker-returned
 * SCWindow under (operationID, windowID) until an explicit window/operation release.
 */
bool lens_present_window_picker_for_operation(
    const char *operationID,
    LensPickerCallback callback,
    void *context
);

/* Legacy one-shot capture, retained for non-picker validation inputs. */
char *lens_capture_window_regions_json(
    uint32_t windowID,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes
);

/* Captures only the exact SCWindow retained by the operation registry. */
char *lens_capture_registered_window_regions_json(
    const char *operationID,
    uint32_t windowID,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes
);

/* Legacy heuristic extraction, retained for explicit validation targets. */
char *lens_extract_window_json(
    int32_t pid,
    const char *selectedTitle,
    double selectedX,
    double selectedY,
    double selectedWidth,
    double selectedHeight,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
);

/*
 * Resolves the operation's retained SCWindow to one AXWindow at most once, then
 * traverses only that retained AXUIElement for every subsequent refresh.
 */
char *lens_extract_registered_window_json(
    const char *operationID,
    uint32_t windowID,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
);

/*
 * Installs one source-scoped AXObserver on the main run loop. Every callback is
 * a finite JSON event containing operation/context/source/epoch/window authority.
 * `startedOut` is required and becomes true only after this call transfers observer
 * ownership to the caller; it remains false for every terminal error.
 */
char *lens_start_window_observation_json(
    const char *operationID,
    const char *contextID,
    const char *sourceRegistrationID,
    uint64_t observerEpoch,
    uint32_t windowID,
    LensWindowObservationCallback callback,
    void *context,
    bool *startedOut
);

/*
 * Stops callbacks and releases AX/run-loop objects. The operation-scoped exact
 * SCWindow remains retained so the same fixed target can resume observation.
 */
bool lens_stop_window_observation(
    const char *operationID,
    const char *sourceRegistrationID
);

/* Releases one picker entry, its observer/AX objects, and its retained SCWindow. */
bool lens_release_registered_window(const char *operationID, uint32_t windowID);

/* Releases every retained picker/source object owned by an operation. */
bool lens_release_window_operation(const char *operationID);

void lens_free_string(char *value);

#if defined(LENS_NATIVE_TESTING)
/* Probe-only injection into the same registry path used by picker completion. */
bool lens_test_store_picker_window(const char *operationID, void *screenCaptureWindow);
#endif

#ifdef __cplusplus
}
#endif

#endif
