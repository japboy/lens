#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

typedef void (*PLPickerCallback)(const char *_Nullable json, void *_Nullable context);
typedef void (*PLWindowObserverCallback)(const char *_Nullable json, void *_Nullable context);

static NSDictionary *PLFrameDictionary(CGRect frame);

static char *PLCopyJSONString(id object) {
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

static NSString *PLStringOrEmpty(NSString *value) {
    return value ?: @"";
}

@interface PLContentPickerCoordinator : NSObject <SCContentSharingPickerObserver>
@property(nonatomic, assign) PLPickerCallback callback;
@property(nonatomic, assign) void *callbackContext;
@property(nonatomic, assign) BOOL observing;
+ (instancetype)shared;
- (BOOL)presentWithCallback:(PLPickerCallback)callback context:(void *)context;
@end

@implementation PLContentPickerCoordinator

+ (instancetype)shared {
    static PLContentPickerCoordinator *coordinator;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        coordinator = [[PLContentPickerCoordinator alloc] init];
    });
    return coordinator;
}

- (void)deliver:(NSDictionary *)payload {
    PLPickerCallback callback = self.callback;
    void *context = self.callbackContext;
    self.callback = NULL;
    self.callbackContext = NULL;

    if (callback != NULL) {
        char *json = PLCopyJSONString(payload);
        callback(json, context);
        free(json);
    }
}

- (BOOL)presentWithCallback:(PLPickerCallback)callback context:(void *)context {
    NSAssert([NSThread isMainThread], @"The ScreenCaptureKit picker must be presented on the main thread");
    if (self.callback != NULL) {
        return NO;
    }

    self.callback = callback;
    self.callbackContext = context;

    SCContentSharingPicker *picker = SCContentSharingPicker.sharedPicker;
    if (!self.observing) {
        [picker addObserver:self];
        self.observing = YES;
    }

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
    picker.active = NO;
    [self deliver:@{ @"status": @"cancelled" }];
}

- (void)contentSharingPicker:(SCContentSharingPicker *)picker
         didUpdateWithFilter:(SCContentFilter *)filter
                   forStream:(SCStream *)stream {
    picker.active = NO;

    if (@available(macOS 15.2, *)) {
        SCWindow *window = filter.includedWindows.firstObject;
        if (window == nil) {
            [self deliver:@{
                @"status": @"error",
                @"message": @"The picker returned no included SCWindow."
            }];
            return;
        }

        SCRunningApplication *application = window.owningApplication;
        CGRect frame = window.frame;
        [self deliver:@{
            @"status": @"selected",
            @"window_id": @(window.windowID),
            @"title": PLStringOrEmpty(window.title),
            @"application_name": PLStringOrEmpty(application.applicationName),
            @"bundle_id": PLStringOrEmpty(application.bundleIdentifier),
            @"pid": @(application.processID),
            @"frame": @{
                @"x": @(frame.origin.x),
                @"y": @(frame.origin.y),
                @"width": @(frame.size.width),
                @"height": @(frame.size.height)
            }
        }];
        return;
    }

    [self deliver:@{
        @"status": @"error",
        @"message": @"PersonalLens requires macOS 15.2 or later for deterministic SCWindow resolution."
    }];
}

- (void)contentSharingPickerStartDidFailWithError:(NSError *)error {
    SCContentSharingPicker.sharedPicker.active = NO;
    [self deliver:@{
        @"status": @"error",
        @"message": error.localizedDescription ?: @"The native window picker failed to start."
    }];
}

@end

bool pl_accessibility_is_trusted(void) {
    return AXIsProcessTrusted();
}

bool pl_accessibility_request_trust(void) {
    NSDictionary *options = @{ (__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES };
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
}

bool pl_present_window_picker(PLPickerCallback callback, void *context) {
    if (callback == NULL) {
        return false;
    }

    if ([NSThread isMainThread]) {
        return [[PLContentPickerCoordinator shared] presentWithCallback:callback context:context];
    }

    __block BOOL presented = NO;
    dispatch_sync(dispatch_get_main_queue(), ^{
        presented = [[PLContentPickerCoordinator shared] presentWithCallback:callback context:context];
    });
    return presented;
}

char *pl_copy_window_frame_json(uint32_t windowID) {
    @autoreleasepool {
        NSArray<NSNumber *> *windowIDs = @[ @(windowID) ];
        CFArrayRef descriptionsRef = CGWindowListCreateDescriptionFromArray(
            (__bridge CFArrayRef)windowIDs
        );
        NSArray<NSDictionary *> *descriptions = CFBridgingRelease(descriptionsRef);
        NSDictionary *description = descriptions.firstObject;
        NSNumber *resolvedWindowID = description[(__bridge NSString *)kCGWindowNumber];
        NSDictionary *bounds = description[(__bridge NSString *)kCGWindowBounds];
        CGRect frame = CGRectZero;
        if (description == nil || resolvedWindowID.unsignedIntValue != windowID ||
            ![bounds isKindOfClass:NSDictionary.class] ||
            !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)bounds, &frame) ||
            !isfinite(frame.origin.x) || !isfinite(frame.origin.y) ||
            !isfinite(frame.size.width) || !isfinite(frame.size.height) ||
            frame.size.width <= 0.0 || frame.size.height <= 0.0) {
            return PLCopyJSONString(@{ @"status": @"unavailable" });
        }

        return PLCopyJSONString(@{
            @"status": @"available",
            @"frame": PLFrameDictionary(frame)
        });
    }
}

static id PLCopyAXAttribute(AXUIElementRef element, CFStringRef attribute, AXError *errorOut) {
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

static NSString *PLAXString(AXUIElementRef element, CFStringRef attribute) {
    id value = PLCopyAXAttribute(element, attribute, NULL);
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

static NSNumber *PLAXNumber(AXUIElementRef element, CFStringRef attribute) {
    id value = PLCopyAXAttribute(element, attribute, NULL);
    return [value isKindOfClass:NSNumber.class] ? value : nil;
}

static BOOL PLAXFrame(AXUIElementRef element, CGRect *frameOut) {
    id positionObject = PLCopyAXAttribute(element, kAXPositionAttribute, NULL);
    id sizeObject = PLCopyAXAttribute(element, kAXSizeAttribute, NULL);
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

static NSDictionary *PLFrameDictionary(CGRect frame) {
    return @{
        @"x": @(frame.origin.x),
        @"y": @(frame.origin.y),
        @"width": @(frame.size.width),
        @"height": @(frame.size.height)
    };
}

static NSArray *PLAXElementsForArrayAttribute(
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

static double PLWindowResolutionScore(
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

static AXUIElementRef PLCopyResolvedAXWindow(
    int32_t pid,
    NSString *selectedTitle,
    CGRect selectedFrame,
    NSString *__autoreleasing _Nullable *resolvedTitleOut,
    CGRect *resolvedFrameOut,
    double *resolutionScoreOut
) {
    AXUIElementRef application = AXUIElementCreateApplication(pid);
    NSArray *windows = PLCopyAXAttribute(application, kAXWindowsAttribute, NULL);
    if (![windows isKindOfClass:NSArray.class] || windows.count == 0) {
        CFRelease(application);
        return NULL;
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
        NSString *candidateTitle = PLAXString(candidate, kAXTitleAttribute) ?: @"";
        CGRect candidateFrame = CGRectZero;
        BOOL hasFrame = PLAXFrame(candidate, &candidateFrame);
        double score = PLWindowResolutionScore(
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
        CFRelease(application);
        return NULL;
    }
    CFRetain(resolvedWindow);
    CFRelease(application);
    if (resolvedTitleOut != NULL) *resolvedTitleOut = resolvedTitle;
    if (resolvedFrameOut != NULL) *resolvedFrameOut = resolvedFrame;
    if (resolutionScoreOut != NULL) *resolutionScoreOut = bestScore;
    return resolvedWindow;
}

@interface PLWindowObserverCoordinator : NSObject
@property(nonatomic, assign) AXObserverRef observer;
@property(nonatomic, assign) AXUIElementRef application;
@property(nonatomic, assign) AXUIElementRef window;
@property(nonatomic, assign) PLWindowObserverCallback callback;
@property(nonatomic, assign) void *callbackContext;
@property(nonatomic, copy) NSArray<NSString *> *registeredNotifications;
+ (instancetype)shared;
- (BOOL)startForPID:(int32_t)pid
              title:(NSString *)title
              frame:(CGRect)frame
           callback:(PLWindowObserverCallback)callback
            context:(void *)context;
- (void *)stopReturningContext;
- (void *)stopIfContext:(void *)context;
- (void)handleElement:(AXUIElementRef)element notification:(CFStringRef)notification;
@end

static void PLWindowObserverDidReceive(
    AXObserverRef observer,
    AXUIElementRef element,
    CFStringRef notification,
    void *refcon
) {
    (void)observer;
    PLWindowObserverCoordinator *coordinator = (__bridge PLWindowObserverCoordinator *)refcon;
    [coordinator handleElement:element notification:notification];
}

@implementation PLWindowObserverCoordinator

+ (instancetype)shared {
    static PLWindowObserverCoordinator *coordinator;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        coordinator = [[PLWindowObserverCoordinator alloc] init];
    });
    return coordinator;
}

- (BOOL)startForPID:(int32_t)pid
              title:(NSString *)title
              frame:(CGRect)frame
           callback:(PLWindowObserverCallback)callback
            context:(void *)context {
    NSAssert([NSThread isMainThread], @"AXObserver must be installed on the main run loop");
    if (self.observer != NULL || callback == NULL) {
        return NO;
    }

    AXUIElementRef window = PLCopyResolvedAXWindow(pid, title, frame, NULL, NULL, NULL);
    if (window == NULL) {
        return NO;
    }
    AXUIElementRef application = AXUIElementCreateApplication(pid);
    AXObserverRef observer = NULL;
    AXError createError = AXObserverCreate(pid, PLWindowObserverDidReceive, &observer);
    if (createError != kAXErrorSuccess || observer == NULL) {
        CFRelease(application);
        CFRelease(window);
        return NO;
    }

    NSArray<NSString *> *notifications = @[
        (__bridge NSString *)kAXMovedNotification,
        (__bridge NSString *)kAXResizedNotification,
        (__bridge NSString *)kAXWindowMovedNotification,
        (__bridge NSString *)kAXWindowResizedNotification,
        (__bridge NSString *)kAXUIElementDestroyedNotification
    ];
    NSMutableArray<NSString *> *registered = [NSMutableArray array];
    NSUInteger registeredGeometryNotifications = 0;
    for (NSString *notification in notifications) {
        AXError error = AXObserverAddNotification(
            observer,
            application,
            (__bridge CFStringRef)notification,
            (__bridge void *)self
        );
        if (error == kAXErrorSuccess || error == kAXErrorNotificationAlreadyRegistered) {
            [registered addObject:notification];
            if (![notification isEqualToString:(__bridge NSString *)kAXUIElementDestroyedNotification]) {
                registeredGeometryNotifications += 1;
            }
        }
    }
    if (registeredGeometryNotifications == 0) {
        for (NSString *notification in registered) {
            AXObserverRemoveNotification(
                observer,
                application,
                (__bridge CFStringRef)notification
            );
        }
        CFRelease(observer);
        CFRelease(application);
        CFRelease(window);
        return NO;
    }

    self.observer = observer;
    self.application = application;
    self.window = window;
    self.callback = callback;
    self.callbackContext = context;
    self.registeredNotifications = registered;
    CFRunLoopAddSource(
        CFRunLoopGetMain(),
        AXObserverGetRunLoopSource(observer),
        kCFRunLoopCommonModes
    );
    return YES;
}

- (void)handleElement:(AXUIElementRef)element notification:(CFStringRef)notification {
    if (self.callback == NULL || self.window == NULL || !CFEqual(element, self.window)) {
        return;
    }
    NSDictionary *payload;
    if (CFEqual(notification, kAXUIElementDestroyedNotification)) {
        payload = @{ @"kind": @"destroyed" };
    } else {
        CGRect frame = CGRectZero;
        if (!PLAXFrame(self.window, &frame)) {
            return;
        }
        payload = @{
            @"kind": @"frame_changed",
            @"frame": PLFrameDictionary(frame)
        };
    }
    char *json = PLCopyJSONString(payload);
    self.callback(json, self.callbackContext);
    free(json);
}

- (void *)stopReturningContext {
    NSAssert([NSThread isMainThread], @"AXObserver must be removed from the main run loop");
    void *context = self.callbackContext;
    self.callback = NULL;
    self.callbackContext = NULL;
    if (self.observer != NULL) {
        CFRunLoopRemoveSource(
            CFRunLoopGetMain(),
            AXObserverGetRunLoopSource(self.observer),
            kCFRunLoopCommonModes
        );
        for (NSString *notification in self.registeredNotifications) {
            AXObserverRemoveNotification(
                self.observer,
                self.application,
                (__bridge CFStringRef)notification
            );
        }
        CFRelease(self.observer);
    }
    if (self.application != NULL) CFRelease(self.application);
    if (self.window != NULL) CFRelease(self.window);
    self.observer = NULL;
    self.application = NULL;
    self.window = NULL;
    self.registeredNotifications = @[];
    return context;
}

- (void *)stopIfContext:(void *)context {
    return self.callbackContext == context ? [self stopReturningContext] : NULL;
}

@end

bool pl_start_window_observer(
    int32_t pid,
    const char *selectedTitleCString,
    double selectedX,
    double selectedY,
    double selectedWidth,
    double selectedHeight,
    PLWindowObserverCallback callback,
    void *context
) {
    NSString *title = selectedTitleCString == NULL
        ? @""
        : [NSString stringWithUTF8String:selectedTitleCString];
    CGRect frame = CGRectMake(selectedX, selectedY, selectedWidth, selectedHeight);
    __block BOOL started = NO;
    dispatch_block_t start = ^{
        started = [[PLWindowObserverCoordinator shared]
            startForPID:pid title:title frame:frame callback:callback context:context];
    };
    if ([NSThread isMainThread]) {
        start();
    } else {
        dispatch_sync(dispatch_get_main_queue(), start);
    }
    return started;
}

void *pl_stop_window_observer(void) {
    __block void *context = NULL;
    dispatch_block_t stop = ^{
        context = [[PLWindowObserverCoordinator shared] stopReturningContext];
    };
    if ([NSThread isMainThread]) {
        stop();
    } else {
        dispatch_sync(dispatch_get_main_queue(), stop);
    }
    return context;
}

void *pl_stop_window_observer_if_context(void *expectedContext) {
    __block void *context = NULL;
    dispatch_block_t stop = ^{
        context = [[PLWindowObserverCoordinator shared] stopIfContext:expectedContext];
    };
    if ([NSThread isMainThread]) {
        stop();
    } else {
        dispatch_sync(dispatch_get_main_queue(), stop);
    }
    return context;
}

static NSString *PLTrimmedText(NSString *text) {
    if (text == nil) {
        return nil;
    }
    NSString *trimmed = [text stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    return trimmed.length > 0 ? trimmed : nil;
}

static void PLAppendTextFragment(
    NSMutableArray<NSString *> *fragments,
    NSMutableSet<NSString *> *seenInNode,
    NSString *candidate,
    NSUInteger maxBytes,
    NSUInteger *textBytes,
    BOOL *truncated
) {
    NSString *text = PLTrimmedText(candidate);
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

static BOOL PLIsWindowChromeText(NSString *candidate, NSString *windowTitle) {
    NSString *text = PLTrimmedText(candidate);
    NSString *title = PLTrimmedText(windowTitle);
    return text != nil && title != nil && [text caseInsensitiveCompare:title] == NSOrderedSame;
}

static NSDictionary *PLExtractionUnavailable(NSString *diagnostic) {
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
            @"children_read_errors": @0
        },
        @"diagnostics": @[diagnostic]
    };
}

char *pl_extract_window_json(
    int32_t pid,
    const char *selectedTitleCString,
    double selectedX,
    double selectedY,
    double selectedWidth,
    double selectedHeight,
    uint32_t maxNodes,
    uint32_t maxTextBytes
) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) {
            return PLCopyJSONString(PLExtractionUnavailable(
                @"Accessibility permission is not granted to PersonalLens."
            ));
        }

        NSString *selectedTitle = selectedTitleCString == NULL
            ? @""
            : [NSString stringWithUTF8String:selectedTitleCString];
        CGRect selectedFrame = CGRectMake(selectedX, selectedY, selectedWidth, selectedHeight);
        double bestScore = -1.0;
        NSString *resolvedTitle = @"";
        CGRect resolvedFrame = CGRectZero;
        AXUIElementRef resolvedWindow = PLCopyResolvedAXWindow(
            pid,
            selectedTitle,
            selectedFrame,
            &resolvedTitle,
            &resolvedFrame,
            &bestScore
        );
        if (resolvedWindow == NULL) {
            return PLCopyJSONString(PLExtractionUnavailable(
                @"LensTargetResolver could not deterministically match the SCWindow to an AXWindow."
            ));
        }

        NSMutableArray *nodes = [NSMutableArray array];
        NSMutableArray<NSString *> *fragments = [NSMutableArray array];
        NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
        NSMutableArray<NSDictionary *> *queue = [NSMutableArray arrayWithObject:@{
            @"element": (__bridge id)resolvedWindow,
            @"depth": @0
        }];
        CFMutableSetRef visited = CFSetCreateMutable(NULL, 0, &kCFTypeSetCallBacks);

        NSUInteger cursor = 0;
        NSUInteger textBytes = 0;
        NSUInteger offscreenTextNodes = 0;
        NSUInteger virtualizationSignals = 0;
        NSUInteger childrenReadErrors = 0;
        BOOL truncatedNodes = NO;
        BOOL truncatedText = NO;

        while (cursor < queue.count) {
            if (nodes.count >= maxNodes) {
                truncatedNodes = YES;
                break;
            }

            NSDictionary *entry = queue[cursor++];
            AXUIElementRef element = (__bridge AXUIElementRef)entry[@"element"];
            if (CFSetContainsValue(visited, element)) {
                continue;
            }
            CFSetAddValue(visited, element);

            NSUInteger depth = [entry[@"depth"] unsignedIntegerValue];
            NSString *role = PLAXString(element, kAXRoleAttribute);
            NSString *subrole = PLAXString(element, kAXSubroleAttribute);
            NSString *title = PLAXString(element, kAXTitleAttribute);
            NSString *value = PLAXString(element, kAXValueAttribute);
            NSString *description = PLAXString(element, kAXDescriptionAttribute);
            CGRect frame = CGRectZero;
            BOOL hasFrame = PLAXFrame(element, &frame);

            NSMutableDictionary *node = [NSMutableDictionary dictionaryWithObject:@(depth) forKey:@"depth"];
            if (role.length > 0) node[@"role"] = role;
            if (subrole.length > 0) node[@"subrole"] = subrole;
            if (title.length > 0) node[@"title"] = title;
            if (value.length > 0) node[@"value"] = value;
            if (description.length > 0) node[@"description"] = description;
            if (hasFrame) node[@"bounds"] = PLFrameDictionary(frame);
            [nodes addObject:node];

            NSUInteger fragmentsBefore = fragments.count;
            // Window-title chrome is target metadata, not document content. Keeping it in the node
            // model helps resolver diagnostics, while excluding root attributes and repeated title
            // descendants ensures a WebView exposing only its chrome is classified as unavailable.
            if (depth > 0) {
                // AX commonly repeats one semantic value across a node's title, value, and
                // description attributes. Deduplicate only within this node so equal text on
                // distinct rows, cells, or list items retains its structural meaning.
                NSMutableSet<NSString *> *seenInNode = [NSMutableSet set];
                if (!PLIsWindowChromeText(title, selectedTitle)) {
                    PLAppendTextFragment(fragments, seenInNode, title, maxTextBytes, &textBytes, &truncatedText);
                }
                if (!PLIsWindowChromeText(value, selectedTitle)) {
                    PLAppendTextFragment(fragments, seenInNode, value, maxTextBytes, &textBytes, &truncatedText);
                }
                if (!PLIsWindowChromeText(description, selectedTitle)) {
                    PLAppendTextFragment(fragments, seenInNode, description, maxTextBytes, &textBytes, &truncatedText);
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
            NSArray *children = PLAXElementsForArrayAttribute(
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
            NSNumber *rowCount = PLAXNumber(element, kAXRowCountAttribute);
            if (rowCount != nil) {
                NSUInteger reportedRows = 0;
                BOOL rowsTruncated = NO;
                NSArray *rows = PLAXElementsForArrayAttribute(
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

            for (id child in children) {
                CFTypeRef childType = (__bridge CFTypeRef)child;
                if (CFGetTypeID(childType) == AXUIElementGetTypeID()) {
                    NSUInteger scheduledNodes = queue.count - cursor;
                    NSUInteger availableSlots = nodes.count < maxNodes
                        ? maxNodes - nodes.count
                        : 0;
                    if (scheduledNodes >= availableSlots) {
                        truncatedNodes = YES;
                        break;
                    }
                    [queue addObject:@{ @"element": child, @"depth": @(depth + 1) }];
                }
            }
        }

        CFRelease(visited);
        CFRelease(resolvedWindow);

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

        NSString *text = [fragments componentsJoinedByString:@"\n"];
        NSString *quality = text.length == 0
            ? @"unavailable"
            : (truncatedNodes || truncatedText || childrenReadErrors > 0 || virtualizationSignals > 0
                ? @"partial"
                : @"full");
        if (text.length == 0) {
            [diagnostics addObject:@"The resolved AXWindow contains no useful textual attributes."];
        }

        NSDictionary *result = @{
            @"quality": quality,
            @"resolved_window": @{
                @"title": resolvedTitle,
                @"bounds": PLFrameDictionary(resolvedFrame),
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
                @"children_read_errors": @(childrenReadErrors)
            },
            @"diagnostics": diagnostics
        };
        return PLCopyJSONString(result);
    }
}

void pl_free_string(char *value) {
    free(value);
}
