#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
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


// Validate and decode the complete tree before mutating any attached menu item.
static NSArray *LensPrepareMenuPresentation(NSMenu *menu, id values, NSUInteger depth,
                                           NSUInteger *count, NSMutableDictionary *images) {
    if (depth > 1 || ![values isKindOfClass:NSArray.class] ||
        [values count] != (NSUInteger)menu.numberOfItems || *count + [values count] > 128) return nil;
    *count += [values count];
    NSMutableArray *prepared = [NSMutableArray arrayWithCapacity:[values count]];
    for (NSUInteger index = 0; index < [values count]; index++) {
        id value = values[index];
        if (![value isKindOfClass:NSDictionary.class]) return nil;
        NSMenuItem *item = [menu itemAtIndex:(NSInteger)index];
        id title = value[@"title"], tooltip = value[@"tooltip"];
        id png = value[@"icon"], children = value[@"children"];
        if (![title isKindOfClass:NSString.class] || ![item.title isEqualToString:title] ||
            (tooltip != NSNull.null && ![tooltip isKindOfClass:NSString.class])) return nil;
        NSImage *image = nil;
        if (png != NSNull.null) {
            if (![png isKindOfClass:NSString.class] || [png length] > 87384) return nil;
            image = images[png];
            if (!image) {
                NSData *data = [[NSData alloc] initWithBase64EncodedString:png options:0];
                image = data ? [[NSImage alloc] initWithData:data] : nil;
                if (!image || image.size.width <= 0 || image.size.height <= 0) return nil;
                image.size = NSMakeSize(18.0 * image.size.width / image.size.height, 18.0);
                image.template = YES;
                images[png] = image;
            }
        }
        id preparedChildren = NSNull.null;
        if (children != NSNull.null) {
            if (!item.submenu) return nil;
            preparedChildren = LensPrepareMenuPresentation(item.submenu, children, depth + 1, count, images);
            if (!preparedChildren) return nil;
        } else if (item.submenu) return nil;
        [prepared addObject:@{@"tooltip": tooltip, @"image": (id)image ?: NSNull.null, @"children": preparedChildren}];
    }
    return prepared;
}

static void LensApplyMenuPresentation(NSMenu *menu, NSArray *values) {
    for (NSUInteger index = 0; index < values.count; index++) {
        NSMenuItem *item = [menu itemAtIndex:(NSInteger)index];
        NSDictionary *value = values[index];
        item.toolTip = value[@"tooltip"] == NSNull.null ? nil : value[@"tooltip"];
        item.image = value[@"image"] == NSNull.null ? nil : value[@"image"];
        if (value[@"children"] != NSNull.null) LensApplyMenuPresentation(item.submenu, value[@"children"]);
    }
}

bool lens_set_menu_presentation(void *rootMenuPointer, size_t submenuIndex,
                               const char *submenuTitle, const char *itemsJSON) {
    if (![NSThread isMainThread] || !rootMenuPointer || !submenuTitle || !itemsJSON) return false;
    NSMenu *root = (__bridge NSMenu *)rootMenuPointer;
    if (!root || submenuIndex >= (size_t)root.numberOfItems) return false;
    NSMenuItem *parent = [root itemAtIndex:(NSInteger)submenuIndex];
    NSString *expectedTitle = [NSString stringWithUTF8String:submenuTitle];
    if (!expectedTitle || ![parent.title isEqualToString:expectedTitle] || !parent.submenu) return false;
    NSData *data = [[NSString stringWithUTF8String:itemsJSON] dataUsingEncoding:NSUTF8StringEncoding];
    if (!data) return false;
    id values = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    NSUInteger count = 0;
    NSArray *prepared = LensPrepareMenuPresentation(parent.submenu, values, 0, &count, [NSMutableDictionary dictionary]);
    if (!prepared) return false;
    LensApplyMenuPresentation(parent.submenu, prepared);
    return true;
}

char *lens_format_short_datetime(double unix_seconds) {
    @autoreleasepool {
        if (!isfinite(unix_seconds)) return NULL;
        NSDateFormatter *formatter = [[NSDateFormatter alloc] init];
        formatter.locale = NSLocale.autoupdatingCurrentLocale;
        formatter.timeZone = NSTimeZone.localTimeZone;
        formatter.dateStyle = NSDateFormatterShortStyle;
        formatter.timeStyle = NSDateFormatterShortStyle;
        NSString *value = [formatter stringFromDate:[NSDate dateWithTimeIntervalSince1970:unix_seconds]];
        return value ? strdup(value.UTF8String) : NULL;
    }
}

int32_t lens_confirm_destructive_action(const char *title, const char *message,
                                      const char *confirmLabel, const char *cancelLabel) {
    if (![NSThread isMainThread] || !NSApp || !title || !message || !confirmLabel || !cancelLabel) return -1;
    NSString *heading = [NSString stringWithUTF8String:title];
    NSString *body = [NSString stringWithUTF8String:message];
    NSString *confirmText = [NSString stringWithUTF8String:confirmLabel];
    NSString *cancelText = [NSString stringWithUTF8String:cancelLabel];
    if (!heading || !body || !confirmText || !cancelText) return -1;
    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = heading;
    alert.informativeText = body;
    alert.alertStyle = NSAlertStyleWarning;
    NSButton *confirm = [alert addButtonWithTitle:confirmText];
    NSButton *cancel = [alert addButtonWithTitle:cancelText];
    confirm.keyEquivalent = @"";
    cancel.keyEquivalent = @"\r";
    [alert layout];
    alert.window.defaultButtonCell = cancel.cell;
    NSWindow *previousKeyWindow = NSApp.keyWindow;
    [NSApp activateIgnoringOtherApps:YES];
    // Giving Cancel the Return equivalent replaces its automatic Escape equivalent.
    // Restore Escape only within this alert's modal event loop, never as a global shortcut.
    id escapeMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
        handler:^NSEvent *(NSEvent *event) {
            if (event.window == alert.window && event.keyCode == 53) {
                [NSApp stopModalWithCode:NSAlertSecondButtonReturn];
                return nil;
            }
            return event;
        }];
    if (!escapeMonitor) return -1;
    NSModalResponse response = NSModalResponseAbort;
    @try {
        response = [alert runModal];
    } @finally {
        [NSEvent removeMonitor:escapeMonitor];
        [alert.window orderOut:nil];
    }
    if (previousKeyWindow.visible) [previousKeyWindow makeKeyAndOrderFront:nil];
    return response == NSAlertFirstButtonReturn ? 1 : 0;
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
    // Invalid numbers raise NSInvalidArgumentException rather than populating NSError.
    // Validate before serialization so malformed native data never unwinds across Rust FFI.
    BOOL valid = [NSJSONSerialization isValidJSONObject:object];
    NSData *data = valid
        ? [NSJSONSerialization dataWithJSONObject:object options:0 error:&error]
        : nil;
    if (data == nil) {
        NSString *message = valid
            ? (error.localizedDescription ?: @"JSON serialization failed")
            : @"Native response contains values that cannot be represented as JSON";
        NSDictionary *fallback = @{ @"status": @"error", @"message": message };
        data = [NSJSONSerialization dataWithJSONObject:fallback options:0 error:nil];
    }
    if (data == nil) {
        return NULL;
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

bool lens_configure_floating_window_radius(void *windowPointer, double radius) {
    if (![NSThread isMainThread] || !windowPointer || !isfinite(radius) || radius < 0 || radius > 64) return false;
    NSWindow *window = (__bridge NSWindow *)windowPointer;
    NSView *content = window.contentView;
    if (!content) return false;
    content.wantsLayer = YES;
    CALayer *layer = content.layer;
    if (!layer) return false;
    layer.cornerRadius = radius;
    layer.masksToBounds = YES;
    [window invalidateShadow];
    return true;
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
@property(nonatomic, assign) uint32_t windowID;
@property(nonatomic, assign) pid_t pid;
@property(nonatomic, copy) NSString *pickerTitle;
@property(nonatomic, copy) NSString *pickerApplicationName;
@property(nonatomic, assign) CGRect pickerFrame;
@property(nonatomic, copy, nullable) NSString *contextID;
@property(nonatomic, copy, nullable) NSString *sourceRegistrationID;
@property(nonatomic, assign) uint64_t observerEpoch;
@property(nonatomic, assign) LensWindowObservationCallback observationCallback;
@property(nonatomic, assign) void *observationContext;
@property(nonatomic, assign) BOOL acceptingCallbacks;
@property(nonatomic, assign) BOOL runLoopSourceAttached;
@property(nonatomic, assign) double initialResolutionScore;
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

static NSMutableDictionary<NSString *, LensNativeWindowSource *> *LensWindowSourceRegistry;
static NSMutableDictionary<NSString *, LensNativeWindowSource *> *LensObservationSourceRegistry;

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
    }
    if (LensObservationSourceRegistry == nil) {
        LensObservationSourceRegistry = [NSMutableDictionary dictionary];
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

typedef NS_ENUM(NSUInteger, LensAXFrameReadState) {
    LensAXFrameReadStateUnavailable,
    LensAXFrameReadStateNonFinite,
    LensAXFrameReadStateAvailable
};

static LensAXFrameReadState LensReadAXFrame(AXUIElementRef element, CGRect *frameOut) {
    id positionObject = LensCopyAXAttribute(element, kAXPositionAttribute, NULL);
    id sizeObject = LensCopyAXAttribute(element, kAXSizeAttribute, NULL);
    if (positionObject == nil || sizeObject == nil) {
        return LensAXFrameReadStateUnavailable;
    }

    AXValueRef positionValue = (__bridge AXValueRef)positionObject;
    AXValueRef sizeValue = (__bridge AXValueRef)sizeObject;
    if (CFGetTypeID(positionValue) != AXValueGetTypeID() ||
        CFGetTypeID(sizeValue) != AXValueGetTypeID()) {
        return LensAXFrameReadStateUnavailable;
    }

    CGPoint position = CGPointZero;
    CGSize size = CGSizeZero;
    if (!AXValueGetValue(positionValue, kAXValueTypeCGPoint, &position) ||
        !AXValueGetValue(sizeValue, kAXValueTypeCGSize, &size)) {
        return LensAXFrameReadStateUnavailable;
    }
    if (!isfinite(position.x) || !isfinite(position.y)
        || !isfinite(size.width) || !isfinite(size.height)) {
        return LensAXFrameReadStateNonFinite;
    }

    *frameOut = (CGRect){ position, size };
    return LensAXFrameReadStateAvailable;
}

static BOOL LensAXFrame(AXUIElementRef element, CGRect *frameOut) {
    return LensReadAXFrame(element, frameOut) == LensAXFrameReadStateAvailable;
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

    if (resolvedWindow == NULL || bestScore < 25.0) {
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
        if (self.pid <= 0) {
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
        @"window_id": @(self.windowID),
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
            NSMutableData *buffer = [NSMutableData dataWithLength:remaining];
            NSUInteger usedLength = 0;
            // Partial conversion is useful even when the entire range cannot fit.
            // Foundation chooses an encodable boundary, not a grapheme boundary.
            [text getBytes:buffer.mutableBytes maxLength:remaining usedLength:&usedLength
                encoding:NSUTF8StringEncoding options:0 range:NSMakeRange(0, text.length)
                remainingRange:NULL];
            if (usedLength > 0) {
                NSString *prefix = [[NSString alloc] initWithBytes:buffer.bytes
                    length:usedLength encoding:NSUTF8StringEncoding];
                [fragments addObject:prefix];
                *textBytes += usedLength;
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
        NSUInteger nonFiniteFrames = 0;
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
            LensAXFrameReadState frameState = LensReadAXFrame(element, &frame);
            BOOL hasFrame = frameState == LensAXFrameReadStateAvailable;
            if (frameState == LensAXFrameReadStateNonFinite) {
                if (depth == 0) {
                    CFRelease(scheduled);
                    [diagnostics addObject:@"The resolved AXWindow exposed a non-finite frame; Accessibility extraction is unavailable."];
                    return LensExtractionUnavailableWithDiagnostics(diagnostics);
                }
                nonFiniteFrames++;
            }

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
        if (nonFiniteFrames > 0) {
            [diagnostics addObject:[NSString stringWithFormat:
                @"Omitted bounds for %lu Accessibility nodes with non-finite frame components; node content was retained.",
                (unsigned long)nonFiniteFrames
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
                || resourceReadErrors > 0 || nonFiniteFrames > 0
                ? @"partial"
                : @"full");
        if (!hasUsefulContent) {
            [diagnostics addObject:@"The resolved AXWindow contains no useful text or URI resource references."];
        }

        NSDictionary *result = @{
            @"quality": quality,
            @"resolved_window": @{
                @"facts": @{
                    @"title": resolvedTitle,
                    @"application_name": applicationName,
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

// Finite metadata boundary: safe Rust retains the exact selected SCWindow; native
// owns only its operation-scoped promoted AXWindow and observer lifecycle.
bool lens_register_ax_window_for_operation(
    const char *operationIDCString, uint32_t windowID, int32_t pid,
    const char *titleCString, const char *applicationNameCString,
    double x, double y, double width, double height
) {
    if (!operationIDCString || !titleCString || !applicationNameCString || !windowID || pid <= 0 ||
        !isfinite(x) || !isfinite(y) || !isfinite(width) || !isfinite(height) ||
        width <= 0 || height <= 0 || !isfinite(x + width) || !isfinite(y + height)) return false;
    NSString *operationID = [NSString stringWithUTF8String:operationIDCString];
    NSString *title = [NSString stringWithUTF8String:titleCString];
    NSString *applicationName = [NSString stringWithUTF8String:applicationNameCString];
    if (!operationID.length || !title || !applicationName) return false;
    __block BOOL registered = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        NSString *key = LensWindowRegistryKey(operationID, windowID);
        // Never rebind the native promoted AX authority for an existing key.
        if (LensWindowSourceRegistry[key] != nil) return;
        LensNativeWindowSource *source = [LensNativeWindowSource new];
        source.operationID = operationID;
        source.windowID = windowID;
        source.pid = pid;
        source.pickerTitle = title;
        source.pickerApplicationName = applicationName;
        source.pickerFrame = CGRectMake(x, y, width, height);
        source.initialResolutionScore = -1;
        LensWindowSourceRegistry[key] = source;
        registered = YES;
    });
    return registered;
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

char *lens_extract_registered_window_json(
    const char *operationIDCString,
    uint32_t windowID,
    uint32_t maxNodes,
    uint32_t maxTextBytes,
    uint32_t maxResourceRefs,
    uint32_t maxResourceURIBytes,
    uint32_t maxTotalResourceURIBytes
) {
    @autoreleasepool {
        NSString *operationID = operationIDCString == NULL
            ? nil
            : [NSString stringWithUTF8String:operationIDCString];
        LensNativeWindowSource *source = operationID.length == 0
            ? nil
            : LensWindowSourceForIdentity(operationID, windowID);
        if (source == nil) {
            return LensCopyJSONString(LensExtractionUnavailable(
                @"The exact operation-scoped selected window is unavailable."
            ));
        }
        NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
        if (![source ensureResolvedWindowWithDiagnostics:diagnostics]) {
            return LensCopyJSONString(LensExtractionUnavailableWithDiagnostics(diagnostics));
        }
        AXUIElementRef resolvedWindow = [source copyResolvedWindow];
        if (resolvedWindow == NULL) {
            return LensCopyJSONString(LensExtractionUnavailable(
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
        if (!LensAXFrame(resolvedWindow, &currentFrame)) {
            CFRelease(resolvedWindow);
            [diagnostics addObject:@"The promoted AXWindow did not expose a finite current frame; stale picker geometry was not published as current facts."];
            return LensCopyJSONString(LensExtractionUnavailableWithDiagnostics(diagnostics));
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
        CFRelease(resolvedWindow);
        return LensCopyJSONString(result);
    }
}

char *lens_start_window_observation_json(
    const char *operationIDCString,
    const char *contextIDCString,
    const char *sourceRegistrationIDCString,
    uint64_t observerEpoch,
    uint32_t windowID,
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
            : [NSString stringWithUTF8String:operationIDCString];
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
        LensNativeWindowSource *source = LensWindowSourceForIdentity(operationID, windowID);
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

bool lens_stop_window_observation(
    const char *operationIDCString,
    const char *sourceRegistrationIDCString,
    const void *expectedCallbackContext
) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString];
    NSString *sourceRegistrationID = sourceRegistrationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:sourceRegistrationIDCString];
    if (operationID.length == 0 || sourceRegistrationID.length == 0 || expectedCallbackContext == NULL) {
        return false;
    }
    __block BOOL stopped = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        NSString *sourceKey = LensObservationRegistryKey(operationID, sourceRegistrationID);
        LensNativeWindowSource *source = LensObservationSourceRegistry[sourceKey];
        // A caller can reuse public IDs/epochs. Only the still-live Arc allocation
        // that supplied this exact observation's refcon owns its stop receipt.
        if (source == nil || source.observationContext != expectedCallbackContext) {
            return;
        }
        [source stopObservation];
        [LensObservationSourceRegistry removeObjectForKey:sourceKey];
        stopped = YES;
    });
    return stopped;
}

bool lens_release_registered_window(const char *operationIDCString, uint32_t windowID) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString];
    if (operationID.length == 0) {
        return false;
    }
    __block BOOL released = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
        NSString *windowKey = LensWindowRegistryKey(operationID, windowID);
        LensNativeWindowSource *source = LensWindowSourceRegistry[windowKey];
        if (source == nil) {
            return;
        }
        NSString *sourceRegistrationID = source.sourceRegistrationID;
        [source releaseAllRetainedObjects];
        if (sourceRegistrationID.length > 0) {
            [LensObservationSourceRegistry removeObjectForKey:
                LensObservationRegistryKey(operationID, sourceRegistrationID)];
        }
        [LensWindowSourceRegistry removeObjectForKey:windowKey];
        released = YES;
    });
    return released;
}

bool lens_release_window_operation(const char *operationIDCString) {
    NSString *operationID = operationIDCString == NULL
        ? nil
        : [NSString stringWithUTF8String:operationIDCString];
    if (operationID.length == 0) {
        return false;
    }
    __block BOOL released = NO;
    LensPerformSyncOnMainThread(^{
        LensEnsureWindowRegistries();
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
            released = YES;
        }
    });
    return released;
}



void lens_free_string(char *value) {
    free(value);
}
