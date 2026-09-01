#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>

static NSString *const LensProbeInitialTitle = @"Lens AXObserver Probe Initial";
static NSString *const LensProbeChangedTitle = @"Lens AXObserver Probe Changed";
static NSString *const LensProbeAfterStopTitle = @"Lens AXObserver Probe After Stop";

@class LensAXObserverProbe;
static LensAXObserverProbe *_Nullable LensActiveAXObserverProbe;
static int LensAXObserverProbeExitCode = 1;

typedef NS_ENUM(NSUInteger, LensProbeRegistrationKind) {
    LensProbeRegistrationKindApplication,
    LensProbeRegistrationKindWindow,
};

typedef struct {
    void *_Nullable owner;
    uint64_t epoch;
    LensProbeRegistrationKind kind;
} LensProbeRegistrationContext;

static void LensAXObserverProbeCallback(
    AXObserverRef observer,
    AXUIElementRef element,
    CFStringRef notification,
    void *_Nullable context
);

@interface LensAXObserverProbe : NSObject {
    AXObserverRef _Nullable _observer;
    AXUIElementRef _Nullable _applicationElement;
    AXUIElementRef _Nullable _windowElement;
    LensProbeRegistrationContext _applicationRegistration;
    LensProbeRegistrationContext _windowRegistration;
}
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, assign) AXError observerCreateError;
@property(nonatomic, assign) AXError messagingTimeoutError;
@property(nonatomic, assign) AXError applicationNotificationError;
@property(nonatomic, assign) AXError windowLookupError;
@property(nonatomic, assign) AXError windowNotificationError;
@property(nonatomic, assign) AXError applicationNotificationRemovalError;
@property(nonatomic, assign) AXError windowNotificationRemovalError;
@property(nonatomic, assign) NSUInteger acceptedCallbacks;
@property(nonatomic, assign) NSUInteger callbacksBeforeStop;
@property(nonatomic, assign) NSUInteger rejectedCallbacksAfterStop;
@property(nonatomic, assign) NSUInteger windowCreatedCallbacks;
@property(nonatomic, assign) NSUInteger titleChangedCallbacks;
@property(nonatomic, assign) NSUInteger titleCallbackElementEqualCount;
@property(nonatomic, strong) NSMutableArray<NSDictionary *> *callbackEvents;
@property(nonatomic, assign) BOOL acceptingCallbacks;
@property(nonatomic, assign) BOOL runLoopSourceAttached;
@property(nonatomic, assign) BOOL runLoopSourceRemoved;
@property(nonatomic, assign) BOOL titleMutationIssued;
@property(nonatomic, assign) BOOL stopSequenceIssued;
@property(nonatomic, assign) BOOL windowResolutionScheduled;
@property(nonatomic, assign) NSUInteger windowResolutionAttempts;
@property(nonatomic, assign) BOOL deadlineWon;
@property(nonatomic, assign) BOOL finished;
@property(nonatomic, assign) CFTimeInterval probeStartedAt;
@property(nonatomic, assign) CFTimeInterval probeDuration;
@property(nonatomic, copy) NSString *failure;
- (void)start;
- (void)receiveElement:(AXUIElementRef)element
          notification:(CFStringRef)notification
          registration:(LensProbeRegistrationContext *)registration;
@end

@implementation LensAXObserverProbe

- (instancetype)init {
    self = [super init];
    if (self != nil) {
        _observerCreateError = kAXErrorFailure;
        _messagingTimeoutError = kAXErrorFailure;
        _applicationNotificationError = kAXErrorFailure;
        _windowLookupError = kAXErrorFailure;
        _windowNotificationError = kAXErrorFailure;
        _applicationNotificationRemovalError = kAXErrorFailure;
        _windowNotificationRemovalError = kAXErrorFailure;
        _acceptingCallbacks = YES;
        _callbackEvents = [NSMutableArray array];
        _applicationRegistration = (LensProbeRegistrationContext){
            .owner = (__bridge void *)self,
            .epoch = 1,
            .kind = LensProbeRegistrationKindApplication,
        };
        _windowRegistration = (LensProbeRegistrationContext){
            .owner = (__bridge void *)self,
            .epoch = 1,
            .kind = LensProbeRegistrationKindWindow,
        };
    }
    return self;
}

- (void)dealloc {
    if (_windowElement != NULL) {
        CFRelease(_windowElement);
    }
    if (_applicationElement != NULL) {
        CFRelease(_applicationElement);
    }
    if (_observer != NULL) {
        CFRelease(_observer);
    }
}

- (void)start {
    NSAssert([NSThread isMainThread], @"AXObserver probe must start on the AppKit main thread");
    self.probeStartedAt = CFAbsoluteTimeGetCurrent();
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            if (self.finished) {
                return;
            }
            self.deadlineWon = YES;
            self.failure = @"AXObserver probe timed out after 5 seconds.";
            [self finish];
        }
    );

    _observerCreateError = AXObserverCreate(
        getpid(),
        LensAXObserverProbeCallback,
        &_observer
    );
    if (_observerCreateError != kAXErrorSuccess || _observer == NULL) {
        [self finish];
        return;
    }

    _applicationElement = AXUIElementCreateApplication(getpid());
    if (_applicationElement == NULL) {
        [self finish];
        return;
    }
    _messagingTimeoutError = AXUIElementSetMessagingTimeout(_applicationElement, 1.0);

    CFRunLoopSourceRef source = AXObserverGetRunLoopSource(_observer);
    if (source != NULL) {
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
        self.runLoopSourceAttached = YES;
    }

    _applicationNotificationError = AXObserverAddNotification(
        _observer,
        _applicationElement,
        kAXWindowCreatedNotification,
        &_applicationRegistration
    );
    if (_messagingTimeoutError != kAXErrorSuccess
        || !self.runLoopSourceAttached
        || _applicationNotificationError != kAXErrorSuccess) {
        self.failure = @"AXObserver setup did not reach the registered state.";
        [self finish];
        return;
    }
    dispatch_async(dispatch_get_main_queue(), ^{
        [self createWindow];
    });
}

- (void)createWindow {
    NSRect frame = NSMakeRect(100.0, 100.0, 480.0, 240.0);
    self.window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                    backing:NSBackingStoreBuffered
                      defer:NO];
    self.window.title = LensProbeInitialTitle;
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}

- (BOOL)resolveAndRegisterExactWindow {
    CFTypeRef focusedWindowValue = NULL;
    AXError focusedWindowError = AXUIElementCopyAttributeValue(
        _applicationElement,
        kAXFocusedWindowAttribute,
        &focusedWindowValue
    );
    if (focusedWindowError == kAXErrorSuccess
        && focusedWindowValue != NULL
        && CFGetTypeID(focusedWindowValue) == AXUIElementGetTypeID()) {
        _windowElement = (AXUIElementRef)focusedWindowValue;
        _windowLookupError = kAXErrorSuccess;
        _windowNotificationError = AXObserverAddNotification(
            _observer,
            _windowElement,
            kAXTitleChangedNotification,
            &_windowRegistration
        );
        return _windowNotificationError == kAXErrorSuccess;
    }
    if (focusedWindowValue != NULL) {
        CFRelease(focusedWindowValue);
    }

    CFTypeRef windowsValue = NULL;
    _windowLookupError = AXUIElementCopyAttributeValue(
        _applicationElement,
        kAXWindowsAttribute,
        &windowsValue
    );
    if (_windowLookupError != kAXErrorSuccess || windowsValue == NULL) {
        if (windowsValue != NULL) {
            CFRelease(windowsValue);
        }
        return NO;
    }
    if (CFGetTypeID(windowsValue) != CFArrayGetTypeID()) {
        CFRelease(windowsValue);
        _windowLookupError = kAXErrorIllegalArgument;
        return NO;
    }

    CFArrayRef windows = (CFArrayRef)windowsValue;
    for (CFIndex index = 0; index < CFArrayGetCount(windows); index += 1) {
        AXUIElementRef candidate = (AXUIElementRef)CFArrayGetValueAtIndex(windows, index);
        CFTypeRef titleValue = NULL;
        AXError titleError = AXUIElementCopyAttributeValue(
            candidate,
            kAXTitleAttribute,
            &titleValue
        );
        BOOL matches = titleError == kAXErrorSuccess
            && titleValue != NULL
            && CFGetTypeID(titleValue) == CFStringGetTypeID()
            && CFEqual(titleValue, (__bridge CFStringRef)LensProbeInitialTitle);
        if (titleValue != NULL) {
            CFRelease(titleValue);
        }
        if (matches) {
            _windowElement = (AXUIElementRef)CFRetain(candidate);
            break;
        }
    }
    CFRelease(windowsValue);

    if (_windowElement == NULL) {
        _windowLookupError = kAXErrorNoValue;
        return NO;
    }
    _windowLookupError = kAXErrorSuccess;
    _windowNotificationError = AXObserverAddNotification(
        _observer,
        _windowElement,
        kAXTitleChangedNotification,
        &_windowRegistration
    );
    return _windowNotificationError == kAXErrorSuccess;
}

- (void)attemptExactWindowRegistration {
    if (self.finished || !self.acceptingCallbacks || _windowElement != NULL) {
        return;
    }
    self.windowResolutionAttempts += 1;
    if ([self resolveAndRegisterExactWindow]) {
        if (!self.titleMutationIssued) {
            self.titleMutationIssued = YES;
            dispatch_async(dispatch_get_main_queue(), ^{
                if (!self.finished && self.acceptingCallbacks) {
                    self.window.title = LensProbeChangedTitle;
                }
            });
        }
        return;
    }
    if (_windowElement != NULL) {
        self.failure = @"The exact-window notification registration failed.";
        [self finish];
        return;
    }
    if (self.windowResolutionAttempts >= 20) {
        self.failure = @"The created window did not become resolvable within 20 attempts.";
        [self finish];
        return;
    }
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.05 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self attemptExactWindowRegistration];
        }
    );
}

- (void)receiveElement:(AXUIElementRef)element
          notification:(CFStringRef)notification
          registration:(LensProbeRegistrationContext *)registration {
    if (self.finished) {
        return;
    }
    if (self.acceptingCallbacks) {
        self.acceptedCallbacks += 1;
        BOOL exactWindow = _windowElement != NULL && CFEqual(element, _windowElement);
        if (registration->kind == LensProbeRegistrationKindApplication
            && CFEqual(notification, kAXWindowCreatedNotification)) {
            self.windowCreatedCallbacks += 1;
        }
        if (registration->kind == LensProbeRegistrationKindWindow
            && CFEqual(notification, kAXTitleChangedNotification)) {
            self.titleChangedCallbacks += 1;
            if (exactWindow) {
                self.titleCallbackElementEqualCount += 1;
            }
        }
        if (self.callbackEvents.count < 16) {
            [self.callbackEvents addObject:@{
                @"notification": (__bridge NSString *)notification,
                @"registration": registration->kind == LensProbeRegistrationKindWindow
                    ? @"window"
                    : @"application",
                @"epoch": @(registration->epoch),
                @"exact_window": @(exactWindow),
            }];
        }
    } else {
        self.rejectedCallbacksAfterStop += 1;
    }

    if (self.acceptingCallbacks
        && _windowElement == NULL
        && CFEqual(notification, kAXWindowCreatedNotification)
        && !self.windowResolutionScheduled) {
        self.windowResolutionScheduled = YES;
        dispatch_async(dispatch_get_main_queue(), ^{
            [self attemptExactWindowRegistration];
        });
    }

    if (self.acceptingCallbacks
        && !self.stopSequenceIssued
        && registration->kind == LensProbeRegistrationKindWindow
        && CFEqual(notification, kAXTitleChangedNotification)) {
        self.stopSequenceIssued = YES;
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            [self stopObserving];
            self.window.title = LensProbeAfterStopTitle;
            dispatch_after(
                dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.25 * NSEC_PER_SEC)),
                dispatch_get_main_queue(),
                ^{
                    [self finish];
                }
            );
        });
    }
}

- (void)stopObserving {
    self.callbacksBeforeStop = self.acceptedCallbacks;
    self.acceptingCallbacks = NO;

    if (_windowElement != NULL && _observer != NULL) {
        _windowNotificationRemovalError = AXObserverRemoveNotification(
            _observer,
            _windowElement,
            kAXTitleChangedNotification
        );
    }
    if (_applicationElement != NULL && _observer != NULL) {
        _applicationNotificationRemovalError = AXObserverRemoveNotification(
            _observer,
            _applicationElement,
            kAXWindowCreatedNotification
        );
    }
    if (_observer != NULL && self.runLoopSourceAttached) {
        CFRunLoopSourceRef source = AXObserverGetRunLoopSource(_observer);
        if (source != NULL) {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            self.runLoopSourceRemoved = YES;
        }
    }
}

- (void)finish {
    if (self.finished) {
        return;
    }
    self.finished = YES;
    self.probeDuration = CFAbsoluteTimeGetCurrent() - self.probeStartedAt;
    if (self.acceptingCallbacks) {
        [self stopObserving];
    }
    BOOL passed = _observerCreateError == kAXErrorSuccess
        && _messagingTimeoutError == kAXErrorSuccess
        && self.runLoopSourceAttached
        && _applicationNotificationError == kAXErrorSuccess
        && _windowLookupError == kAXErrorSuccess
        && _windowNotificationError == kAXErrorSuccess
        && self.windowCreatedCallbacks >= 1
        && self.titleChangedCallbacks >= 1
        && self.acceptedCallbacks == self.callbacksBeforeStop
        && self.rejectedCallbacksAfterStop == 0
        && _applicationNotificationRemovalError == kAXErrorSuccess
        && _windowNotificationRemovalError == kAXErrorSuccess
        && self.runLoopSourceRemoved
        && !self.deadlineWon
        && self.failure == nil;
    NSDictionary *result = @{
        @"status": passed ? @"passed" : @"failed",
        @"trusted": @(AXIsProcessTrusted()),
        @"observer_create_error": @(_observerCreateError),
        @"messaging_timeout_error": @(_messagingTimeoutError),
        @"run_loop_source_attached": @(self.runLoopSourceAttached),
        @"application_notification_error": @(_applicationNotificationError),
        @"window_lookup_error": @(_windowLookupError),
        @"window_resolution_attempts": @(self.windowResolutionAttempts),
        @"window_notification_error": @(_windowNotificationError),
        @"callbacks_before_stop": @(self.callbacksBeforeStop),
        @"window_created_callbacks": @(self.windowCreatedCallbacks),
        @"title_changed_callbacks": @(self.titleChangedCallbacks),
        @"title_callback_element_cf_equal_count": @(self.titleCallbackElementEqualCount),
        @"callback_events": self.callbackEvents,
        @"accepted_callbacks_after_stop": @(self.acceptedCallbacks - self.callbacksBeforeStop),
        @"rejected_callbacks_after_stop": @(self.rejectedCallbacksAfterStop),
        @"application_notification_removal_error": @(_applicationNotificationRemovalError),
        @"window_notification_removal_error": @(_windowNotificationRemovalError),
        @"run_loop_source_removed": @(self.runLoopSourceRemoved),
        @"deadline_won": @(self.deadlineWon),
        @"probe_duration_ms": @(round(self.probeDuration * 1000.0)),
        @"failure": self.failure ?: [NSNull null],
    };
    NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
    NSString *jsonString = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    printf("LENS_AX_OBSERVER_PROBE=%s\n", jsonString.UTF8String);
    fflush(stdout);
    LensAXObserverProbeExitCode = passed ? 0 : 1;
    [self.window close];
    LensActiveAXObserverProbe = nil;
    [NSApp terminate:nil];
}

@end

static void LensAXObserverProbeCallback(
    AXObserverRef observer,
    AXUIElementRef element,
    CFStringRef notification,
    void *_Nullable context
) {
    (void)observer;
    if (context == NULL) {
        return;
    }
    LensProbeRegistrationContext *registration = context;
    if (registration->owner == NULL) {
        return;
    }
    LensAXObserverProbe *probe = (__bridge LensAXObserverProbe *)registration->owner;
    [probe receiveElement:element notification:notification registration:registration];
}

int main(void) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) {
            NSDictionary *result = @{
                @"status": @"permission_required",
                @"trusted": @NO,
            };
            NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
            NSString *jsonString = [[NSString alloc]
                initWithData:json
                    encoding:NSUTF8StringEncoding];
            printf("LENS_AX_OBSERVER_PROBE=%s\n", jsonString.UTF8String);
            return 2;
        }

        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        LensActiveAXObserverProbe = [[LensAXObserverProbe alloc] init];
        dispatch_async(dispatch_get_main_queue(), ^{
            [LensActiveAXObserverProbe start];
        });
        [NSApp run];
        return LensAXObserverProbeExitCode;
    }
}
