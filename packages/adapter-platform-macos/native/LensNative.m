#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <math.h>

#import "LensNative.h"

static const NSTimeInterval LensWindowFrameTransitionDuration = 0.18;

static NSDictionary *LensFrameDictionary(CGRect frame);

static void LensPerformSyncOnMainThread(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
        return;
    }
    dispatch_sync(dispatch_get_main_queue(), block);
}

static void LensPerformOnMainThread(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
        return;
    }
    dispatch_async(dispatch_get_main_queue(), block);
}

bool lens_window_background_rgba(uint8_t *rgba) {
    if (![NSThread isMainThread] || rgba == NULL || NSApp == nil) {
        return false;
    }
    __block bool resolved = false;
    [NSApp.effectiveAppearance performAsCurrentDrawingAppearance:^{
        NSColor *color = [NSColor.windowBackgroundColor colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
        if (color == nil) {
            return;
        }
        rgba[0] = (uint8_t)lround(color.redComponent * 255.0);
        rgba[1] = (uint8_t)lround(color.greenComponent * 255.0);
        rgba[2] = (uint8_t)lround(color.blueComponent * 255.0);
        rgba[3] = (uint8_t)lround(color.alphaComponent * 255.0);
        resolved = true;
    }];
    return resolved;
}

static char *LensCopyJSONString(id object) {
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:&error];
    if (data == nil) {
        NSString *message = error.localizedDescription ?: @"JSON serialization failed";
        NSDictionary *fallback = @{ @"status": @"error", @"message": message };
        data = [NSJSONSerialization dataWithJSONObject:fallback options:0 error:nil];
    }
    char *result = malloc(data.length + 1);
    if (result == NULL) {
        return NULL;
    }
    memcpy(result, data.bytes, data.length);
    result[data.length] = '\0';
    return result;
}

static NSString *LensStringOrEmpty(NSString *value) {
    return value ?: @"";
}

static NSScreen *LensScreenContainingFrame(NSRect frame) {
    NSScreen *bestScreen = nil;
    CGFloat bestIntersectionArea = -1.0;
    for (NSScreen *screen in NSScreen.screens) {
        NSRect intersection = NSIntersectionRect(frame, screen.frame);
        CGFloat area = MAX(NSWidth(intersection), 0.0) * MAX(NSHeight(intersection), 0.0);
        if (area > bestIntersectionArea) {
            bestScreen = screen;
            bestIntersectionArea = area;
        }
    }
    return bestScreen;
}

static NSRect LensFrameBeyondScreenRight(NSRect frame, NSScreen *screen) {
    NSRect outsideFrame = frame;
    outsideFrame.origin.x = NSMaxX(screen.frame);
    return outsideFrame;
}

bool lens_present_window_from_screen_right(void *windowPointer) {
    if (windowPointer == NULL) {
        return false;
    }

    __block BOOL presented = NO;
    dispatch_block_t presentation = ^{
        NSWindow *window = (__bridge NSWindow *)windowPointer;
        NSRect settledFrame = window.frame;
        NSScreen *screen = window.screen ?: LensScreenContainingFrame(settledFrame);
        if (screen == nil) {
            return;
        }

        if (NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
            [window setFrame:settledFrame display:YES];
            [window makeKeyAndOrderFront:nil];
            presented = YES;
            return;
        }

        NSRect entranceFrame = LensFrameBeyondScreenRight(settledFrame, screen);
        [window setFrame:entranceFrame display:YES];
        [window makeKeyAndOrderFront:nil];
        [window setFrame:settledFrame display:YES animate:YES];
        if (!NSEqualRects(window.frame, settledFrame)) {
            [window setFrame:settledFrame display:YES];
        }
        presented = YES;
    };
    if ([NSThread isMainThread]) {
        presentation();
    } else {
        dispatch_sync(dispatch_get_main_queue(), presentation);
    }
    return presented;
}

bool lens_dismiss_window_to_screen_right(
    void *windowPointer,
    LensWindowTransitionCallback callback,
    void *context
) {
    if (windowPointer == NULL || callback == NULL) {
        return false;
    }

    dispatch_block_t dismissal = ^{
        NSWindow *window = (__bridge NSWindow *)windowPointer;
        NSScreen *screen = window.screen ?: LensScreenContainingFrame(window.frame);
        if (screen == nil) {
            callback(false, context);
            return;
        }

        if (NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
            [window orderOut:nil];
            callback(true, context);
            return;
        }

        NSRect exitFrame = LensFrameBeyondScreenRight(window.frame, screen);
        [NSAnimationContext runAnimationGroup:^(NSAnimationContext *animationContext) {
            animationContext.duration = [window animationResizeTime:exitFrame];
            [[window animator] setFrame:exitFrame display:YES];
        } completionHandler:^{
            if (!NSEqualRects(window.frame, exitFrame)) {
                [window setFrame:exitFrame display:YES];
            }
            [window orderOut:nil];
            callback(true, context);
        }];
    };
    if ([NSThread isMainThread]) {
        dismissal();
    } else {
        dispatch_async(dispatch_get_main_queue(), dismissal);
    }
    return true;
}

bool lens_transition_window_frame(
    void *windowPointer,
    double topLeftDeltaX,
    double topLeftDeltaY,
    double contentWidth,
    double contentHeight,
    LensWindowTransitionCallback callback,
    void *context
) {
    if (windowPointer == NULL || callback == NULL ||
        !isfinite(topLeftDeltaX) || !isfinite(topLeftDeltaY) ||
        !isfinite(contentWidth) || !isfinite(contentHeight) ||
        contentWidth <= 0.0 || contentHeight <= 0.0) {
        return false;
    }

    dispatch_block_t transition = ^{
        NSWindow *window = (__bridge NSWindow *)windowPointer;
        NSRect currentFrame = window.frame;
        NSRect targetFrame = [window frameRectForContentRect:NSMakeRect(
            0.0,
            0.0,
            contentWidth,
            contentHeight
        )];
        targetFrame.origin.x = NSMinX(currentFrame) + topLeftDeltaX;
        targetFrame.origin.y = NSMaxY(currentFrame) - topLeftDeltaY - NSHeight(targetFrame);

        if (NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion ||
            NSEqualRects(currentFrame, targetFrame)) {
            [window setFrame:targetFrame display:YES];
            callback(true, context);
            return;
        }

        [NSAnimationContext runAnimationGroup:^(NSAnimationContext *animationContext) {
            animationContext.duration = LensWindowFrameTransitionDuration;
            [[window animator] setFrame:targetFrame display:YES];
        } completionHandler:^{
            if (!NSEqualRects(window.frame, targetFrame)) {
                [window setFrame:targetFrame display:YES];
            }
            callback(true, context);
        }];
    };
    if ([NSThread isMainThread]) {
        transition();
    } else {
        dispatch_async(dispatch_get_main_queue(), transition);
    }
    return true;
}

static const float LensAccessibilityMessagingTimeoutSeconds = 1.0f;
enum { LensMaximumObservationRegistrations = 6 };

typedef NS_ENUM(NSUInteger, LensNativeObservationKind) {
    LensNativeObservationKindWindowTitleChanged,
    LensNativeObservationKindWindowMoved,
    LensNativeObservationKindWindowResized,
    LensNativeObservationKindWindowDestroyed,
    LensNativeObservationKindApplicationChanged,
};

@class LensNativeWindowSource;

typedef struct {
    void *_Nullable owner;
    uint64_t observerEpoch;
    LensNativeObservationKind kind;
    AXUIElementRef _Nullable element;
    CFStringRef _Nullable notification;
    BOOL registered;
} LensNativeObservationRegistration;

@interface LensNativeWindowSource : NSObject {
    AXUIElementRef _Nullable _applicationElement;
    AXUIElementRef _Nullable _windowElement;
    AXObserverRef _Nullable _observer;
    LensNativeObservationRegistration
        _registrations[LensMaximumObservationRegistrations];
    NSUInteger _registrationCount;
}
@property(nonatomic, copy) NSString *operationID;
@property(nonatomic, copy) NSString *receipt;
@property(nonatomic, assign) uint32_t selectionOrdinal;
@property(nonatomic, assign) uint32_t windowID;
@property(nonatomic, assign) pid_t pid;
@property(nonatomic, copy) NSString *pickerTitle;
@property(nonatomic, copy) NSString *pickerApplicationName;
@property(nonatomic, assign) CGRect pickerFrame;
@property(nonatomic, strong, nullable) SCWindow *screenWindow;
@property(nonatomic, copy, nullable) NSString *contextID;
@property(nonatomic, copy, nullable) NSString *sourceRegistrationID;
@property(nonatomic, assign) uint64_t observerEpoch;
@property(nonatomic, assign) LensWindowObservationCallback observationCallback;
@property(nonatomic, assign) void *observationContext;
@property(nonatomic, assign) BOOL acceptingCallbacks;
@property(nonatomic, assign) BOOL runLoopSourceAttached;
@property(nonatomic, assign) double initialResolutionScore;
- (instancetype)initWithOperationID:(NSString *)operationID window:(SCWindow *)window;
- (BOOL)ensureResolvedWindowWithDiagnostics:(NSMutableArray<NSString *> *)diagnostics;
- (NSDictionary *)startObservationWithContextID:(NSString *)contextID
                           sourceRegistrationID:(NSString *)sourceRegistrationID
                                  observerEpoch:(uint64_t)observerEpoch
                                       callback:(LensWindowObservationCallback)callback
                                callbackContext:(void *)callbackContext;
- (void)stopObservation;
- (void)releaseAllRetainedObjects;
- (AXUIElementRef _Nullable)copyResolvedWindow;
@end

static BOOL LensAXFrame(AXUIElementRef element, CGRect *frameOut);
static id LensGeometryObservation(CGRect before, CGRect after);

static NSMutableDictionary<NSString *, LensNativeWindowSource *> *LensWindowSourceRegistry;
static NSMutableDictionary<NSString *, LensNativeWindowSource *> *LensReceiptSourceRegistry;
static NSMutableDictionary<NSString *, LensNativeWindowSource *> *LensObservationSourceRegistry;

@interface LensWindowOperation : NSObject
@property(nonatomic, assign) BOOL open;
@property(nonatomic, assign) uint32_t lastSelectionOrdinal;
@property(nonatomic, copy) NSString *invocationID;
@property(nonatomic, strong, nullable) LensNativeWindowSource *pendingSource;
@end
@implementation LensWindowOperation
@end
static NSMutableDictionary<NSString *, LensWindowOperation *> *LensWindowOperations;

static NSString *LensWindowRegistryKey(NSString *operationID, uint32_t windowID) {
    return [NSString stringWithFormat:@"%@:%u", operationID, windowID];
}

static NSString *LensObservationRegistryKey(
    NSString *operationID,
    NSString *sourceRegistrationID
) {
    return [NSString stringWithFormat:@"%@:%@", operationID, sourceRegistrationID];
}

static void LensEnsureWindowRegistries(void) {
    NSCAssert([NSThread isMainThread], @"Native selected-window registries are main-thread confined");
    if (LensWindowSourceRegistry == nil) {
        LensWindowSourceRegistry = [NSMutableDictionary dictionary];
        LensReceiptSourceRegistry = [NSMutableDictionary dictionary];
    }
    if (LensObservationSourceRegistry == nil) {
        LensObservationSourceRegistry = [NSMutableDictionary dictionary];
    }
    if (LensWindowOperations == nil) {
        LensWindowOperations = [NSMutableDictionary dictionary];
    }
}

static LensNativeWindowSource *_Nullable LensWindowSourceForIdentity(
    NSString *operationID,
    uint32_t windowID
) {
    __block LensNativeWindowSource *source = nil;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        source = LensWindowSourceRegistry[LensWindowRegistryKey(operationID, windowID)];
    });
    return source;
}

static BOOL LensStorePickerWindow(NSString *operationID, SCWindow *window) {
    NSCAssert([NSThread isMainThread], @"Picker-selected SCWindow storage is main-thread confined");
    if (operationID.length == 0 || window == nil || window.windowID == 0) {
        return NO;
    }
    LensEnsureWindowRegistries();
    NSString *key = LensWindowRegistryKey(operationID, window.windowID);
    LensNativeWindowSource *existing = LensWindowSourceRegistry[key];
    if (existing != nil) {
        // Never replace a retained reviewed target merely because a later object reused its ID.
        return existing.screenWindow == window;
    }
    LensWindowOperation *operation = LensWindowOperations[operationID];
    if (!operation.open || operation.lastSelectionOrdinal == UINT32_MAX) return NO;
    LensNativeWindowSource *source = [[LensNativeWindowSource alloc]
        initWithOperationID:operationID
                     window:window];
    source.receipt = NSUUID.UUID.UUIDString.lowercaseString;
    source.selectionOrdinal = ++operation.lastSelectionOrdinal;
    LensWindowSourceRegistry[key] = source;
    LensReceiptSourceRegistry[LensObservationRegistryKey(operationID, source.receipt)] = source;
    return YES;
}

static LensNativeWindowSource *_Nullable LensWindowSourceForReceipt(
    NSString *operationID, NSString *receipt
) {
    operationID = [[NSUUID alloc] initWithUUIDString:operationID].UUIDString.lowercaseString;
    receipt = [[NSUUID alloc] initWithUUIDString:receipt].UUIDString.lowercaseString;
    if (operationID.length == 0 || receipt.length == 0) return nil;
    __block LensNativeWindowSource *source = nil;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        if (LensWindowOperations[operationID].open) {
            source = LensReceiptSourceRegistry[LensObservationRegistryKey(operationID, receipt)];
        }
    });
    return source;
}

static BOOL LensStagePickerWindow(
    LensWindowOperation *operation, NSString *operationID, NSString *invocationID, SCWindow *window
) {
    NSCAssert([NSThread isMainThread], @"Picker admission is main-thread confined");
    if (!operation.open || ![operation.invocationID isEqual:invocationID] ||
        window == nil || window.windowID == 0) return NO;
    LensNativeWindowSource *existing = LensWindowSourceRegistry[
        LensWindowRegistryKey(operationID, window.windowID)];
    if (existing != nil && existing.screenWindow != window) return NO;
    if (existing == nil && operation.lastSelectionOrdinal == UINT32_MAX) return NO;
    LensNativeWindowSource *source = existing ?: [[LensNativeWindowSource alloc]
        initWithOperationID:operationID window:window];
    if (existing == nil) {
        source.receipt = NSUUID.UUID.UUIDString.lowercaseString;
        source.selectionOrdinal = ++operation.lastSelectionOrdinal;
    }
    operation.pendingSource = source;
    return YES;
}

@class LensContentPickerCoordinator;
static LensContentPickerCoordinator *_Nullable LensActiveContentPickerCoordinator;

@interface LensContentPickerCoordinator : NSObject <SCContentSharingPickerObserver>
@property(nonatomic, assign) LensPickerCallback callback;
@property(nonatomic, assign) void *callbackContext;
@property(nonatomic, copy, nullable) NSString *operationID;
@property(nonatomic, strong, nullable) LensWindowOperation *operation;
@property(nonatomic, copy, nullable) NSString *invocationID;
@property(nonatomic, assign) BOOL autoAccept;
- (BOOL)presentWithOperationID:(NSString *_Nullable)operationID
                       callback:(LensPickerCallback)callback
                        context:(void *)context;
@end

@implementation LensContentPickerCoordinator

- (void)deliver:(NSDictionary *)payload {
    NSAssert([NSThread isMainThread], @"Picker completion must be delivered on the main thread");
    LensPickerCallback callback = self.callback;
    void *context = self.callbackContext;
    self.callback = NULL;
    self.callbackContext = NULL;
    if (self.autoAccept && [payload[@"status"] isEqual:@"selected"]) {
        lens_accept_window_picker_invocation(self.operationID.UTF8String, self.invocationID.UTF8String);
    }
    if (self.operation != nil && ![payload[@"status"] isEqual:@"selected"] &&
        [self.operation.invocationID isEqual:self.invocationID]) {
        self.operation.invocationID = nil;
        self.operation.pendingSource = nil;
    }
    SCContentSharingPicker *picker = SCContentSharingPicker.sharedPicker;
    picker.active = NO;
    [picker removeObserver:self];
    if (LensActiveContentPickerCoordinator == self) {
        LensActiveContentPickerCoordinator = nil;
    }

    if (callback != NULL) {
        char *json = LensCopyJSONString(payload);
        callback(json, context);
        free(json);
    }
}

- (NSDictionary *)selectedPayloadForFilter:(SCContentFilter *)filter API_AVAILABLE(macos(15.2)) {
    NSArray<SCWindow *> *windows = filter.includedWindows;
    if (windows.count != 1) {
        return @{
            @"status": @"error",
            @"message": [NSString stringWithFormat:
                @"The single-window picker returned %lu included windows.",
                (unsigned long)windows.count]
        };
    }

    NSMutableArray<NSDictionary *> *serializedWindows =
        [NSMutableArray arrayWithCapacity:windows.count];
    for (SCWindow *window in windows) {
        BOOL admitted = YES;
        if (self.operation != nil) {
            admitted = LensStagePickerWindow(self.operation, self.operationID, self.invocationID, window);
        } else if (self.operationID.length > 0) {
            admitted = LensStorePickerWindow(self.operationID, window);
        }
        if (!admitted) {
            return @{
                @"status": @"error",
                @"message": @"The selected window identity conflicts with an existing retained target."
            };
        }
        SCRunningApplication *application = window.owningApplication;
        CGRect frame = window.frame;
        NSMutableDictionary *serialized = [@{
            @"title": LensStringOrEmpty(window.title),
            @"application_name": LensStringOrEmpty(application.applicationName),
            @"frame": @{
                @"x": @(frame.origin.x),
                @"y": @(frame.origin.y),
                @"width": @(frame.size.width),
                @"height": @(frame.size.height)
            }
        } mutableCopy];
        if (self.operation != nil && !self.autoAccept) {
            LensNativeWindowSource *source = self.operation.pendingSource;
            serialized[@"operation_id"] = self.operationID;
            serialized[@"receipt"] = source.receipt;
            serialized[@"selection_ordinal"] = @(source.selectionOrdinal);
            serialized[@"application_id"] = LensStringOrEmpty(application.bundleIdentifier);
        } else {
            serialized[@"window_id"] = @(window.windowID);
            serialized[@"bundle_id"] = LensStringOrEmpty(application.bundleIdentifier);
            serialized[@"pid"] = @(application.processID);
        }
        [serializedWindows addObject:serialized];
    }
    return @{ @"status": @"selected", @"windows": serializedWindows };
}

- (BOOL)presentWithOperationID:(NSString *_Nullable)operationID
                       callback:(LensPickerCallback)callback
                        context:(void *)context {
    NSAssert([NSThread isMainThread], @"The ScreenCaptureKit picker must be presented on the main thread");
    self.operationID = operationID;
    self.callback = callback;
    self.callbackContext = context;

    SCContentSharingPicker *picker = SCContentSharingPicker.sharedPicker;
    [picker addObserver:self];

    SCContentSharingPickerConfiguration *configuration = [[SCContentSharingPickerConfiguration alloc] init];
    configuration.allowedPickerModes = SCContentSharingPickerModeSingleWindow;
    configuration.allowsChangingSelectedContent = NO;
    NSString *ownBundleID = NSBundle.mainBundle.bundleIdentifier;
    configuration.excludedBundleIDs = ownBundleID.length > 0 ? @[ownBundleID] : @[];
    picker.defaultConfiguration = configuration;
    picker.active = YES;
    [picker presentPickerUsingContentStyle:SCShareableContentStyleWindow];
    return YES;
}

- (void)contentSharingPicker:(SCContentSharingPicker *)picker
          didCancelForStream:(SCStream *)stream {
    LensPerformOnMainThread(^{
        if (self.callback == NULL) {
            return;
        }
        [self deliver:@{ @"status": @"cancelled" }];
    });
}

- (void)contentSharingPicker:(SCContentSharingPicker *)picker
         didUpdateWithFilter:(SCContentFilter *)filter
                   forStream:(SCStream *)stream {
    LensPerformOnMainThread(^{
        if (self.callback == NULL) {
            return;
        }
        if (@available(macOS 15.2, *)) {
            [self deliver:[self selectedPayloadForFilter:filter]];
            return;
        }

        [self deliver:@{
            @"status": @"error",
            @"message": @"Lens requires macOS 15.2 or later for deterministic SCWindow resolution."
        }];
    });
}

- (void)contentSharingPickerStartDidFailWithError:(NSError *)error {
    LensPerformOnMainThread(^{
        if (self.callback == NULL) {
            return;
        }
        [self deliver:@{
            @"status": @"error",
            @"message": error.localizedDescription ?: @"The native window picker failed to start."
        }];
    });
}

@end

bool lens_accessibility_is_trusted(void) {
    return AXIsProcessTrusted();
}

bool lens_accessibility_request_trust(void) {
    NSDictionary *options = @{ (__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES };
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
}

static bool LensPresentWindowPicker(
    NSString *_Nullable operationID,
    LensPickerCallback callback,
    void *context
) {
    if (callback == NULL) {
        return false;
    }

    __block BOOL presented = NO;
    dispatch_block_t presentation = ^{
        if (LensActiveContentPickerCoordinator != nil) {
            return;
        }
        LensContentPickerCoordinator *coordinator = [[LensContentPickerCoordinator alloc] init];
        LensActiveContentPickerCoordinator = coordinator;
        presented = [coordinator
            presentWithOperationID:operationID
                          callback:callback
                           context:context];
        if (!presented && LensActiveContentPickerCoordinator == coordinator) {
            LensActiveContentPickerCoordinator = nil;
        }
    };
    if ([NSThread isMainThread]) {
        presentation();
    } else {
        dispatch_sync(dispatch_get_main_queue(), presentation);
    }
    return presented;
}

bool lens_present_window_picker(LensPickerCallback callback, void *context) {
    return LensPresentWindowPicker(nil, callback, context);
}

bool lens_present_window_picker_for_operation(
    const char *operationIDCString,
    LensPickerCallback callback,
    void *context
) {
    if (operationIDCString == NULL) {
        return false;
    }
    NSString *operationID = [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    if (operationID.length == 0) {
        return false;
    }
    lens_open_window_operation(operationIDCString);
    NSString *invocationID = NSUUID.UUID.UUIDString;
    __block BOOL presented = NO;
    LensPerformSyncOnMainThread(^{
        LensWindowOperation *operation = LensWindowOperations[operationID];
        if (callback == NULL || !operation.open || operation.invocationID != nil ||
            LensActiveContentPickerCoordinator != nil) return;
        operation.invocationID = invocationID;
        LensContentPickerCoordinator *coordinator = [[LensContentPickerCoordinator alloc] init];
        coordinator.operation = operation;
        coordinator.invocationID = invocationID;
        coordinator.autoAccept = YES;
        LensActiveContentPickerCoordinator = coordinator;
        presented = [coordinator presentWithOperationID:operationID callback:callback context:context];
        if (!presented) {
            operation.invocationID = nil;
            LensActiveContentPickerCoordinator = nil;
        }
    });
    return presented;
}

bool lens_open_window_operation(const char *operationIDCString) {
    NSString *operationID = operationIDCString == NULL ? nil :
        [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    if (operationID.length == 0) return false;
    __block BOOL opened = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        if (LensWindowOperations[operationID] != nil) return;
        LensWindowOperation *operation = [[LensWindowOperation alloc] init];
        operation.open = YES;
        LensWindowOperations[operationID] = operation;
        opened = YES;
    });
    return opened;
}

bool lens_present_window_picker_for_invocation(
    const char *operationIDCString, const char *invocationIDCString,
    LensPickerCallback callback, void *context
) {
    NSString *operationID = operationIDCString == NULL ? nil :
        [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    NSString *invocationID = invocationIDCString == NULL ? nil :
        [NSString stringWithUTF8String:invocationIDCString];
    if (operationID.length == 0 || invocationID.length == 0 || callback == NULL) return false;
    __block BOOL presented = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        LensWindowOperation *operation = LensWindowOperations[operationID];
        if (!operation.open || operation.invocationID != nil ||
            LensActiveContentPickerCoordinator != nil) return;
        operation.invocationID = invocationID;
        LensContentPickerCoordinator *coordinator = [[LensContentPickerCoordinator alloc] init];
        coordinator.operation = operation;
        coordinator.invocationID = invocationID;
        LensActiveContentPickerCoordinator = coordinator;
        presented = [coordinator presentWithOperationID:operationID callback:callback context:context];
        if (!presented) {
            operation.invocationID = nil;
            LensActiveContentPickerCoordinator = nil;
        }
    });
    return presented;
}

bool lens_accept_window_picker_invocation(const char *operationIDCString, const char *invocationIDCString) {
    NSString *operationID = operationIDCString == NULL ? nil : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    NSString *invocationID = invocationIDCString == NULL ? nil : [NSString stringWithUTF8String:invocationIDCString];
    if (operationID.length == 0 || invocationID.length == 0) return false;
    __block BOOL accepted = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        LensWindowOperation *operation = LensWindowOperations[operationID];
        LensNativeWindowSource *source = operation.pendingSource;
        if (!operation.open || ![operation.invocationID isEqual:invocationID] ||
            source == nil || source.screenWindow == nil) return;
        NSString *key = LensWindowRegistryKey(operationID, source.windowID);
        LensNativeWindowSource *existing = LensWindowSourceRegistry[key];
        if (existing != nil && existing != source) return;
        LensWindowSourceRegistry[key] = source;
        LensReceiptSourceRegistry[LensObservationRegistryKey(operationID, source.receipt)] = source;
        operation.pendingSource = nil;
        operation.invocationID = nil;
        accepted = YES;
    });
    return accepted;
}

bool lens_cancel_window_picker_invocation(const char *operationIDCString, const char *invocationIDCString) {
    NSString *operationID = operationIDCString == NULL ? nil : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    NSString *invocationID = invocationIDCString == NULL ? nil : [NSString stringWithUTF8String:invocationIDCString];
    if (operationID.length == 0 || invocationID.length == 0) return false;
    __block BOOL cancelled = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        LensWindowOperation *operation = LensWindowOperations[operationID];
        if (!operation.open || ![operation.invocationID isEqual:invocationID]) return;
        operation.invocationID = nil;
        operation.pendingSource = nil;
        LensContentPickerCoordinator *coordinator = LensActiveContentPickerCoordinator;
        if (coordinator.operation == operation && [coordinator.invocationID isEqual:invocationID]) {
            [coordinator deliver:@{ @"status": @"cancelled" }];
        }
        cancelled = YES;
    });
    return cancelled;
}

static NSDictionary *LensImageCaptureFailure(NSString *diagnostic) {
    return @{
        @"captures": @[],
        @"omissions": @[],
        @"diagnostics": @[diagnostic]
    };
}

static CGImageRef LensCopyScaledImage(
    CGImageRef image,
    size_t maxLongEdge,
    size_t maxPixels
) {
    size_t width = CGImageGetWidth(image);
    size_t height = CGImageGetHeight(image);
    if (width == 0 || height == 0) {
        return NULL;
    }
    double scale = 1.0;
    size_t longEdge = MAX(width, height);
    if (longEdge > maxLongEdge) {
        scale = MIN(scale, (double)maxLongEdge / (double)longEdge);
    }
    double pixels = (double)width * (double)height;
    if (pixels > (double)maxPixels) {
        scale = MIN(scale, sqrt((double)maxPixels / pixels));
    }
    if (scale >= 1.0) {
        return CGImageRetain(image);
    }

    size_t targetWidth = MAX((size_t)1, (size_t)floor((double)width * scale));
    size_t targetHeight = MAX((size_t)1, (size_t)floor((double)height * scale));
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(
        NULL,
        targetWidth,
        targetHeight,
        8,
        0,
        colorSpace,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big
    );
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) {
        return NULL;
    }
    CGContextSetInterpolationQuality(context, kCGInterpolationHigh);
    CGContextDrawImage(context, CGRectMake(0, 0, targetWidth, targetHeight), image);
    CGImageRef scaled = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    return scaled;
}

static NSData *LensPNGData(CGImageRef image) {
    NSBitmapImageRep *representation = [[NSBitmapImageRep alloc] initWithCGImage:image];
    return [representation representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
}

static SCWindow *_Nullable LensShareableWindowByID(
    uint32_t windowID,
    NSString *__autoreleasing _Nullable *diagnosticOut
) {
    dispatch_semaphore_t completion = dispatch_semaphore_create(0);
    __block SCWindow *selectedWindow = nil;
    __block NSString *diagnostic = nil;
    [SCShareableContent getShareableContentWithCompletionHandler:^(
        SCShareableContent *shareableContent,
        NSError *shareableError
    ) {
        if (shareableContent == nil) {
            diagnostic = shareableError.localizedDescription
                ?: @"ScreenCaptureKit returned no shareable content.";
        } else {
            for (SCWindow *window in shareableContent.windows) {
                if (window.windowID == windowID) {
                    selectedWindow = window;
                    break;
                }
            }
            if (selectedWindow == nil) {
                diagnostic = @"The requested one-shot window is no longer shareable.";
            }
        }
        dispatch_semaphore_signal(completion);
    }];

    dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC);
    if (dispatch_semaphore_wait(completion, timeout) != 0) {
        diagnostic = @"ScreenCaptureKit window lookup timed out after 15 seconds.";
        selectedWindow = nil;
    }
    if (diagnosticOut != NULL) {
        *diagnosticOut = diagnostic;
    }
    return selectedWindow;
}

static BOOL LensFinitePositiveRect(CGRect rect) {
    return !CGRectIsNull(rect) && !CGRectIsInfinite(rect)
        && isfinite(rect.origin.x) && isfinite(rect.origin.y)
        && isfinite(rect.size.width) && isfinite(rect.size.height)
        && rect.size.width > 0 && rect.size.height > 0
        && isfinite(CGRectGetMaxX(rect)) && isfinite(CGRectGetMaxY(rect))
        && CGRectGetMaxX(rect) > rect.origin.x && CGRectGetMaxY(rect) > rect.origin.y;
}

// Deliberately mirrors the portable validator's edge arithmetic, not CGRectIntersection's
// internal arithmetic. Contained requests retain their exact original width/height bits.
static BOOL LensLogicalIntersection(CGRect window, CGRect requested, CGRect *captured, BOOL *full) {
    if (!LensFinitePositiveRect(window) || !LensFinitePositiveRect(requested)) return NO;
    CGFloat windowRight = window.origin.x + window.size.width;
    CGFloat windowBottom = window.origin.y + window.size.height;
    CGFloat right = requested.origin.x + requested.size.width;
    CGFloat bottom = requested.origin.y + requested.size.height;
    *full = requested.origin.x >= window.origin.x && requested.origin.y >= window.origin.y
        && right <= windowRight && bottom <= windowBottom;
    if (*full) {
        *captured = requested;
        return YES;
    }
    CGFloat left = MAX(window.origin.x, requested.origin.x);
    CGFloat top = MAX(window.origin.y, requested.origin.y);
    right = MIN(windowRight, right);
    bottom = MIN(windowBottom, bottom);
    if (right <= left || bottom <= top) return NO;
    *captured = CGRectMake(left, top, right - left, bottom - top);
    return LensFinitePositiveRect(*captured);
}

static BOOL LensPixelCrop(CGRect window, CGRect region, size_t width, size_t height, CGRect *crop) {
    if (!LensFinitePositiveRect(window) || !LensFinitePositiveRect(region)
        || width == 0 || height == 0 || width > UINT32_MAX || height > UINT32_MAX) return NO;
    CGFloat sx = (CGFloat)width / window.size.width, sy = (CGFloat)height / window.size.height;
    CGFloat x0 = (CGRectGetMinX(region) - CGRectGetMinX(window)) * sx;
    CGFloat y0 = (CGRectGetMinY(region) - CGRectGetMinY(window)) * sy;
    CGFloat x1 = (CGRectGetMaxX(region) - CGRectGetMinX(window)) * sx;
    CGFloat y1 = (CGRectGetMaxY(region) - CGRectGetMinY(window)) * sy;
    if (!isfinite(sx) || !isfinite(sy) || sx <= 0 || sy <= 0
        || !isfinite(x0) || !isfinite(y0) || !isfinite(x1) || !isfinite(y1)) return NO;
    CGFloat left = MAX(0, floor(x0)), top = MAX(0, floor(y0));
    *crop = CGRectMake(left, top, MIN((CGFloat)width, ceil(x1)) - left,
        MIN((CGFloat)height, ceil(y1)) - top);
    return LensFinitePositiveRect(*crop) && CGRectGetMaxX(*crop) <= width && CGRectGetMaxY(*crop) <= height;
}

static NSDictionary *LensPixelGeometry(CGImageRef original, CGRect crop, CGImageRef cropped, CGImageRef encoded) {
    if (original == NULL || cropped == NULL || encoded == NULL || !LensFinitePositiveRect(crop)
        || crop.origin.x < 0 || crop.origin.y < 0
        || floor(crop.origin.x) != crop.origin.x || floor(crop.origin.y) != crop.origin.y
        || floor(crop.size.width) != crop.size.width || floor(crop.size.height) != crop.size.height
        || CGImageGetWidth(original) > UINT32_MAX || CGImageGetHeight(original) > UINT32_MAX
        || CGRectGetMaxX(crop) > CGImageGetWidth(original) || CGRectGetMaxY(crop) > CGImageGetHeight(original)
        || CGImageGetWidth(cropped) != crop.size.width || CGImageGetHeight(cropped) != crop.size.height
        || CGImageGetWidth(encoded) == 0 || CGImageGetHeight(encoded) == 0
        || CGImageGetWidth(encoded) > UINT32_MAX || CGImageGetHeight(encoded) > UINT32_MAX) return nil;
    return @{ @"original_extent": @{ @"width": @(CGImageGetWidth(original)), @"height": @(CGImageGetHeight(original)) },
        @"crop": @{ @"x": @((uint32_t)crop.origin.x), @"y": @((uint32_t)crop.origin.y),
            @"extent": @{ @"width": @(CGImageGetWidth(cropped)), @"height": @(CGImageGetHeight(cropped)) } },
        @"encoded_extent": @{ @"width": @(CGImageGetWidth(encoded)), @"height": @(CGImageGetHeight(encoded)) } };
}

#if defined(LENS_NATIVE_TESTING)
bool lens_test_pixel_geometry(void) {
    CGRect intersection = CGRectZero;
    BOOL full = NO;
    CGRect fractional = CGRectMake(0.1, 0.2, 0.3, 0.4);
    if (!LensLogicalIntersection(CGRectMake(0, 0, 1, 1), fractional, &intersection, &full)
        || !full || intersection.size.width != fractional.size.width || intersection.size.height != fractional.size.height) return false;
    CGRect large = CGRectMake(-1e15, -1, 1e15 + 0.125, 2);
    CGRect crossing = CGRectMake(-0.25, -0.5, 1, 1);
    CGFloat expectedRight = large.origin.x + large.size.width;
    if (!LensLogicalIntersection(large, crossing, &intersection, &full) || full
        || intersection.origin.x != crossing.origin.x
        || intersection.size.width != expectedRight - crossing.origin.x
        || intersection.origin.y != crossing.origin.y || intersection.size.height != crossing.size.height) return false;
    if (LensLogicalIntersection(CGRectMake(0, 0, 1, 1), CGRectMake(1, 0, 1, 1), &intersection, &full)) return false;
    CGColorSpaceRef color = CGColorSpaceCreateDeviceRGB();
    CGContextRef bitmap = CGBitmapContextCreate(NULL, 8, 6, 8, 32, color, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
    CGColorSpaceRelease(color);
    if (bitmap == NULL) return false;
    CGImageRef image = CGBitmapContextCreateImage(bitmap);
    CGContextRelease(bitmap);
    if (image == NULL) return false;
    CGRect window = CGRectMake(-10.25, -20.5, 4, 3), crop = CGRectZero;
    BOOL valid = LensPixelCrop(window, CGRectMake(-10, -20.25, 1.1, 1.1), 8, 6, &crop)
        && CGRectEqualToRect(crop, CGRectMake(0, 0, 3, 3));
    CGImageRef cropped = valid ? CGImageCreateWithImageInRect(image, crop) : NULL;
    CGImageRef bounded = cropped == NULL ? NULL : LensCopyScaledImage(cropped, 2, 4);
    NSDictionary *metadata = LensPixelGeometry(image, crop, cropped, bounded);
    valid = valid && [metadata[@"original_extent"] isEqual:@{ @"width": @8, @"height": @6 }]
        && [metadata[@"crop"] isEqual:@{ @"x": @0, @"y": @0, @"extent": @{ @"width": @3, @"height": @3 } }]
        && [metadata[@"encoded_extent"] isEqual:@{ @"width": @2, @"height": @2 }];
    if (cropped != NULL) CGImageRelease(cropped);
    if (bounded != NULL) CGImageRelease(bounded);
    valid = valid && LensPixelCrop(window, CGRectMake(-11, -21, 6, 5), 8, 6, &crop)
        && CGRectEqualToRect(crop, CGRectMake(0, 0, 8, 6))
        && !LensPixelCrop(window, CGRectMake(INFINITY, 0, 1, 1), 8, 6, &crop)
        && !LensPixelCrop(window, window, 0, 6, &crop);
    CGImageRelease(image);
    return valid;
}
#endif

static char *LensCaptureWindowRegionsJSON(
    SCWindow *_Nullable selectedWindow,
    CGRect currentWindowFrame,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes,
    NSString *_Nullable unavailableDiagnostic
) {
    if (selectedWindow == nil) {
        return LensCopyJSONString(LensImageCaptureFailure(
            unavailableDiagnostic ?: @"The retained selected window is unavailable."
        ));
    }
    if (requestsJSON == NULL || maxLongEdge == 0 || maxPixels == 0 ||
        maxAttachmentBytes == 0 || maxTotalBytes == 0) {
        return LensCopyJSONString(LensImageCaptureFailure(
            @"Image capture requests and limits must be present and greater than zero."
        ));
    }
    NSData *requestData = [NSData dataWithBytes:requestsJSON length:strlen(requestsJSON)];
    NSError *requestError = nil;
    id requestObject = [NSJSONSerialization
        JSONObjectWithData:requestData
        options:0
        error:&requestError];
    if (![requestObject isKindOfClass:NSArray.class]) {
        return LensCopyJSONString(LensImageCaptureFailure(
            requestError.localizedDescription ?: @"Image capture requests must be a JSON array."
        ));
    }
    NSArray<NSDictionary *> *requests = requestObject;
    if (requests.count == 0) {
        return LensCopyJSONString(@{
            @"captures": @[],
            @"omissions": @[],
            @"diagnostics": @[]
        });
    }

    // `SCWindow` identifies the retained capture target. Apple does not document its readonly
    // metadata as a live observation stream. The native regression probe demonstrates that a
    // filter created from a retained `SCWindow` can keep picker-time geometry after resize. The
    // exact promoted AXWindow therefore owns current point geometry while ScreenCaptureKit owns
    // capture identity and pixels.
    SCContentFilter *filter = [[SCContentFilter alloc]
        initWithDesktopIndependentWindow:selectedWindow];
    CGRect windowFrame = currentWindowFrame;
    if (!LensFinitePositiveRect(windowFrame)) {
        return LensCopyJSONString(LensImageCaptureFailure(
            @"The exact selected window did not expose finite current capture geometry."
        ));
    }
    CGFloat sourceWidth = windowFrame.size.width;
    CGFloat sourceHeight = windowFrame.size.height;
    CGFloat scale = MIN(2.0, MIN(4096.0 / sourceWidth, 4096.0 / sourceHeight));
    SCStreamConfiguration *configuration = [[SCStreamConfiguration alloc] init];
    configuration.width = (size_t)MAX(1.0, floor(sourceWidth * scale));
    configuration.height = (size_t)MAX(1.0, floor(sourceHeight * scale));
    configuration.scalesToFit = YES;
    // The declared per-axis mapping covers the entire output, without hidden letterboxing.
    // Independent integer output rounding may intentionally produce slightly unequal scales.
    configuration.preservesAspectRatio = NO;
    configuration.destinationRect = CGRectMake(0, 0, configuration.width, configuration.height);
    configuration.showsCursor = NO;
    configuration.ignoreShadowsSingleWindow = YES;
    configuration.shouldBeOpaque = YES;
    dispatch_semaphore_t completion = dispatch_semaphore_create(0);
    __block NSDictionary *result = nil;
    [SCScreenshotManager
        captureImageWithFilter:filter
        configuration:configuration
        completionHandler:^(CGImageRef image, NSError *captureError) {
            if (image == NULL) {
                result = LensImageCaptureFailure(
                    captureError.localizedDescription
                        ?: @"ScreenCaptureKit returned no window image."
                );
                dispatch_semaphore_signal(completion);
                return;
            }
            if (CGImageGetWidth(image) != configuration.width || CGImageGetHeight(image) != configuration.height) {
                result = LensImageCaptureFailure(@"Native capture extent does not match its declared destination rectangle.");
                dispatch_semaphore_signal(completion);
                return;
            }

            NSUInteger totalBytes = 0;
            NSMutableArray<NSDictionary *> *captures = [NSMutableArray array];
            NSMutableArray<NSDictionary *> *omissions = [NSMutableArray array];
            for (NSDictionary *request in requests) {
                NSString *attachmentID = request[@"id"];
                NSString *scope = request[@"scope"];
                if (attachmentID.length == 0 || scope.length == 0) {
                    [omissions addObject:@{
                        @"attachment_id": attachmentID ?: @"",
                        @"reason": @"capture_failed",
                        @"detail": @"Image capture request is missing its id or scope."
                    }];
                    continue;
                }
                CGRect requestedBounds = windowFrame;
                if (![scope isEqualToString:@"window_fallback"]) {
                    NSDictionary *bounds = request[@"bounds"];
                    if (![bounds isKindOfClass:NSDictionary.class]) {
                        [omissions addObject:@{
                            @"attachment_id": attachmentID,
                            @"reason": @"capture_failed",
                            @"detail": @"AX image-region request has no bounds."
                        }];
                        continue;
                    }
                    requestedBounds = CGRectMake(
                        [bounds[@"x"] doubleValue],
                        [bounds[@"y"] doubleValue],
                        [bounds[@"width"] doubleValue],
                        [bounds[@"height"] doubleValue]
                    );
                }
                if (!LensFinitePositiveRect(requestedBounds)) {
                    [omissions addObject:@{ @"attachment_id": attachmentID, @"reason": @"capture_failed", @"detail": @"Invalid requested rectangle edges." }];
                    continue;
                }
                CGRect capturedBounds = CGRectZero;
                BOOL capturedFullRegion = NO;
                if (!LensLogicalIntersection(windowFrame, requestedBounds, &capturedBounds, &capturedFullRegion)) {
                    [omissions addObject:@{
                        @"attachment_id": attachmentID,
                        @"reason": @"outside_window",
                        @"detail": @"AX image region does not intersect the current selected-window frame."
                    }];
                    continue;
                }

                CGRect pixelRect;
                if (!LensPixelCrop(windowFrame, capturedBounds, CGImageGetWidth(image), CGImageGetHeight(image), &pixelRect)) {
                    [omissions addObject:@{
                        @"attachment_id": attachmentID,
                        @"reason": @"outside_window",
                        @"detail": @"AX image region produced an empty pixel intersection."
                    }];
                    continue;
                }

                CGImageRef cropped = CGImageCreateWithImageInRect(image, pixelRect);
                CGImageRef bounded = cropped == NULL
                    ? NULL
                    : LensCopyScaledImage(cropped, maxLongEdge, maxPixels);
                NSDictionary *pixelGeometry = LensPixelGeometry(image, pixelRect, cropped, bounded);
                if (cropped != NULL) CGImageRelease(cropped);
                if (bounded == NULL || pixelGeometry == nil) {
                    if (bounded != NULL) CGImageRelease(bounded);
                    [omissions addObject:@{
                        @"attachment_id": attachmentID,
                        @"reason": @"capture_failed",
                        @"detail": @"Unable to create the bounded image-region bitmap."
                    }];
                    continue;
                }
                NSData *png = LensPNGData(bounded);
                size_t pixelWidth = CGImageGetWidth(bounded);
                size_t pixelHeight = CGImageGetHeight(bounded);
                CGImageRelease(bounded);
                if (png == nil) {
                    [omissions addObject:@{
                        @"attachment_id": attachmentID,
                        @"reason": @"capture_failed",
                        @"detail": @"Unable to encode the image-region bitmap as PNG."
                    }];
                    continue;
                }
                if (png.length > maxAttachmentBytes ||
                    png.length > maxTotalBytes - MIN(totalBytes, maxTotalBytes)) {
                    [omissions addObject:@{
                        @"attachment_id": attachmentID,
                        @"reason": @"byte_budget",
                        @"detail": @"Encoded PNG exceeded the versioned per-attachment or total media byte budget."
                    }];
                    continue;
                }
                totalBytes += png.length;
                [captures addObject:@{
                    @"attachment_id": attachmentID,
                    @"source_bounds": LensFrameDictionary(requestedBounds),
                    // Unrounded logical intersection, not the outward-rounded pixel footprint.
                    @"captured_bounds": LensFrameDictionary(capturedBounds),
                    @"window_bounds": LensFrameDictionary(windowFrame),
                    @"coverage": capturedFullRegion ? @"full_region" : @"visible_subregion",
                    @"mime_type": @"image/png",
                    @"pixel_width": @(pixelWidth),
                    @"pixel_height": @(pixelHeight),
                    @"pixel_geometry": pixelGeometry,
                    @"encoded_bytes": @(png.length),
                    @"data": [png base64EncodedStringWithOptions:0]
                }];
            }
            result = @{
                @"window_bounds": LensFrameDictionary(windowFrame),
                @"captures": captures,
                @"omissions": omissions,
                @"diagnostics": @[]
            };
            dispatch_semaphore_signal(completion);
        }];

    dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC);
    if (dispatch_semaphore_wait(completion, timeout) != 0) {
        return LensCopyJSONString(LensImageCaptureFailure(
            @"ScreenCaptureKit image-region capture timed out after 15 seconds."
        ));
    }
    return LensCopyJSONString(
        result ?: LensImageCaptureFailure(@"Image-region capture produced no result.")
    );
}

char *lens_capture_window_regions_json(
    uint32_t windowID,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes
) {
    @autoreleasepool {
        NSString *diagnostic = nil;
        SCWindow *selectedWindow = LensShareableWindowByID(windowID, &diagnostic);
        return LensCaptureWindowRegionsJSON(
            selectedWindow,
            selectedWindow == nil ? CGRectNull : selectedWindow.frame,
            requestsJSON,
            maxLongEdge,
            maxPixels,
            maxAttachmentBytes,
            maxTotalBytes,
            diagnostic
        );
    }
}

static char *LensCaptureRegisteredSourceJSON(
    LensNativeWindowSource *source,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes
) {
    @autoreleasepool {
        // Keep screen-capture-only access available without AX permission. The
        // retained SCWindow frame is a snapshot, not an observed current frame.
        CGRect currentWindowFrame = source == nil ? CGRectNull : source.screenWindow.frame;
        AXUIElementRef resolvedWindow = [source copyResolvedWindow];
        if (resolvedWindow != NULL && (!LensAXFrame(resolvedWindow, &currentWindowFrame)
            || !LensFinitePositiveRect(currentWindowFrame))) {
            CFRelease(resolvedWindow);
            NSMutableDictionary *failure = [LensImageCaptureFailure(@"The promoted AXWindow did not expose finite current capture geometry.") mutableCopy];
            failure[@"geometry_observation"] = NSNull.null;
            return LensCopyJSONString(failure);
        }
        char *json = LensCaptureWindowRegionsJSON(
            source.screenWindow,
            currentWindowFrame,
            requestsJSON,
            maxLongEdge,
            maxPixels,
            maxAttachmentBytes,
            maxTotalBytes,
            @"The exact operation-scoped selected window is unavailable."
        );
        id result = nil;
        if (json != NULL) {
            NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
            result = data == nil ? nil : [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingMutableContainers error:nil];
            free(json);
        }
        CGRect afterFrame = CGRectNull;
        BOOL successful = [result isKindOfClass:NSMutableDictionary.class]
            && [result[@"captures"] isKindOfClass:NSArray.class] && [result[@"captures"] count] > 0;
        BOOL afterAvailable = successful && resolvedWindow != NULL
            && LensAXFrame(resolvedWindow, &afterFrame) && LensFinitePositiveRect(afterFrame);
        if (resolvedWindow != NULL) CFRelease(resolvedWindow);
        if (![result isKindOfClass:NSMutableDictionary.class]) return NULL;
        result[@"geometry_observation"] = LensGeometryObservation(currentWindowFrame, afterAvailable ? afterFrame : CGRectNull);
        if (successful && !afterAvailable) {
            NSMutableArray *messages = [result[@"diagnostics"] mutableCopy] ?: [NSMutableArray array];
            [messages addObject:@"Current desktop geometry observation is unavailable; retained SCWindow metadata is only a snapshot, not evidence of frame stability."];
            result[@"diagnostics"] = messages;
        }
        return LensCopyJSONString(result);
    }
}

static id LensCopyAXAttribute(AXUIElementRef element, CFStringRef attribute, AXError *errorOut) {
    CFTypeRef value = NULL;
    AXError error = AXUIElementCopyAttributeValue(element, attribute, &value);
    if (errorOut != NULL) {
        *errorOut = error;
    }
    if (error != kAXErrorSuccess || value == NULL) {
        if (value != NULL) {
            CFRelease(value);
        }
        return nil;
    }
    return CFBridgingRelease(value);
}

typedef NS_ENUM(NSUInteger, LensAXApplicationPreparationState) {
    LensAXApplicationPreparationStateReady,
    LensAXApplicationPreparationStateUnsupported,
    LensAXApplicationPreparationStateFailed,
};

typedef struct {
    LensAXApplicationPreparationState state;
    AXError roleError;
} LensAXApplicationPreparation;

static NSString *LensAXErrorName(AXError error) {
    switch (error) {
        case kAXErrorSuccess: return @"success";
        case kAXErrorFailure: return @"failure";
        case kAXErrorIllegalArgument: return @"illegal_argument";
        case kAXErrorInvalidUIElement: return @"invalid_ui_element";
        case kAXErrorInvalidUIElementObserver: return @"invalid_ui_element_observer";
        case kAXErrorCannotComplete: return @"cannot_complete";
        case kAXErrorAttributeUnsupported: return @"attribute_unsupported";
        case kAXErrorActionUnsupported: return @"action_unsupported";
        case kAXErrorNotificationUnsupported: return @"notification_unsupported";
        case kAXErrorNotImplemented: return @"not_implemented";
        case kAXErrorNotificationAlreadyRegistered: return @"notification_already_registered";
        case kAXErrorNotificationNotRegistered: return @"notification_not_registered";
        case kAXErrorAPIDisabled: return @"api_disabled";
        case kAXErrorNoValue: return @"no_value";
        case kAXErrorParameterizedAttributeUnsupported: return @"parameterized_attribute_unsupported";
        case kAXErrorNotEnoughPrecision: return @"not_enough_precision";
    }
    return @"unknown";
}

static NSString *LensAXApplicationPreparationStateName(LensAXApplicationPreparationState state) {
    switch (state) {
        case LensAXApplicationPreparationStateReady: return @"ready";
        case LensAXApplicationPreparationStateUnsupported: return @"unsupported";
        case LensAXApplicationPreparationStateFailed: return @"failed";
    }
    return @"unknown";
}

static LensAXApplicationPreparation LensPrepareAXApplication(AXUIElementRef application) {
    AXError roleError = kAXErrorFailure;
    id role = LensCopyAXAttribute(application, kAXRoleAttribute, &roleError);
    LensAXApplicationPreparationState state;
    if (roleError == kAXErrorSuccess && [role isKindOfClass:NSString.class]) {
        state = LensAXApplicationPreparationStateReady;
    } else if (roleError == kAXErrorAttributeUnsupported || roleError == kAXErrorNoValue) {
        state = LensAXApplicationPreparationStateUnsupported;
    } else {
        state = LensAXApplicationPreparationStateFailed;
    }
    return (LensAXApplicationPreparation){ state, roleError };
}

static NSString *LensAXString(AXUIElementRef element, CFStringRef attribute) {
    id value = LensCopyAXAttribute(element, attribute, NULL);
    if ([value isKindOfClass:NSString.class]) {
        return value;
    }
    if ([value isKindOfClass:NSNumber.class]) {
        return [value stringValue];
    }
    if ([value isKindOfClass:NSAttributedString.class]) {
        return [value string];
    }
    return nil;
}

static NSString *LensAXURIString(
    AXUIElementRef element,
    CFStringRef attribute,
    BOOL acceptsString,
    NSUInteger *readErrors
) {
    AXError error = kAXErrorFailure;
    id value = LensCopyAXAttribute(element, attribute, &error);
    if (value == nil) {
        if (error != kAXErrorAttributeUnsupported && error != kAXErrorNoValue) {
            *readErrors += 1;
        }
        return nil;
    }
    CFTypeRef type = (__bridge CFTypeRef)value;
    NSString *candidate = nil;
    if (CFGetTypeID(type) == CFURLGetTypeID()) {
        candidate = [(__bridge NSURL *)type absoluteString];
    } else if (acceptsString && [value isKindOfClass:NSString.class]) {
        candidate = value;
    }
    if (candidate.length == 0) {
        *readErrors += 1;
        return nil;
    }
    NSURLComponents *components = [NSURLComponents componentsWithString:candidate];
    if (components.scheme.length == 0) {
        *readErrors += 1;
        return nil;
    }
    return candidate;
}

static void LensAppendAXResourceReference(
    NSMutableArray<NSDictionary *> *references,
    NSString *uri,
    NSString *sourceAttribute,
    NSUInteger maxResourceRefs,
    NSUInteger maxResourceURIBytes,
    NSUInteger maxTotalResourceURIBytes,
    NSUInteger *resourceRefCount,
    NSUInteger *resourceURIBytes,
    NSUInteger *omittedResourceRefs
) {
    if (uri.length == 0) {
        return;
    }
    NSUInteger uriBytes = [uri lengthOfBytesUsingEncoding:NSUTF8StringEncoding];
    BOOL fitsTotal = *resourceURIBytes <= maxTotalResourceURIBytes
        && uriBytes <= maxTotalResourceURIBytes - *resourceURIBytes;
    if (uriBytes <= maxResourceURIBytes
        && *resourceRefCount < maxResourceRefs
        && fitsTotal) {
        [references addObject:@{
            @"uri": uri,
            @"source_attribute": sourceAttribute
        }];
        *resourceRefCount += 1;
        *resourceURIBytes += uriBytes;
    } else {
        *omittedResourceRefs += 1;
    }
}

static NSNumber *LensAXNumber(AXUIElementRef element, CFStringRef attribute) {
    id value = LensCopyAXAttribute(element, attribute, NULL);
    return [value isKindOfClass:NSNumber.class] ? value : nil;
}

static BOOL LensAXFrame(AXUIElementRef element, CGRect *frameOut) {
    id positionObject = LensCopyAXAttribute(element, kAXPositionAttribute, NULL);
    id sizeObject = LensCopyAXAttribute(element, kAXSizeAttribute, NULL);
    if (positionObject == nil || sizeObject == nil) {
        return NO;
    }

    AXValueRef positionValue = (__bridge AXValueRef)positionObject;
    AXValueRef sizeValue = (__bridge AXValueRef)sizeObject;
    if (CFGetTypeID(positionValue) != AXValueGetTypeID() ||
        CFGetTypeID(sizeValue) != AXValueGetTypeID()) {
        return NO;
    }

    CGPoint position = CGPointZero;
    CGSize size = CGSizeZero;
    if (!AXValueGetValue(positionValue, kAXValueTypeCGPoint, &position) ||
        !AXValueGetValue(sizeValue, kAXValueTypeCGSize, &size)) {
        return NO;
    }

    *frameOut = (CGRect){ position, size };
    return YES;
}

static NSDictionary *LensFrameDictionary(CGRect frame) {
    return @{
        @"x": @(frame.origin.x),
        @"y": @(frame.origin.y),
        @"width": @(frame.size.width),
        @"height": @(frame.size.height)
    };
}

static NSArray *LensAXElementsForArrayAttribute(
    AXUIElementRef element,
    CFStringRef attribute,
    NSUInteger maxValues,
    NSUInteger *reportedCount,
    BOOL *truncated,
    NSUInteger *readErrorCount
) {
    CFIndex count = 0;
    AXError countError = AXUIElementGetAttributeValueCount(element, attribute, &count);
    if (countError == kAXErrorAttributeUnsupported || countError == kAXErrorNoValue) {
        if (reportedCount != NULL) *reportedCount = 0;
        if (truncated != NULL) *truncated = NO;
        return @[];
    }
    if (countError != kAXErrorSuccess) {
        if (reportedCount != NULL) *reportedCount = 0;
        if (truncated != NULL) *truncated = NO;
        if (readErrorCount != NULL) {
            (*readErrorCount)++;
        }
        return @[];
    }

    NSUInteger availableCount = count > 0 ? (NSUInteger)count : 0;
    if (reportedCount != NULL) *reportedCount = availableCount;
    NSMutableArray *result = [NSMutableArray arrayWithCapacity:MIN(availableCount, maxValues)];
    const CFIndex batchSize = 256;
    for (CFIndex offset = 0; offset < count && result.count < maxValues;) {
        CFArrayRef batch = NULL;
        NSUInteger remainingBudget = maxValues - result.count;
        CFIndex requested = MIN(batchSize, count - offset);
        requested = MIN(requested, (CFIndex)remainingBudget);
        AXError batchError = AXUIElementCopyAttributeValues(
            element,
            attribute,
            offset,
            requested,
            &batch
        );
        if (batchError != kAXErrorSuccess || batch == NULL) {
            if (readErrorCount != NULL) {
                (*readErrorCount)++;
            }
            if (batch != NULL) {
                CFRelease(batch);
            }
            break;
        }
        NSArray *values = CFBridgingRelease(batch);
        if (values.count == 0) {
            break;
        }
        [result addObjectsFromArray:values];
        offset += (CFIndex)values.count;
    }
    if (truncated != NULL) *truncated = result.count < availableCount;
    return result;
}

static double LensWindowResolutionScore(
    NSString *selectedTitle,
    CGRect selectedFrame,
    NSString *candidateTitle,
    CGRect candidateFrame
) {
    double score = 0.0;
    NSString *left = [selectedTitle stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    NSString *right = [candidateTitle stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (left.length > 0 && right.length > 0) {
        if ([left isEqualToString:right]) {
            score += 100.0;
        } else if ([left localizedCaseInsensitiveContainsString:right] ||
                   [right localizedCaseInsensitiveContainsString:left]) {
            score += 55.0;
        }
    }

    double delta = fabs(selectedFrame.origin.x - candidateFrame.origin.x) +
        fabs(selectedFrame.origin.y - candidateFrame.origin.y) +
        fabs(selectedFrame.size.width - candidateFrame.size.width) +
        fabs(selectedFrame.size.height - candidateFrame.size.height);
    if (delta <= 4.0) {
        score += 100.0;
    } else if (delta <= 24.0) {
        score += 70.0;
    } else {
        CGRect intersection = CGRectIntersection(selectedFrame, candidateFrame);
        double unionArea = selectedFrame.size.width * selectedFrame.size.height +
            candidateFrame.size.width * candidateFrame.size.height -
            MAX(intersection.size.width, 0.0) * MAX(intersection.size.height, 0.0);
        double intersectionArea = MAX(intersection.size.width, 0.0) * MAX(intersection.size.height, 0.0);
        if (unionArea > 0.0) {
            score += 50.0 * intersectionArea / unionArea;
        }
    }
    return score;
}

static AXUIElementRef LensCopyResolvedAXWindowForApplication(
    AXUIElementRef application,
    NSString *selectedTitle,
    CGRect selectedFrame,
    NSString *__autoreleasing _Nullable *resolvedTitleOut,
    CGRect *resolvedFrameOut,
    double *resolutionScoreOut,
    NSMutableArray<NSString *> *_Nullable diagnostics
) {
    // Firefox lazily enables its macOS accessibility service when the application AXRole is read.
    // Keep this standard read before AXWindows so extraction does not depend on another assistive
    // technology having activated the target first.
    // Source: https://searchfox.org/firefox-main/source/accessible/mac/Platform.mm#829-842
    LensAXApplicationPreparation preparation = LensPrepareAXApplication(application);
    AXError windowsError = kAXErrorFailure;
    NSArray *windows = LensCopyAXAttribute(application, kAXWindowsAttribute, &windowsError);
    if (![windows isKindOfClass:NSArray.class] || windows.count == 0) {
        if (diagnostics != nil) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"Accessibility target window list is unavailable after application preparation "
                 "(preparation: %@; AXRole: %@ (%d); AXWindows: %@ (%d)).",
                LensAXApplicationPreparationStateName(preparation.state),
                LensAXErrorName(preparation.roleError),
                preparation.roleError,
                LensAXErrorName(windowsError),
                windowsError
            ]];
        }
        return NULL;
    }
    if (preparation.state != LensAXApplicationPreparationStateReady && diagnostics != nil) {
        [diagnostics addObject:[NSString stringWithFormat:
            @"Accessibility application preparation was %@ (AXRole: %@ (%d)); "
             "window extraction continued because AXWindows remained available.",
            LensAXApplicationPreparationStateName(preparation.state),
            LensAXErrorName(preparation.roleError),
            preparation.roleError
        ]];
    }

    AXUIElementRef resolvedWindow = NULL;
    double bestScore = -1.0;
    NSString *resolvedTitle = @"";
    CGRect resolvedFrame = CGRectZero;
    for (id candidateObject in windows) {
        CFTypeRef candidateType = (__bridge CFTypeRef)candidateObject;
        if (CFGetTypeID(candidateType) != AXUIElementGetTypeID()) {
            continue;
        }
        AXUIElementRef candidate = (__bridge AXUIElementRef)candidateObject;
        NSString *candidateTitle = LensAXString(candidate, kAXTitleAttribute) ?: @"";
        CGRect candidateFrame = CGRectZero;
        BOOL hasFrame = LensAXFrame(candidate, &candidateFrame);
        double score = LensWindowResolutionScore(
            selectedTitle,
            selectedFrame,
            candidateTitle,
            hasFrame ? candidateFrame : CGRectZero
        );
        if (score > bestScore) {
            bestScore = score;
            resolvedWindow = candidate;
            resolvedTitle = candidateTitle;
            resolvedFrame = candidateFrame;
        }
    }

    if (resolvedWindow == NULL || (bestScore < 25.0 && windows.count > 1)) {
        if (diagnostics != nil) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"LensTargetResolver found %lu AXWindows but none met the deterministic match "
                 "contract (best score %.1f).",
                (unsigned long)windows.count,
                bestScore
            ]];
        }
        return NULL;
    }
    CFRetain(resolvedWindow);
    if (resolvedTitleOut != NULL) *resolvedTitleOut = resolvedTitle;
    if (resolvedFrameOut != NULL) *resolvedFrameOut = resolvedFrame;
    if (resolutionScoreOut != NULL) *resolutionScoreOut = bestScore;
    return resolvedWindow;
}

static AXUIElementRef LensCopyResolvedAXWindow(
    int32_t pid,
    NSString *selectedTitle,
    CGRect selectedFrame,
    NSString *__autoreleasing _Nullable *resolvedTitleOut,
    CGRect *resolvedFrameOut,
    double *resolutionScoreOut,
    NSMutableArray<NSString *> *_Nullable diagnostics
) {
    AXUIElementRef application = AXUIElementCreateApplication(pid);
    if (application == NULL) {
        if (diagnostics != nil) {
            [diagnostics addObject:@"Unable to create the target application AXUIElement."];
        }
        return NULL;
    }
    AXUIElementRef resolvedWindow = LensCopyResolvedAXWindowForApplication(
        application,
        selectedTitle,
        selectedFrame,
        resolvedTitleOut,
        resolvedFrameOut,
        resolutionScoreOut,
        diagnostics
    );
    CFRelease(application);
    return resolvedWindow;
}

static NSString *LensNativeObservationKindName(LensNativeObservationKind kind) {
    switch (kind) {
        case LensNativeObservationKindWindowTitleChanged:
            return @"window_title_changed";
        case LensNativeObservationKindWindowMoved:
            return @"window_moved";
        case LensNativeObservationKindWindowResized:
            return @"window_resized";
        case LensNativeObservationKindWindowDestroyed:
            return @"window_destroyed";
        case LensNativeObservationKindApplicationChanged:
            return @"application_changed";
    }
    return @"application_changed";
}

static void LensNativeAXObserverCallback(
    AXObserverRef observer,
    AXUIElementRef element,
    CFStringRef notification,
    void *_Nullable context
);

@implementation LensNativeWindowSource

- (instancetype)initWithOperationID:(NSString *)operationID window:(SCWindow *)window {
    self = [super init];
    if (self != nil) {
        _operationID = [operationID copy];
        _windowID = window.windowID;
        _screenWindow = window;
        _pid = window.owningApplication.processID;
        _pickerTitle = [LensStringOrEmpty(window.title) copy];
        _pickerApplicationName = [LensStringOrEmpty(
            window.owningApplication.applicationName
        ) copy];
        _pickerFrame = window.frame;
        _initialResolutionScore = -1.0;
    }
    return self;
}

- (void)dealloc {
    if (_observer != NULL) {
        CFRelease(_observer);
    }
    if (_windowElement != NULL) {
        CFRelease(_windowElement);
    }
    if (_applicationElement != NULL) {
        CFRelease(_applicationElement);
    }
}

- (BOOL)ensureResolvedWindowWithDiagnostics:(NSMutableArray<NSString *> *)diagnostics {
    @synchronized(self) {
        if (_windowElement != NULL && _applicationElement != NULL) {
            return YES;
        }
        if (self.screenWindow == nil || self.pid <= 0) {
            [diagnostics addObject:@"The exact picker-retained SCWindow is unavailable."];
            return NO;
        }
        if (!AXIsProcessTrusted()) {
            [diagnostics addObject:@"Accessibility permission is not granted to Lens."];
            return NO;
        }

        AXUIElementRef application = AXUIElementCreateApplication(self.pid);
        if (application == NULL) {
            [diagnostics addObject:@"Unable to create the retained target's application AXUIElement."];
            return NO;
        }
        // AXUIElement.h defines this as the finite messaging boundary for the supplied object.
        AXError timeoutError = AXUIElementSetMessagingTimeout(
            application,
            LensAccessibilityMessagingTimeoutSeconds
        );
        if (timeoutError != kAXErrorSuccess) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"Unable to establish the Accessibility messaging timeout (%@ (%d)).",
                LensAXErrorName(timeoutError),
                timeoutError
            ]];
            CFRelease(application);
            return NO;
        }

        NSString *resolvedTitle = @"";
        CGRect resolvedFrame = CGRectZero;
        double resolutionScore = -1.0;
        AXUIElementRef window = LensCopyResolvedAXWindowForApplication(
            application,
            self.pickerTitle,
            self.pickerFrame,
            &resolvedTitle,
            &resolvedFrame,
            &resolutionScore,
            diagnostics
        );
        if (window == NULL) {
            CFRelease(application);
            return NO;
        }
        _applicationElement = application;
        _windowElement = window;
        self.initialResolutionScore = resolutionScore;
        return YES;
    }
}

- (AXUIElementRef _Nullable)copyResolvedWindow {
    @synchronized(self) {
        return _windowElement == NULL
            ? NULL
            : (AXUIElementRef)CFRetain(_windowElement);
    }
}

- (AXError)addObservationForElement:(AXUIElementRef)element
                        notification:(CFStringRef)notification
                                 kind:(LensNativeObservationKind)kind {
    if (_observer == NULL || _registrationCount >= LensMaximumObservationRegistrations) {
        return kAXErrorFailure;
    }
    LensNativeObservationRegistration *registration = &_registrations[_registrationCount];
    *registration = (LensNativeObservationRegistration){
        .owner = (__bridge void *)self,
        .observerEpoch = self.observerEpoch,
        .kind = kind,
        .element = element,
        .notification = notification,
        .registered = NO,
    };
    AXError error = AXObserverAddNotification(
        _observer,
        element,
        notification,
        registration
    );
    if (error == kAXErrorSuccess) {
        registration->registered = YES;
        _registrationCount += 1;
    } else {
        *registration = (LensNativeObservationRegistration){0};
    }
    return error;
}

- (NSDictionary *)startObservationWithContextID:(NSString *)contextID
                           sourceRegistrationID:(NSString *)sourceRegistrationID
                                  observerEpoch:(uint64_t)observerEpoch
                                       callback:(LensWindowObservationCallback)callback
                                callbackContext:(void *)callbackContext {
    NSAssert([NSThread isMainThread], @"AXObserver lifecycle is main-run-loop confined");
    if (_observer != NULL || self.acceptingCallbacks) {
        return @{
            @"status": @"error",
            @"message": @"The native source already owns an active observer."
        };
    }
    if (_applicationElement == NULL || _windowElement == NULL || callback == NULL ||
        contextID.length == 0 || sourceRegistrationID.length == 0 || observerEpoch == 0) {
        return @{
            @"status": @"error",
            @"message": @"The native source observation authority is incomplete."
        };
    }

    self.contextID = contextID;
    self.sourceRegistrationID = sourceRegistrationID;
    self.observerEpoch = observerEpoch;
    self.observationCallback = callback;
    self.observationContext = callbackContext;
    self.acceptingCallbacks = YES;
    _registrationCount = 0;

    AXError createError = AXObserverCreate(self.pid, LensNativeAXObserverCallback, &_observer);
    if (createError != kAXErrorSuccess || _observer == NULL) {
        self.acceptingCallbacks = NO;
        return @{
            @"status": @"error",
            @"message": [NSString stringWithFormat:
                @"Unable to create AXObserver (%@ (%d)).",
                LensAXErrorName(createError),
                createError
            ]
        };
    }

    // AXUIElement.h requires this source to be attached before notifications can arrive.
    CFRunLoopSourceRef source = AXObserverGetRunLoopSource(_observer);
    if (source == NULL) {
        [self stopObservation];
        return @{
            @"status": @"error",
            @"message": @"AXObserver returned no run-loop source."
        };
    }
    CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    self.runLoopSourceAttached = YES;

    NSMutableArray<NSString *> *registered = [NSMutableArray array];
    NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
    struct {
        AXUIElementRef element;
        CFStringRef notification;
        LensNativeObservationKind kind;
    } plan[] = {
        {_windowElement, kAXTitleChangedNotification, LensNativeObservationKindWindowTitleChanged},
        {_windowElement, kAXMovedNotification, LensNativeObservationKindWindowMoved},
        {_windowElement, kAXResizedNotification, LensNativeObservationKindWindowResized},
        {_windowElement, kAXUIElementDestroyedNotification, LensNativeObservationKindWindowDestroyed},
        {_applicationElement, kAXFocusedUIElementChangedNotification, LensNativeObservationKindApplicationChanged},
        {_applicationElement, kAXWindowCreatedNotification, LensNativeObservationKindApplicationChanged},
    };
    for (NSUInteger index = 0; index < sizeof(plan) / sizeof(plan[0]); index += 1) {
        AXError error = [self
            addObservationForElement:plan[index].element
                         notification:plan[index].notification
                                  kind:plan[index].kind];
        NSString *kindName = LensNativeObservationKindName(plan[index].kind);
        if (error == kAXErrorSuccess) {
            [registered addObject:kindName];
        } else {
            [diagnostics addObject:[NSString stringWithFormat:
                @"%@ registration failed with %@ (%d).",
                kindName,
                LensAXErrorName(error),
                error
            ]];
        }
    }
    if (_registrationCount == 0) {
        [self stopObservation];
        return @{
            @"status": @"error",
            @"message": @"The fixed target supports none of the bounded observation plan."
        };
    }
    return @{
        @"status": @"started",
        @"registered_notifications": registered,
        @"diagnostics": diagnostics
    };
}

- (void)receiveObservationRegistration:(LensNativeObservationRegistration *)registration {
    NSAssert([NSThread isMainThread], @"AXObserver callbacks are handled on the main run loop");
    if (!self.acceptingCallbacks || registration == NULL || !registration->registered ||
        registration->owner != (__bridge void *)self ||
        registration->observerEpoch != self.observerEpoch ||
        self.observationCallback == NULL) {
        return;
    }
    NSDictionary *event = @{
        @"operation_id": self.operationID,
        @"context_id": self.contextID,
        @"source_registration_id": self.sourceRegistrationID,
        @"observer_epoch": @(registration->observerEpoch),
        @"receipt": self.receipt,
        @"notification": LensNativeObservationKindName(registration->kind)
    };
    char *json = LensCopyJSONString(event);
    if (json != NULL) {
        self.observationCallback(json, self.observationContext);
        free(json);
    }
}

- (void)stopObservation {
    NSAssert([NSThread isMainThread], @"AXObserver teardown is main-run-loop confined");
    @synchronized(self) {
        self.acceptingCallbacks = NO;
        self.observationCallback = NULL;
        self.observationContext = NULL;
        for (NSUInteger index = 0; index < _registrationCount; index += 1) {
            LensNativeObservationRegistration *registration = &_registrations[index];
            registration->owner = NULL;
            if (_observer != NULL && registration->registered && registration->element != NULL &&
                registration->notification != NULL) {
                AXObserverRemoveNotification(
                    _observer,
                    registration->element,
                    registration->notification
                );
            }
            *registration = (LensNativeObservationRegistration){0};
        }
        _registrationCount = 0;
        if (_observer != NULL && self.runLoopSourceAttached) {
            CFRunLoopSourceRef source = AXObserverGetRunLoopSource(_observer);
            if (source != NULL) {
                CFRunLoopRemoveSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            }
        }
        self.runLoopSourceAttached = NO;
        if (_observer != NULL) {
            CFRelease(_observer);
            _observer = NULL;
        }
        self.contextID = nil;
        self.sourceRegistrationID = nil;
        self.observerEpoch = 0;
    }
}

- (void)releaseAllRetainedObjects {
    NSAssert([NSThread isMainThread], @"Selected-source release is main-thread confined");
    [self stopObservation];
    @synchronized(self) {
        if (_windowElement != NULL) {
            CFRelease(_windowElement);
            _windowElement = NULL;
        }
        if (_applicationElement != NULL) {
            CFRelease(_applicationElement);
            _applicationElement = NULL;
        }
        self.screenWindow = nil;
    }
}

@end

static void LensNativeAXObserverCallback(
    AXObserverRef observer,
    AXUIElementRef element,
    CFStringRef notification,
    void *_Nullable context
) {
    (void)observer;
    (void)element;
    (void)notification;
    if (context == NULL) {
        return;
    }
    LensNativeObservationRegistration *registration = context;
    if (registration->owner == NULL) {
        return;
    }
    LensNativeWindowSource *source = (__bridge LensNativeWindowSource *)registration->owner;
    [source receiveObservationRegistration:registration];
}

static NSString *LensTrimmedText(NSString *text) {
    if (text == nil) {
        return nil;
    }
    NSString *trimmed = [text stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    return trimmed.length > 0 ? trimmed : nil;
}

static void LensAppendTextFragment(
    NSMutableArray<NSString *> *fragments,
    NSMutableSet<NSString *> *seenInNode,
    NSString *candidate,
    NSUInteger maxBytes,
    NSUInteger *textBytes,
    BOOL *truncated
) {
    NSString *text = LensTrimmedText(candidate);
    if (text == nil || [seenInNode containsObject:text] || *truncated) {
        return;
    }

    NSUInteger candidateBytes = [text lengthOfBytesUsingEncoding:NSUTF8StringEncoding];
    if (*textBytes + candidateBytes > maxBytes) {
        NSUInteger remaining = maxBytes > *textBytes ? maxBytes - *textBytes : 0;
        if (remaining > 0) {
            NSUInteger length = MIN(text.length, remaining);
            NSString *prefix = [text substringToIndex:length];
            while (prefix.length > 0 && [prefix lengthOfBytesUsingEncoding:NSUTF8StringEncoding] > remaining) {
                prefix = [prefix substringToIndex:prefix.length - 1];
            }
            if (prefix.length > 0) {
                [fragments addObject:prefix];
                *textBytes += [prefix lengthOfBytesUsingEncoding:NSUTF8StringEncoding];
            }
        }
        *truncated = YES;
        return;
    }

    [seenInNode addObject:text];
    [fragments addObject:text];
    *textBytes += candidateBytes;
}

static BOOL LensIsWindowChromeText(NSString *candidate, NSString *windowTitle) {
    NSString *text = LensTrimmedText(candidate);
    NSString *title = LensTrimmedText(windowTitle);
    return text != nil && title != nil && [text caseInsensitiveCompare:title] == NSOrderedSame;
}

static BOOL LensIsUsefulNodeText(NSString *candidate, NSString *windowTitle) {
    return LensTrimmedText(candidate) != nil && !LensIsWindowChromeText(candidate, windowTitle);
}

static NSDictionary *LensExtractionUnavailableWithDiagnostics(NSArray<NSString *> *diagnostics) {
    return @{
        @"quality": @"unavailable",
        @"nodes": @[],
        @"text": @"",
        @"metrics": @{
            @"visited_nodes": @0,
            @"text_bytes": @0,
            @"offscreen_text_nodes": @0,
            @"virtualization_signals": @0,
            @"truncated_nodes": @NO,
            @"truncated_text": @NO,
            @"children_read_errors": @0,
            @"resource_ref_count": @0,
            @"resource_uri_bytes": @0,
            @"omitted_resource_refs": @0,
            @"resource_read_errors": @0
        },
        @"diagnostics": diagnostics
    };
}

static NSDictionary *LensExtractionUnavailable(NSString *diagnostic) {
    return LensExtractionUnavailableWithDiagnostics(@[diagnostic]);
}

static NSDictionary *LensExtractResolvedWindow(
    AXUIElementRef resolvedWindow,
    NSString *selectedTitle,
    CGRect selectedFrame,
    NSString *applicationName,
    NSString *resolvedTitle,
    CGRect resolvedFrame,
    double bestScore,
    NSMutableArray<NSString *> *diagnostics,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
) {
        NSMutableArray *nodes = [NSMutableArray array];
        NSMutableArray<NSString *> *fragments = [NSMutableArray array];
        NSMutableArray<NSDictionary *> *queue = [NSMutableArray arrayWithObject:@{
            @"element": (__bridge id)resolvedWindow,
            @"id": @"node-000000",
            @"order": @0,
            @"depth": @0
        }];
        CFMutableSetRef scheduled = CFSetCreateMutable(NULL, 0, &kCFTypeSetCallBacks);
        CFSetAddValue(scheduled, resolvedWindow);

        NSUInteger cursor = 0;
        NSUInteger nextNodeOrder = 1;
        NSUInteger textBytes = 0;
        NSUInteger offscreenTextNodes = 0;
        NSUInteger virtualizationSignals = 0;
        NSUInteger childrenReadErrors = 0;
        NSUInteger resourceRefCount = 0;
        NSUInteger resourceURIBytes = 0;
        NSUInteger omittedResourceRefs = 0;
        NSUInteger resourceReadErrors = 0;
        BOOL hasUsefulNodeText = NO;
        BOOL truncatedNodes = NO;
        BOOL truncatedText = NO;

        while (cursor < queue.count) {
            if (nodes.count >= maxNodes) {
                truncatedNodes = YES;
                break;
            }

            NSDictionary *entry = queue[cursor++];
            AXUIElementRef element = (__bridge AXUIElementRef)entry[@"element"];
            NSString *nodeID = entry[@"id"];
            NSString *parentID = entry[@"parent_id"];
            NSUInteger order = [entry[@"order"] unsignedIntegerValue];
            NSUInteger depth = [entry[@"depth"] unsignedIntegerValue];
            NSString *role = LensAXString(element, kAXRoleAttribute);
            NSString *subrole = LensAXString(element, kAXSubroleAttribute);
            NSString *title = LensAXString(element, kAXTitleAttribute);
            NSString *value = LensAXString(element, kAXValueAttribute);
            NSString *description = LensAXString(element, kAXDescriptionAttribute);
            NSString *axURL = LensAXURIString(
                element, kAXURLAttribute, NO, &resourceReadErrors
            );
            NSString *axDocument = depth == 0 || [role isEqualToString:(__bridge NSString *)kAXWindowRole]
                ? LensAXURIString(element, kAXDocumentAttribute, YES, &resourceReadErrors)
                : nil;
            CGRect frame = CGRectZero;
            BOOL hasFrame = LensAXFrame(element, &frame);

            NSMutableDictionary *node = [NSMutableDictionary dictionaryWithDictionary:@{
                @"id": nodeID,
                @"order": @(order),
                @"depth": @(depth),
                @"children": @[]
            }];
            if (parentID != nil) node[@"parent_id"] = parentID;
            if (role.length > 0) node[@"role"] = role;
            if (subrole.length > 0) node[@"subrole"] = subrole;
            if (title.length > 0) node[@"title"] = title;
            if (value.length > 0) node[@"value"] = value;
            if (description.length > 0) node[@"description"] = description;
            if (hasFrame) node[@"bounds"] = LensFrameDictionary(frame);
            NSMutableArray<NSDictionary *> *resourceRefs = [NSMutableArray array];
            LensAppendAXResourceReference(
                resourceRefs, axURL, @"AXURL",
                maxResourceRefs, maxResourceURIBytes, maxTotalResourceURIBytes,
                &resourceRefCount, &resourceURIBytes, &omittedResourceRefs
            );
            LensAppendAXResourceReference(
                resourceRefs, axDocument, @"AXDocument",
                maxResourceRefs, maxResourceURIBytes, maxTotalResourceURIBytes,
                &resourceRefCount, &resourceURIBytes, &omittedResourceRefs
            );
            if (resourceRefs.count > 0) {
                node[@"resource_refs"] = resourceRefs;
            }
            [nodes addObject:node];

            NSUInteger fragmentsBefore = fragments.count;
            // Window-title chrome is target metadata, not document content. Keeping it in the node
            // model helps resolver diagnostics, while excluding root attributes and repeated title
            // descendants ensures a WebView exposing only its chrome is classified as unavailable.
            if (depth > 0) {
                hasUsefulNodeText = hasUsefulNodeText
                    || LensIsUsefulNodeText(title, selectedTitle)
                    || LensIsUsefulNodeText(value, selectedTitle)
                    || LensIsUsefulNodeText(description, selectedTitle);
                // AX commonly repeats one semantic value across a node's title, value, and
                // description attributes. Deduplicate only within this node so equal text on
                // distinct rows, cells, or list items retains its structural meaning.
                NSMutableSet<NSString *> *seenInNode = [NSMutableSet set];
                if (!LensIsWindowChromeText(title, selectedTitle)) {
                    LensAppendTextFragment(fragments, seenInNode, title, maxTextBytes, &textBytes, &truncatedText);
                }
                if (!LensIsWindowChromeText(value, selectedTitle)) {
                    LensAppendTextFragment(fragments, seenInNode, value, maxTextBytes, &textBytes, &truncatedText);
                }
                if (!LensIsWindowChromeText(description, selectedTitle)) {
                    LensAppendTextFragment(fragments, seenInNode, description, maxTextBytes, &textBytes, &truncatedText);
                }
            }
            BOOL addedText = fragments.count > fragmentsBefore;
            if (addedText && hasFrame && !CGRectIntersectsRect(selectedFrame, frame)) {
                offscreenTextNodes++;
            }

            NSUInteger pendingNodes = queue.count - cursor;
            NSUInteger remainingNodeBudget = 0;
            if (nodes.count < maxNodes && pendingNodes < maxNodes - nodes.count) {
                remainingNodeBudget = maxNodes - nodes.count - pendingNodes;
            }
            BOOL childrenTruncated = NO;
            NSArray *children = LensAXElementsForArrayAttribute(
                element,
                kAXChildrenAttribute,
                remainingNodeBudget,
                NULL,
                &childrenTruncated,
                &childrenReadErrors
            );
            if (childrenTruncated) {
                truncatedNodes = YES;
            }
            NSNumber *rowCount = LensAXNumber(element, kAXRowCountAttribute);
            if (rowCount != nil) {
                NSUInteger reportedRows = 0;
                BOOL rowsTruncated = NO;
                NSArray *rows = LensAXElementsForArrayAttribute(
                    element,
                    kAXRowsAttribute,
                    children.count == 0 ? remainingNodeBudget : 0,
                    &reportedRows,
                    &rowsTruncated,
                    &childrenReadErrors
                );
                if (rowCount.unsignedIntegerValue > reportedRows) {
                    virtualizationSignals++;
                }
                if (children.count == 0) {
                    if (rowsTruncated) {
                        truncatedNodes = YES;
                    }
                    if (rows.count > 0) {
                        children = rows;
                    }
                }
            }

            NSMutableArray<NSString *> *childIDs = [NSMutableArray array];
            for (id child in children) {
                CFTypeRef childType = (__bridge CFTypeRef)child;
                if (CFGetTypeID(childType) == AXUIElementGetTypeID()
                    && !CFSetContainsValue(scheduled, childType)) {
                    NSUInteger scheduledNodes = queue.count - cursor;
                    NSUInteger availableSlots = nodes.count < maxNodes
                        ? maxNodes - nodes.count
                        : 0;
                    if (scheduledNodes >= availableSlots) {
                        truncatedNodes = YES;
                        break;
                    }
                    NSString *childID = [NSString stringWithFormat:
                        @"node-%06lu",
                        (unsigned long)nextNodeOrder
                    ];
                    nextNodeOrder += 1;
                    CFSetAddValue(scheduled, childType);
                    [childIDs addObject:childID];
                    [queue addObject:@{
                        @"element": child,
                        @"id": childID,
                        @"parent_id": nodeID,
                        @"order": @(nextNodeOrder - 1),
                        @"depth": @(depth + 1)
                    }];
                }
            }
            node[@"children"] = childIDs;
        }

        CFRelease(scheduled);

        if (bestScore < 100.0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"AXWindow match used title/bounds similarity (score %.1f); public AX APIs expose no SCWindow windowID mapping.",
                bestScore
            ]];
        }
        if (offscreenTextNodes > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"Accessibility exposed %lu text-bearing nodes outside the selected window bounds.",
                (unsigned long)offscreenTextNodes
            ]];
        }
        if (virtualizationSignals > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"Detected %lu concrete row-count virtualization signals; partial extraction is expected.",
                (unsigned long)virtualizationSignals
            ]];
        }
        if (truncatedNodes) {
            [diagnostics addObject:@"Traversal stopped at the configured node limit."];
        }
        if (truncatedText) {
            [diagnostics addObject:@"Text collection stopped at the configured UTF-8 byte limit."];
        }
        if (childrenReadErrors > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"%lu Accessibility child-array reads could not be completed.",
                (unsigned long)childrenReadErrors
            ]];
        }
        if (omittedResourceRefs > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"%lu URI resource references were omitted whole after reaching the configured count or UTF-8 byte limits.",
                (unsigned long)omittedResourceRefs
            ]];
        }
        if (resourceReadErrors > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"%lu URI-valued Accessibility attributes could not be read or did not contain an absolute URI of their declared type.",
                (unsigned long)resourceReadErrors
            ]];
        }

        NSString *text = [fragments componentsJoinedByString:@"\n"];
        // The flat text buffer is retained only for diagnostic compatibility. Reaching its byte
        // limit must not discard a structurally useful AX graph or its AXImage capture plan.
        BOOL hasUsefulContent = hasUsefulNodeText || resourceRefCount > 0;
        NSString *quality = !hasUsefulContent
            ? @"unavailable"
            : (truncatedNodes || truncatedText || childrenReadErrors > 0
                || virtualizationSignals > 0 || omittedResourceRefs > 0
                || resourceReadErrors > 0
                ? @"partial"
                : @"full");
        if (!hasUsefulContent) {
            [diagnostics addObject:@"The resolved AXWindow contains no useful text or URI resource references."];
        }

        pid_t resolvedPID = 0;
        AXUIElementGetPid(resolvedWindow, &resolvedPID);
        NSString *applicationID = LensStringOrEmpty(
            [NSRunningApplication runningApplicationWithProcessIdentifier:resolvedPID].bundleIdentifier);
        NSDictionary *result = @{
            @"quality": quality,
            @"resolved_window": @{
                @"facts": @{
                    @"title": resolvedTitle,
                    @"application_name": applicationName,
                    @"application_id": applicationID,
                    @"frame": LensFrameDictionary(resolvedFrame)
                },
                @"resolution_score": @(bestScore)
            },
            @"nodes": nodes,
            @"text": text,
            @"metrics": @{
                @"visited_nodes": @(nodes.count),
                @"text_bytes": @(textBytes),
                @"offscreen_text_nodes": @(offscreenTextNodes),
                @"virtualization_signals": @(virtualizationSignals),
                @"truncated_nodes": @(truncatedNodes),
                @"truncated_text": @(truncatedText),
                @"children_read_errors": @(childrenReadErrors),
                @"resource_ref_count": @(resourceRefCount),
                @"resource_uri_bytes": @(resourceURIBytes),
                @"omitted_resource_refs": @(omittedResourceRefs),
                @"resource_read_errors": @(resourceReadErrors)
            },
            @"diagnostics": diagnostics
        };
        return result;
}

char *lens_extract_window_json(
    int32_t pid,
    const char *selectedTitleCString,
    const char *applicationNameCString,
    double selectedX,
    double selectedY,
    double selectedWidth,
    double selectedHeight,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) {
            return LensCopyJSONString(LensExtractionUnavailable(
                @"Accessibility permission is not granted to Lens."
            ));
        }

        NSString *selectedTitle = selectedTitleCString == NULL
            ? @""
            : [NSString stringWithUTF8String:selectedTitleCString];
        NSString *applicationName = applicationNameCString == NULL
            ? @""
            : [NSString stringWithUTF8String:applicationNameCString];
        NSString *runningApplicationName = LensStringOrEmpty(
            [NSRunningApplication runningApplicationWithProcessIdentifier:pid].localizedName
        );
        if (runningApplicationName.length > 0) {
            applicationName = runningApplicationName;
        }
        CGRect selectedFrame = CGRectMake(selectedX, selectedY, selectedWidth, selectedHeight);
        double bestScore = -1.0;
        NSString *resolvedTitle = @"";
        CGRect resolvedFrame = CGRectZero;
        NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
        AXUIElementRef resolvedWindow = LensCopyResolvedAXWindow(
            pid,
            selectedTitle,
            selectedFrame,
            &resolvedTitle,
            &resolvedFrame,
            &bestScore,
            diagnostics
        );
        if (resolvedWindow == NULL) {
            return LensCopyJSONString(LensExtractionUnavailableWithDiagnostics(diagnostics));
        }
        NSDictionary *result = LensExtractResolvedWindow(
            resolvedWindow,
            selectedTitle,
            selectedFrame,
            applicationName,
            resolvedTitle,
            resolvedFrame,
            bestScore,
            diagnostics,
            maxNodes,
            maxTextBytes,
            maxResourceRefs,
            maxResourceURIBytes,
            maxTotalResourceURIBytes
        );
        CFRelease(resolvedWindow);
        return LensCopyJSONString(result);
    }
}

static id LensGeometryObservation(CGRect before, CGRect after) {
    if (!LensFinitePositiveRect(before) || !LensFinitePositiveRect(after)) return NSNull.null;
    return @{ @"before": LensFrameDictionary(before), @"after": LensFrameDictionary(after) };
}

#if defined(LENS_NATIVE_TESTING)
bool lens_test_extraction_geometry_observation(void) {
    CGRect before = CGRectMake(-12.25, -5.5, 20.5, 10.25);
    CGRect after = CGRectMake(-11.25, -5.5, 20.5, 10.25);
    NSDictionary *stable = LensGeometryObservation(before, before);
    NSDictionary *changed = LensGeometryObservation(before, after);
    return [stable[@"before"] isEqual:LensFrameDictionary(before)]
        && [stable[@"after"] isEqual:LensFrameDictionary(before)]
        && [changed[@"before"] isEqual:LensFrameDictionary(before)]
        && [changed[@"after"] isEqual:LensFrameDictionary(after)]
        && ![changed[@"before"] isEqual:changed[@"after"]]
        && LensGeometryObservation(before, CGRectNull) == NSNull.null
        && LensGeometryObservation(CGRectMake(NAN, 0, 1, 1), after) == NSNull.null
        && LensGeometryObservation(CGRectZero, after) == NSNull.null;
}
#endif

static char *LensRegisteredExtractionUnavailable(NSDictionary *unavailable) {
    NSMutableDictionary *result = [unavailable mutableCopy];
    result[@"geometry_observation"] = NSNull.null;
    return LensCopyJSONString(result);
}

static char *LensExtractRegisteredSourceJSON(
    LensNativeWindowSource *source,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
) {
    @autoreleasepool {
        if (source == nil) {
            return LensRegisteredExtractionUnavailable(LensExtractionUnavailable(
                @"The exact operation-scoped selected window is unavailable."
            ));
        }
        NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
        if (![source ensureResolvedWindowWithDiagnostics:diagnostics]) {
            return LensRegisteredExtractionUnavailable(LensExtractionUnavailableWithDiagnostics(diagnostics));
        }
        AXUIElementRef resolvedWindow = [source copyResolvedWindow];
        if (resolvedWindow == NULL) {
            return LensRegisteredExtractionUnavailable(LensExtractionUnavailable(
                @"The promoted AXWindow is unavailable."
            ));
        }
        NSString *currentTitle = LensAXString(resolvedWindow, kAXTitleAttribute) ?: @"";
        NSString *currentApplicationName = LensStringOrEmpty(
            [NSRunningApplication
                runningApplicationWithProcessIdentifier:source.pid].localizedName
        );
        if (currentApplicationName.length == 0) {
            currentApplicationName = source.pickerApplicationName;
        }
        CGRect currentFrame = CGRectZero;
        if (!LensAXFrame(resolvedWindow, &currentFrame) || !LensFinitePositiveRect(currentFrame)) {
            CFRelease(resolvedWindow);
            [diagnostics addObject:@"The promoted AXWindow did not expose a current frame; stale picker geometry was not published as current facts."];
            return LensRegisteredExtractionUnavailable(LensExtractionUnavailableWithDiagnostics(diagnostics));
        }
        [diagnostics addObject:@"Extraction used the exact promoted AXWindow without heuristic re-resolution."];
        NSDictionary *result = LensExtractResolvedWindow(
            resolvedWindow,
            currentTitle,
            currentFrame,
            currentApplicationName,
            currentTitle,
            currentFrame,
            source.initialResolutionScore,
            diagnostics,
            maxNodes,
            maxTextBytes,
            maxResourceRefs,
            maxResourceURIBytes,
            maxTotalResourceURIBytes
        );
        CGRect afterFrame = CGRectNull;
        BOOL afterAvailable = LensAXFrame(resolvedWindow, &afterFrame) && LensFinitePositiveRect(afterFrame);
        CFRelease(resolvedWindow);
        NSMutableDictionary *observed = [result mutableCopy];
        observed[@"geometry_observation"] = LensGeometryObservation(currentFrame, afterAvailable ? afterFrame : CGRectNull);
        if (!afterAvailable) {
            NSMutableArray *messages = [result[@"diagnostics"] mutableCopy] ?: [NSMutableArray array];
            [messages addObject:@"The retained AXWindow had no finite frame after traversal; geometry observation is unavailable."];
            observed[@"diagnostics"] = messages;
        }
        return LensCopyJSONString(observed);
    }
}

static char *LensStartSourceObservationJSON(
    LensNativeWindowSource *source,
    const char *operationIDCString,
    const char *contextIDCString,
    const char *sourceRegistrationIDCString,
    uint64_t observerEpoch,
    LensWindowObservationCallback callback,
    void *context,
    bool *startedOut
) {
    @autoreleasepool {
        if (startedOut == NULL) {
            return LensCopyJSONString(@{
                @"status": @"error",
                @"message": @"The source observation ownership output is required."
            });
        }
        *startedOut = false;
        NSString *operationID = operationIDCString == NULL
            ? nil
            : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
        NSString *contextID = contextIDCString == NULL
            ? nil
            : [NSString stringWithUTF8String:contextIDCString];
        NSString *sourceRegistrationID = sourceRegistrationIDCString == NULL
            ? nil
            : [NSString stringWithUTF8String:sourceRegistrationIDCString];
        if (operationID.length == 0 || contextID.length == 0 ||
            sourceRegistrationID.length == 0 || observerEpoch == 0 || callback == NULL) {
            return LensCopyJSONString(@{
                @"status": @"error",
                @"message": @"The source observation request has incomplete authority."
            });
        }
        if (source == nil) {
            return LensCopyJSONString(@{
                @"status": @"error",
                @"message": @"The exact operation-scoped selected window is unavailable."
            });
        }
        NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
        if (![source ensureResolvedWindowWithDiagnostics:diagnostics]) {
            return LensCopyJSONString(@{
                @"status": @"error",
                @"message": diagnostics.count > 0
                    ? [diagnostics componentsJoinedByString:@"; "]
                    : @"The exact AXWindow could not be promoted."
            });
        }

        __block NSDictionary *reply = nil;
        LensPerformSyncOnMainThread(^{
            LensEnsureWindowRegistries();
            if (LensReceiptSourceRegistry[LensObservationRegistryKey(operationID, source.receipt)] != source ||
                !LensWindowOperations[operationID].open) {
                reply = @{ @"status": @"error", @"message": @"The selected source was released before observation admission." };
                return;
            }
            NSString *sourceKey = LensObservationRegistryKey(
                operationID,
                sourceRegistrationID
            );
            LensNativeWindowSource *existing = LensObservationSourceRegistry[sourceKey];
            if (existing != nil && existing != source) {
                reply = @{
                    @"status": @"error",
                    @"message": @"The source registration identity is already owned by another target."
                };
                return;
            }
            reply = [source
                startObservationWithContextID:contextID
                         sourceRegistrationID:sourceRegistrationID
                                observerEpoch:observerEpoch
                                     callback:callback
                              callbackContext:context];
            if ([reply[@"status"] isEqualToString:@"started"]) {
                LensObservationSourceRegistry[sourceKey] = source;
                *startedOut = true;
            }
        });
        NSDictionary *terminalReply = reply ?: @{
            @"status": @"error",
            @"message": @"Native source observation produced no terminal result."
        };
        char *json = LensCopyJSONString(terminalReply);
        if (json == NULL && *startedOut) {
            // A started observer must never outlive a caller that could not receive ownership.
            // Tear down the exact registration before returning an allocation failure.
            LensPerformSyncOnMainThread(^{
                LensEnsureWindowRegistries();
                NSString *sourceKey = LensObservationRegistryKey(
                    operationID,
                    sourceRegistrationID
                );
                LensNativeWindowSource *startedSource =
                    LensObservationSourceRegistry[sourceKey];
                if (startedSource == source) {
                    [startedSource stopObservation];
                    [LensObservationSourceRegistry removeObjectForKey:sourceKey];
                }
            });
            *startedOut = false;
        }
        return json;
    }
}

static NSString *LensNativeString(const char *value) {
    return value == NULL ? @"" : ([NSString stringWithUTF8String:value].lowercaseString ?: @"");
}

/* Authority decorates failures as well as successes; no lookup is retried by numeric ID. */
static BOOL LensCanonicalReadSequence(const char *sequence) {
    if (sequence == NULL || sequence[0] < '1' || sequence[0] > '9') return NO;
    uint64_t value = 0;
    for (const unsigned char *cursor = (const unsigned char *)sequence; *cursor != 0; cursor++) {
        if (*cursor < '0' || *cursor > '9') return NO;
        uint64_t digit = (uint64_t)(*cursor - '0');
        if (value > (UINT64_MAX - digit) / 10) return NO;
        value = value * 10 + digit;
    }
    return YES;
}

static char *LensAttachReceiptRead(char *json, NSString *operationID, NSString *receipt, const char *sequence) {
    if (json == NULL) return NULL;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    free(json);
    id decoded = data == nil ? nil : [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingMutableContainers error:nil];
    if (![decoded isKindOfClass:NSMutableDictionary.class]) return NULL;
    decoded[@"read"] = @{ @"target": @{ @"operation_id": operationID, @"receipt": receipt },
        @"sequence": [NSString stringWithUTF8String:sequence] };
    return LensCopyJSONString(decoded);
}

char *lens_extract_registered_window_json(
    const char *operationID, uint32_t windowID, uint32_t maxNodes, uint32_t maxTextBytes,
    uint32_t maxResourceRefs, uint32_t maxResourceURIBytes, uint32_t maxTotalResourceURIBytes
) {
    return LensExtractRegisteredSourceJSON(LensWindowSourceForIdentity(LensNativeString(operationID), windowID),
        maxNodes, maxTextBytes, maxResourceRefs, maxResourceURIBytes, maxTotalResourceURIBytes);
}

char *lens_extract_receipt_window_json(
    const char *operationID, const char *receipt, const char *readSequence, uint32_t maxNodes, uint32_t maxTextBytes,
    uint32_t maxResourceRefs, uint32_t maxResourceURIBytes, uint32_t maxTotalResourceURIBytes
) {
    if (!LensCanonicalReadSequence(readSequence)) return NULL;
    NSString *operation = LensNativeString(operationID), *target = LensNativeString(receipt);
    return LensAttachReceiptRead(LensExtractRegisteredSourceJSON(LensWindowSourceForReceipt(operation, target),
        maxNodes, maxTextBytes, maxResourceRefs, maxResourceURIBytes, maxTotalResourceURIBytes), operation, target, readSequence);
}

char *lens_capture_registered_window_regions_json(
    const char *operationID, uint32_t windowID, const char *requestsJSON, uint32_t maxLongEdge,
    uint32_t maxPixels, uint32_t maxAttachmentBytes, uint32_t maxTotalBytes
) {
    return LensCaptureRegisteredSourceJSON(LensWindowSourceForIdentity(LensNativeString(operationID), windowID),
        requestsJSON, maxLongEdge, maxPixels, maxAttachmentBytes, maxTotalBytes);
}

char *lens_capture_receipt_window_regions_json(
    const char *operationID, const char *receipt, const char *readSequence, const char *requestsJSON, uint32_t maxLongEdge,
    uint32_t maxPixels, uint32_t maxAttachmentBytes, uint32_t maxTotalBytes
) {
    if (!LensCanonicalReadSequence(readSequence)) return NULL;
    NSString *operation = LensNativeString(operationID), *target = LensNativeString(receipt);
    return LensAttachReceiptRead(LensCaptureRegisteredSourceJSON(LensWindowSourceForReceipt(operation, target),
        requestsJSON, maxLongEdge, maxPixels, maxAttachmentBytes, maxTotalBytes), operation, target, readSequence);
}

char *lens_start_window_observation_json(
    const char *operationID, const char *contextID, const char *sourceRegistrationID,
    uint64_t observerEpoch, uint32_t windowID, LensWindowObservationCallback callback,
    void *context, bool *startedOut
) {
    return LensStartSourceObservationJSON(LensWindowSourceForIdentity(LensNativeString(operationID), windowID),
        operationID, contextID, sourceRegistrationID, observerEpoch, callback, context, startedOut);
}

char *lens_start_receipt_window_observation_json(
    const char *operationID, const char *contextID, const char *sourceRegistrationID,
    uint64_t observerEpoch, const char *receipt, LensWindowObservationCallback callback,
    void *context, bool *startedOut
) {
    return LensStartSourceObservationJSON(LensWindowSourceForReceipt(LensNativeString(operationID), LensNativeString(receipt)),
        operationID, contextID, sourceRegistrationID, observerEpoch, callback, context, startedOut);
}

bool lens_stop_window_observation(
    const char *operationIDCString,
    const char *sourceRegistrationIDCString
) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    NSString *sourceRegistrationID = sourceRegistrationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:sourceRegistrationIDCString];
    if (operationID.length == 0 || sourceRegistrationID.length == 0) {
        return false;
    }
    __block BOOL stopped = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        NSString *sourceKey = LensObservationRegistryKey(operationID, sourceRegistrationID);
        LensNativeWindowSource *source = LensObservationSourceRegistry[sourceKey];
        if (source == nil) {
            return;
        }
        [source stopObservation];
        [LensObservationSourceRegistry removeObjectForKey:sourceKey];
        stopped = YES;
    });
    return stopped;
}

static bool LensReleaseWindowSource(const char *operationIDCString, uint32_t windowID, NSString *receipt) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    if (operationID.length == 0) {
        return false;
    }
    __block BOOL released = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        LensNativeWindowSource *source = receipt == nil
            ? LensWindowSourceRegistry[LensWindowRegistryKey(operationID, windowID)]
            : LensReceiptSourceRegistry[LensObservationRegistryKey(operationID, receipt)];
        if (source == nil) {
            return;
        }
        NSString *windowKey = LensWindowRegistryKey(operationID, source.windowID);
        NSString *sourceRegistrationID = source.sourceRegistrationID;
        [source releaseAllRetainedObjects];
        if (sourceRegistrationID.length > 0) {
            [LensObservationSourceRegistry removeObjectForKey:
                LensObservationRegistryKey(operationID, sourceRegistrationID)];
        }
        [LensWindowSourceRegistry removeObjectForKey:windowKey];
        [LensReceiptSourceRegistry removeObjectForKey:LensObservationRegistryKey(operationID, source.receipt)];
        released = YES;
    });
    return released;
}

bool lens_release_registered_window(const char *operationID, uint32_t windowID) {
    return LensReleaseWindowSource(operationID, windowID, nil);
}

bool lens_release_receipt_window(const char *operationID, const char *receipt) {
    NSString *target = LensNativeString(receipt);
    return target.length > 0 && LensReleaseWindowSource(operationID, 0, target);
}

bool lens_release_window_operation(const char *operationIDCString) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    if (operationID.length == 0) {
        return false;
    }
    __block BOOL released = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        LensWindowOperation *operation = LensWindowOperations[operationID];
        operation.open = NO;
        operation.invocationID = nil;
        operation.pendingSource = nil;
        [LensWindowOperations removeObjectForKey:operationID];
        LensContentPickerCoordinator *coordinator = LensActiveContentPickerCoordinator;
        released = YES;
        NSArray<NSString *> *windowKeys = [LensWindowSourceRegistry.allKeys copy];
        for (NSString *windowKey in windowKeys) {
            LensNativeWindowSource *source = LensWindowSourceRegistry[windowKey];
            if (![source.operationID isEqualToString:operationID]) {
                continue;
            }
            NSString *sourceRegistrationID = source.sourceRegistrationID;
            [source releaseAllRetainedObjects];
            if (sourceRegistrationID.length > 0) {
                [LensObservationSourceRegistry removeObjectForKey:
                    LensObservationRegistryKey(operationID, sourceRegistrationID)];
            }
            [LensWindowSourceRegistry removeObjectForKey:windowKey];
            [LensReceiptSourceRegistry removeObjectForKey:LensObservationRegistryKey(operationID, source.receipt)];
            released = YES;
        }
        if (operation != nil && coordinator.operation == operation) {
            [coordinator deliver:@{ @"status": @"cancelled" }];
        }
    });
    return released;
}

#if defined(LENS_NATIVE_TESTING)
char *lens_test_copy_picker_receipt(const char *operationID, uint32_t windowID) {
    LensNativeWindowSource *source = LensWindowSourceForIdentity(LensNativeString(operationID), windowID);
    return source == nil ? NULL : strdup(source.receipt.UTF8String);
}

/* No picker presentation or AX calls: exercise the exact admission state transitions. */
bool lens_test_window_operation_lifecycle(void *screenCaptureWindow) {
    if (screenCaptureWindow == NULL) return false;
    __block BOOL passed = NO;
    LensPerformSyncOnMainThread(^{
        SCWindow *window = (__bridge SCWindow *)screenCaptureWindow;
        NSString *operationID = NSUUID.UUID.UUIDString.lowercaseString;
        NSString *invocationID = NSUUID.UUID.UUIDString;
        BOOL valid = lens_open_window_operation(operationID.UTF8String) &&
            !lens_open_window_operation(operationID.UTF8String);
        LensWindowOperation *operation = LensWindowOperations[operationID];
        operation.invocationID = invocationID;
        valid &= LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= LensWindowSourceForIdentity(operationID, window.windowID) == nil;
        valid &= !lens_accept_window_picker_invocation(operationID.UTF8String, "wrong-invocation");
        valid &= lens_cancel_window_picker_invocation(operationID.UTF8String, invocationID.UTF8String);
        valid &= operation.pendingSource == nil && operation.invocationID == nil;
        valid &= LensWindowSourceForIdentity(operationID, window.windowID) == nil;
        operation.invocationID = invocationID;
        valid &= LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= lens_accept_window_picker_invocation(operationID.UTF8String, invocationID.UTF8String);
        LensNativeWindowSource *accepted = LensWindowSourceForIdentity(operationID, window.windowID);
        valid &= accepted != nil;
        NSString *receipt = accepted.receipt;
        uint32_t ordinal = accepted.selectionOrdinal;
        valid &= ordinal > 0 && [[NSUUID alloc] initWithUUIDString:receipt] != nil;
        valid &= LensWindowSourceForReceipt(operationID.uppercaseString, receipt.uppercaseString) == accepted;
        valid &= LensWindowSourceForReceipt(NSUUID.UUID.UUIDString, receipt) == nil;
        valid &= LensWindowSourceForReceipt(operationID, NSUUID.UUID.UUIDString) == nil;
        valid &= !lens_release_receipt_window(NSUUID.UUID.UUIDString.UTF8String, receipt.UTF8String);
        operation.invocationID = invocationID;
        // This test double supplies windowID but is deliberately not the retained SCWindow.
        // Rejection must occur before any other SCWindow property is accessed.
        valid &= !LensStagePickerWindow(operation, operationID, invocationID, (SCWindow *)(id)accepted);
        valid &= LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= operation.pendingSource == accepted;
        valid &= [operation.pendingSource.receipt isEqual:receipt] && operation.pendingSource.selectionOrdinal == ordinal;
        valid &= lens_cancel_window_picker_invocation(operationID.UTF8String, invocationID.UTF8String);
        valid &= LensWindowSourceForIdentity(operationID, window.windowID) == accepted;
        valid &= lens_release_receipt_window(operationID.UTF8String, receipt.UTF8String);
        valid &= LensWindowSourceForReceipt(operationID, receipt) == nil;
        valid &= !lens_release_receipt_window(operationID.UTF8String, receipt.UTF8String);
        operation.invocationID = invocationID;
        valid &= LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= ![operation.pendingSource.receipt isEqual:receipt] && operation.pendingSource.selectionOrdinal > ordinal;
        valid &= lens_accept_window_picker_invocation(operationID.UTF8String, invocationID.UTF8String);
        NSString *replacementReceipt = LensWindowSourceForIdentity(operationID, window.windowID).receipt;
        valid &= lens_release_receipt_window(operationID.UTF8String, replacementReceipt.UTF8String);
        operation.lastSelectionOrdinal = UINT32_MAX;
        operation.invocationID = invocationID;
        valid &= !LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= operation.lastSelectionOrdinal == UINT32_MAX;
        operation.invocationID = invocationID;
        valid &= lens_release_window_operation(operationID.UTF8String);
        valid &= !operation.open && LensWindowSourceForIdentity(operationID, window.windowID) == nil;
        valid &= lens_open_window_operation(operationID.UTF8String);
        valid &= LensWindowOperations[operationID] != operation;
        valid &= !LensStagePickerWindow(operation, operationID, invocationID, window);
        valid &= !lens_accept_window_picker_invocation(operationID.UTF8String, invocationID.UTF8String);
        valid &= lens_release_window_operation(operationID.UTF8String);
        valid &= lens_release_window_operation(operationID.UTF8String);
        passed = valid;
    });
    return passed;
}

bool lens_test_store_picker_window(const char *operationIDCString, void *screenCaptureWindow) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString].lowercaseString;
    if (operationID.length == 0 || screenCaptureWindow == NULL) {
        return false;
    }
    __block BOOL stored = NO;
    LensPerformSyncOnMainThread(^{
        SCWindow *window = (__bridge SCWindow *)screenCaptureWindow;
        if (LensWindowOperations[operationID] == nil) lens_open_window_operation(operationIDCString);
        stored = LensStorePickerWindow(operationID, window);
    });
    return stored;
}
#endif

void lens_free_string(char *value) {
    free(value);
}
