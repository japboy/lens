#ifndef LENS_NATIVE_H
#define LENS_NATIVE_H

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*LensPickerCallback)(const char *json, void *context);
typedef void (*LensWindowTransitionCallback)(bool completed, void *context);
typedef void (*LensWindowObservationCallback)(const char *json, void *context);

/* Main-thread borrowed NSStatusItem. Validates all titles/count before setting any tooltip. */
bool lens_set_menu_presentation(void *statusItemPointer, size_t submenuIndex,
                            const char *submenuTitle, const char *itemsJSON);

bool lens_accessibility_is_trusted(void);
bool lens_accessibility_request_trust(void);

/* Localized short date/time; release the returned string with lens_free_string. */
char *lens_format_short_datetime(double unix_seconds);

/* Resolve the current application appearance to four sRGB bytes on the main thread. */
bool lens_window_background_rgba(uint8_t *rgba);

typedef struct {
    uint8_t control_surface[4];
    uint8_t window_surface[4];
    uint8_t button_fill[4];
    uint8_t button_pressed_fill[4];
    uint8_t separator[4];
    uint8_t primary_button_fill[4];
    uint8_t primary_button_foreground[4];
    bool colors_available;
    bool increase_contrast;
    bool reduce_transparency;
    bool window_active;
} LensControlPalette;

/* Pure Lens policy for opaque sRGB accent input; checks contrast after byte quantization. */
bool lens_primary_button_colors(const double *accentRGB, bool increaseContrast,
                                uint8_t *fillRGBA, uint8_t *foregroundRGBA);

/* Main-thread snapshot; null window uses NSApp.effectiveAppearance before creation.
 * Color unavailability clears colors_available without erasing display/window state.
 * False means invalid infrastructure (thread, application or output pointer). */
bool lens_control_palette(void *windowPointer, LensControlPalette *palette);
/* Borrow live NSWindow/WKWebView on the main thread; window owns observer teardown. */
bool lens_observe_control_palette(void *windowPointer, void *webviewPointer);

/* Main-thread modal alert: 1 explicit confirmation, 0 cancellation/dismissal, -1 error. */
int32_t lens_confirm_destructive_action(const char *title, const char *message,
                                      const char *confirmLabel, const char *cancelLabel);

/* Clip all floating-window content layers with the constructor-owned radius. */
bool lens_configure_floating_window_radius(void *windowPointer, double radius);
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
    const char *applicationName,
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
