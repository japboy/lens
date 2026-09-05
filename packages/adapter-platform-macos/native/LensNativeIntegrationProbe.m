#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <math.h>
#include <signal.h>
#include <string.h>

#import "LensNative.h"

static NSString *const LensIntegrationInitialTitle = @"Lens Native Integration Initial";
static NSString *const LensIntegrationChangedTitle = @"Lens Native Integration Changed";
static NSString *const LensIntegrationAfterStopTitle = @"Lens Native Integration After Stop";
static NSString *const LensIntegrationResumeTitle = @"Lens Native Integration Resume";
static NSString *const LensIntegrationFinalStopTitle = @"Lens Native Integration Final Stop";
static NSString *const LensIntegrationOperationID = @"11111111-1111-4111-8111-111111111111";
static NSString *const LensIntegrationContextID = @"22222222-2222-4222-8222-222222222222";
static NSString *const LensIntegrationSourceID = @"33333333-3333-4333-8333-333333333333";
static const uint64_t LensIntegrationObserverEpoch = 7;
static const uint64_t LensIntegrationResumeObserverEpoch = 8;

@class LensNativeIntegrationProbe;
static LensNativeIntegrationProbe *_Nullable LensActiveNativeIntegrationProbe;
static int LensNativeIntegrationProbeExitCode = 1;

static NSDictionary *_Nullable LensJSONObjectFromOwnedCString(char *_Nullable json) {
    if (json == NULL) {
        return nil;
    }
    NSData *data = [NSData dataWithBytes:json length:strlen(json)];
    lens_free_string(json);
    id object = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    return [object isKindOfClass:NSDictionary.class] ? object : nil;
}

static BOOL LensPNGHasGreenCenterPixel(NSData *png, NSArray<NSNumber *> **rgbaOut) {
    NSBitmapImageRep *bitmap = [NSBitmapImageRep imageRepWithData:png];
    if (bitmap == nil || bitmap.pixelsWide == 0 || bitmap.pixelsHigh == 0) {
        return NO;
    }
    NSColor *color = [[bitmap colorAtX:bitmap.pixelsWide / 2
                                     y:bitmap.pixelsHigh / 2]
        colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    if (color == nil) {
        return NO;
    }
    NSInteger red = (NSInteger)round(color.redComponent * 255.0);
    NSInteger green = (NSInteger)round(color.greenComponent * 255.0);
    NSInteger blue = (NSInteger)round(color.blueComponent * 255.0);
    NSInteger alpha = (NSInteger)round(color.alphaComponent * 255.0);
    if (rgbaOut != NULL) {
        *rgbaOut = @[@(red), @(green), @(blue), @(alpha)];
    }
    return green >= 100 && green > red + 30 && green > blue + 30 && alpha >= 200;
}

static CGRect LensIntegrationFrame(NSDictionary *_Nullable value, BOOL *validOut) {
    BOOL valid = [value isKindOfClass:NSDictionary.class]
        && [value[@"x"] isKindOfClass:NSNumber.class]
        && [value[@"y"] isKindOfClass:NSNumber.class]
        && [value[@"width"] isKindOfClass:NSNumber.class]
        && [value[@"height"] isKindOfClass:NSNumber.class];
    if (validOut != NULL) {
        *validOut = valid;
    }
    return valid
        ? CGRectMake(
            [value[@"x"] doubleValue],
            [value[@"y"] doubleValue],
            [value[@"width"] doubleValue],
            [value[@"height"] doubleValue]
        )
        : CGRectNull;
}

static BOOL LensIntegrationFramesEqual(CGRect left, CGRect right) {
    return !CGRectIsNull(left)
        && !CGRectIsNull(right)
        && fabs(left.origin.x - right.origin.x) <= 1.0
        && fabs(left.origin.y - right.origin.y) <= 1.0
        && fabs(left.size.width - right.size.width) <= 1.0
        && fabs(left.size.height - right.size.height) <= 1.0;
}

static void LensNativeIntegrationObservationCallback(
    const char *json,
    void *_Nullable context
);

@interface LensNativeIntegrationProbe : NSObject
@property(nonatomic, strong) NSTask *targetTask;
@property(nonatomic, strong) SCWindow *selectedWindow;
@property(nonatomic, assign) CGWindowID selectedWindowID;
@property(nonatomic, assign) CGRect selectedFrame;
@property(nonatomic, assign) NSUInteger selectionAttempts;
@property(nonatomic, assign) NSUInteger promotionAttempts;
@property(nonatomic, assign) NSUInteger callbacksBeforeStop;
@property(nonatomic, assign) NSUInteger callbacksAfterFirstStop;
@property(nonatomic, assign) NSUInteger callbacksAfterFinalStop;
@property(nonatomic, assign) BOOL authorityValid;
@property(nonatomic, assign) BOOL titleCallbackReceived;
@property(nonatomic, assign) BOOL initialPromotionSucceeded;
@property(nonatomic, assign) BOOL observationStarted;
@property(nonatomic, assign) BOOL registeredRefreshSucceeded;
@property(nonatomic, assign) BOOL registeredTitleCurrent;
@property(nonatomic, assign) BOOL registeredFrameCurrent;
@property(nonatomic, assign) BOOL registeredCaptureSucceeded;
@property(nonatomic, assign) BOOL captureWindowBoundsCurrent;
@property(nonatomic, assign) BOOL captureRegionGeometryCurrent;
@property(nonatomic, assign) BOOL captureCenterPixelGreen;
@property(nonatomic, assign) BOOL stopSucceeded;
@property(nonatomic, assign) BOOL retainedSourceAvailableAfterStop;
@property(nonatomic, assign) BOOL resumeStarted;
@property(nonatomic, assign) BOOL resumeCallbackReceived;
@property(nonatomic, assign) BOOL resumeFactsCurrent;
@property(nonatomic, assign) BOOL finalStopSucceeded;
@property(nonatomic, assign) BOOL registrationUnavailableAfterRelease;
@property(nonatomic, assign) BOOL paused;
@property(nonatomic, assign) BOOL finalStopped;
@property(nonatomic, assign) BOOL refreshStarted;
@property(nonatomic, assign) BOOL deadlineWon;
@property(nonatomic, assign) BOOL finished;
@property(nonatomic, assign) CFTimeInterval probeStartedAt;
@property(nonatomic, strong) NSArray<NSNumber *> *centerPixelRGBA;
@property(nonatomic, strong) NSMutableArray<NSDictionary *> *callbackEvents;
@property(nonatomic, strong) NSArray<NSString *> *lastPromotionDiagnostics;
@property(nonatomic, copy) NSString *failure;
@property(nonatomic, assign) uint64_t expectedObserverEpoch;
- (void)start;
- (void)receiveObservationJSON:(const char *)json;
- (void)verifyResumeFactsThenStop;
@end

@implementation LensNativeIntegrationProbe

- (instancetype)init {
    self = [super init];
    if (self != nil) {
        _authorityValid = YES;
        _expectedObserverEpoch = LensIntegrationObserverEpoch;
        _callbackEvents = [NSMutableArray array];
        _centerPixelRGBA = @[];
        _lastPromotionDiagnostics = @[];
    }
    return self;
}

- (void)start {
    NSAssert([NSThread isMainThread], @"Product bridge probe must start on main");
    self.probeStartedAt = CFAbsoluteTimeGetCurrent();
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(12 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            if (self.finished) {
                return;
            }
            self.deadlineWon = YES;
            self.failure = @"Product native integration probe timed out after 12 seconds.";
            [self finish];
        }
    );
    NSError *launchError = nil;
    self.targetTask = [[NSTask alloc] init];
    self.targetTask.executableURL = [NSURL fileURLWithPath:NSBundle.mainBundle.executablePath];
    self.targetTask.arguments = @[@"--controlled-target"];
    if (![self.targetTask launchAndReturnError:&launchError]) {
        self.failure = launchError.localizedDescription
            ?: @"Unable to launch the controlled target process.";
        [self finish];
        return;
    }
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.20 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self selectAndStoreExactWindow];
        }
    );
}

- (void)selectAndStoreExactWindow {
    self.selectionAttempts += 1;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                                onScreenWindowsOnly:YES
                                                 completionHandler:^(
        SCShareableContent *shareableContent,
        NSError *error
    ) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            if (shareableContent == nil) {
                self.failure = error.localizedDescription
                    ?: @"ScreenCaptureKit returned no shareable content.";
                [self finish];
                return;
            }
            for (SCWindow *window in shareableContent.windows) {
                if (window.owningApplication.processID == self.targetTask.processIdentifier
                    && [window.title isEqualToString:LensIntegrationInitialTitle]) {
                    self.selectedWindow = window;
                    break;
                }
            }
            if (self.selectedWindow == nil) {
                if (self.selectionAttempts >= 20) {
                    self.failure = @"The controlled window was not returned by ScreenCaptureKit.";
                    [self finish];
                    return;
                }
                dispatch_after(
                    dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.10 * NSEC_PER_SEC)),
                    dispatch_get_main_queue(),
                    ^{
                        [self selectAndStoreExactWindow];
                    }
                );
                return;
            }
            self.selectedWindowID = self.selectedWindow.windowID;
            self.selectedFrame = self.selectedWindow.frame;
            BOOL stored = lens_test_store_picker_window(
                LensIntegrationOperationID.UTF8String,
                (__bridge void *)self.selectedWindow
            );
            if (!stored) {
                self.failure = @"The product picker registry rejected the controlled SCWindow.";
                [self finish];
                return;
            }
            [self attemptInitialPromotion];
        });
    }];
}

- (void)attemptInitialPromotion {
    if (self.finished) {
        return;
    }
    self.promotionAttempts += 1;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSDictionary *reply = LensJSONObjectFromOwnedCString(
            lens_extract_registered_window_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                128,
                32768,
                16,
                4096,
                16384
            )
        );
        NSDictionary *resolvedWindow = reply[@"resolved_window"];
        NSArray<NSString *> *diagnostics = reply[@"diagnostics"];
        BOOL promoted = [resolvedWindow isKindOfClass:NSDictionary.class]
            && [diagnostics containsObject:
                @"Extraction used the exact promoted AXWindow without heuristic re-resolution."];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            if (promoted) {
                self.initialPromotionSucceeded = YES;
                [self startProductObservation];
                return;
            }
            self.lastPromotionDiagnostics = diagnostics ?: @[];
            if (self.promotionAttempts >= 20) {
                self.failure = [NSString stringWithFormat:
                    @"The product bridge could not promote the controlled AXWindow: %@",
                    [self.lastPromotionDiagnostics componentsJoinedByString:@"; "]];
                [self finish];
                return;
            }
            dispatch_after(
                dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.05 * NSEC_PER_SEC)),
                dispatch_get_main_queue(),
                ^{
                    [self attemptInitialPromotion];
                }
            );
        });
    });
}

- (void)startProductObservation {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        bool started = false;
        NSDictionary *reply = LensJSONObjectFromOwnedCString(
            lens_start_window_observation_json(
                LensIntegrationOperationID.UTF8String,
                LensIntegrationContextID.UTF8String,
                LensIntegrationSourceID.UTF8String,
                LensIntegrationObserverEpoch,
                self.selectedWindowID,
                LensNativeIntegrationObservationCallback,
                (__bridge void *)self,
                &started
            )
        );
        BOOL validStart = started && [reply[@"status"] isEqualToString:@"started"];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            if (!validStart) {
                self.failure = reply[@"message"] ?: @"Product AXObserver start failed.";
                [self finish];
                return;
            }
            self.observationStarted = YES;
            if (![self mutateControlledTarget]) {
                self.failure = @"Unable to request the controlled target's first mutation.";
                [self finish];
            }
        });
    });
}

- (BOOL)mutateControlledTarget {
    pid_t targetPID = self.targetTask.processIdentifier;
    return self.targetTask.running && targetPID > 0 && kill(targetPID, SIGUSR1) == 0;
}

- (void)receiveObservationJSON:(const char *)json {
    NSAssert([NSThread isMainThread], @"Product callbacks must arrive on main run loop");
    if (json == NULL || self.finished) {
        return;
    }
    NSData *data = [NSData dataWithBytes:json length:strlen(json)];
    NSDictionary *event = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    if (![event isKindOfClass:NSDictionary.class]) {
        self.authorityValid = NO;
        return;
    }
    if (self.paused) {
        if (self.finalStopped) {
            self.callbacksAfterFinalStop += 1;
        } else {
            self.callbacksAfterFirstStop += 1;
        }
        return;
    }
    self.callbacksBeforeStop += 1;
    BOOL eventAuthorityValid =
        [event[@"operation_id"] isEqualToString:LensIntegrationOperationID]
        && [event[@"context_id"] isEqualToString:LensIntegrationContextID]
        && [event[@"source_registration_id"] isEqualToString:LensIntegrationSourceID]
        && [event[@"observer_epoch"] unsignedLongLongValue] == self.expectedObserverEpoch
        && [event[@"window_id"] unsignedIntValue] == self.selectedWindowID;
    self.authorityValid = self.authorityValid && eventAuthorityValid;
    if (self.callbackEvents.count < 16) {
        [self.callbackEvents addObject:event];
    }
    if ([event[@"notification"] isEqualToString:@"window_title_changed"]
        && !self.refreshStarted) {
        self.titleCallbackReceived = YES;
        self.refreshStarted = YES;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.15 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{
                [self runRegisteredRefreshAndCapture];
            }
        );
    } else if ([event[@"notification"] isEqualToString:@"window_title_changed"]
        && self.resumeStarted
        && !self.resumeCallbackReceived
        && [event[@"observer_epoch"] unsignedLongLongValue]
            == LensIntegrationResumeObserverEpoch) {
        self.resumeCallbackReceived = YES;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.10 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{
                [self verifyResumeFactsThenStop];
            }
        );
    }
}

- (void)runRegisteredRefreshAndCapture {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSDictionary *extraction = LensJSONObjectFromOwnedCString(
            lens_extract_registered_window_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                128,
                32768,
                16,
                4096,
                16384
            )
        );
        NSDictionary *resolvedWindow = extraction[@"resolved_window"];
        NSDictionary *facts = resolvedWindow[@"facts"];
        BOOL observedFrameValid = NO;
        CGRect observedFrame = LensIntegrationFrame(facts[@"frame"], &observedFrameValid);
        NSArray<NSString *> *diagnostics = extraction[@"diagnostics"];
        BOOL exactDiagnostic = [diagnostics containsObject:
            @"Extraction used the exact promoted AXWindow without heuristic re-resolution."];
        self.registeredTitleCurrent = [facts[@"title"]
            isEqualToString:LensIntegrationChangedTitle];
        self.registeredFrameCurrent = observedFrameValid
            && fabs(observedFrame.size.width - self.selectedFrame.size.width - 120.0) <= 1.0
            && fabs(observedFrame.size.height - self.selectedFrame.size.height - 80.0) <= 1.0
            && (fabs(observedFrame.origin.x - self.selectedFrame.origin.x) > 1.0
                || fabs(observedFrame.origin.y - self.selectedFrame.origin.y) > 1.0);
        self.registeredRefreshSucceeded =
            [resolvedWindow isKindOfClass:NSDictionary.class]
            && exactDiagnostic
            && self.registeredTitleCurrent
            && self.registeredFrameCurrent;

        CGRect requestedRegion = CGRectMake(
            observedFrame.origin.x + 80.0,
            observedFrame.origin.y + 70.0,
            120.0,
            90.0
        );
        NSArray *requests = @[@{
            @"id": @"probe-region",
            @"scope": @"ax_element_region",
            @"source_node_id": @"probe-node",
            @"bounds": @{
                @"x": @(requestedRegion.origin.x),
                @"y": @(requestedRegion.origin.y),
                @"width": @(requestedRegion.size.width),
                @"height": @(requestedRegion.size.height),
            },
        }];
        NSData *requestsData = [NSJSONSerialization
            dataWithJSONObject:requests
            options:0
            error:NULL];
        NSString *requestsJSON = [[NSString alloc]
            initWithData:requestsData
            encoding:NSUTF8StringEncoding];
        NSDictionary *capture = LensJSONObjectFromOwnedCString(
            lens_capture_registered_window_regions_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                requestsJSON.UTF8String,
                2048,
                4194304,
                4194304,
                4194304
            )
        );
        NSArray<NSDictionary *> *captures = capture[@"captures"];
        NSDictionary *firstCapture = captures.firstObject;
        BOOL captureWindowFrameValid = NO;
        CGRect captureWindowFrame = LensIntegrationFrame(
            capture[@"window_bounds"],
            &captureWindowFrameValid
        );
        BOOL sourceBoundsValid = NO;
        CGRect sourceBounds = LensIntegrationFrame(
            firstCapture[@"source_bounds"],
            &sourceBoundsValid
        );
        BOOL capturedBoundsValid = NO;
        CGRect capturedBounds = LensIntegrationFrame(
            firstCapture[@"captured_bounds"],
            &capturedBoundsValid
        );
        self.captureWindowBoundsCurrent = captureWindowFrameValid
            && LensIntegrationFramesEqual(captureWindowFrame, observedFrame);
        self.captureRegionGeometryCurrent = sourceBoundsValid
            && capturedBoundsValid
            && LensIntegrationFramesEqual(sourceBounds, requestedRegion)
            && LensIntegrationFramesEqual(capturedBounds, requestedRegion)
            && [firstCapture[@"coverage"] isEqualToString:@"full_region"];
        NSData *png = [[NSData alloc]
            initWithBase64EncodedString:firstCapture[@"data"] ?: @""
                             options:0];
        self.registeredCaptureSucceeded =
            captures.count == 1
            && [firstCapture[@"attachment_id"] isEqualToString:@"probe-region"]
            && [firstCapture[@"mime_type"] isEqualToString:@"image/png"]
            && self.captureWindowBoundsCurrent
            && self.captureRegionGeometryCurrent
            && png.length > 0;
        if (self.registeredCaptureSucceeded) {
            NSArray<NSNumber *> *rgba = nil;
            self.captureCenterPixelGreen = LensPNGHasGreenCenterPixel(png, &rgba);
            self.centerPixelRGBA = rgba ?: @[];
        }

        dispatch_async(dispatch_get_main_queue(), ^{
            [self stopAndVerifyNoLateCallback];
        });
    });
}

- (void)stopAndVerifyNoLateCallback {
    if (self.finished) {
        return;
    }
    self.stopSucceeded = lens_stop_window_observation(
        LensIntegrationOperationID.UTF8String,
        LensIntegrationSourceID.UTF8String
    );
    self.paused = YES;
    if (!self.stopSucceeded) {
        self.failure = @"The first product observer stop failed.";
        [self finish];
        return;
    }
    if (![self mutateControlledTarget]) {
        self.failure = @"Unable to mutate the controlled target after the first stop.";
        [self finish];
        return;
    }
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.35 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self verifyRetainedSourceAndResume];
        }
    );
}

- (void)verifyRetainedSourceAndResume {
    if (self.finished) {
        return;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSDictionary *afterStop = LensJSONObjectFromOwnedCString(
            lens_extract_registered_window_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                32,
                4096,
                4,
                1024,
                4096
            )
        );
        NSDictionary *resolvedWindow = afterStop[@"resolved_window"];
        NSDictionary *facts = resolvedWindow[@"facts"];
        NSArray<NSString *> *diagnostics = afterStop[@"diagnostics"];
        BOOL retained = [resolvedWindow isKindOfClass:NSDictionary.class]
            && [facts[@"title"] isEqualToString:LensIntegrationAfterStopTitle]
            && [diagnostics containsObject:
                @"Extraction used the exact promoted AXWindow without heuristic re-resolution."];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            self.retainedSourceAvailableAfterStop = retained;
            self.expectedObserverEpoch = LensIntegrationResumeObserverEpoch;
            self.paused = NO;
            [self resumeProductObservation];
        });
    });
}

- (void)resumeProductObservation {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        bool started = false;
        NSDictionary *reply = LensJSONObjectFromOwnedCString(
            lens_start_window_observation_json(
                LensIntegrationOperationID.UTF8String,
                LensIntegrationContextID.UTF8String,
                LensIntegrationSourceID.UTF8String,
                LensIntegrationResumeObserverEpoch,
                self.selectedWindowID,
                LensNativeIntegrationObservationCallback,
                (__bridge void *)self,
                &started
            )
        );
        BOOL validStart = started && [reply[@"status"] isEqualToString:@"started"];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            if (!validStart) {
                self.failure = reply[@"message"]
                    ?: @"The fixed source could not resume product observation.";
                self.paused = YES;
                [self finish];
                return;
            }
            self.resumeStarted = YES;
            if (![self mutateControlledTarget]) {
                self.failure = @"Unable to request the controlled target's resume mutation.";
                [self finish];
            }
        });
    });
}

- (void)verifyResumeFactsThenStop {
    if (self.finished || self.finalStopped) {
        return;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSDictionary *refresh = LensJSONObjectFromOwnedCString(
            lens_extract_registered_window_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                32,
                4096,
                4,
                1024,
                4096
            )
        );
        NSDictionary *facts = refresh[@"resolved_window"][@"facts"];
        NSArray<NSString *> *diagnostics = refresh[@"diagnostics"];
        BOOL current = [facts[@"title"] isEqualToString:LensIntegrationResumeTitle]
            && [diagnostics containsObject:
                @"Extraction used the exact promoted AXWindow without heuristic re-resolution."];
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self.finished) {
                return;
            }
            self.resumeFactsCurrent = current;
            [self stopAfterResumeAndVerifyNoLateCallback];
        });
    });
}

- (void)stopAfterResumeAndVerifyNoLateCallback {
    if (self.finished || self.finalStopped) {
        return;
    }
    self.finalStopSucceeded = lens_stop_window_observation(
        LensIntegrationOperationID.UTF8String,
        LensIntegrationSourceID.UTF8String
    );
    self.finalStopped = YES;
    self.paused = YES;
    if (![self mutateControlledTarget]) {
        self.failure = @"Unable to mutate the controlled target after the final stop.";
        [self finish];
        return;
    }
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.35 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self releaseExactSourceAndFinish];
        }
    );
}

- (void)releaseExactSourceAndFinish {
    if (self.finished) {
        return;
    }
    BOOL released = lens_release_registered_window(
        LensIntegrationOperationID.UTF8String,
        self.selectedWindowID
    );
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSDictionary *afterRelease = LensJSONObjectFromOwnedCString(
            lens_extract_registered_window_json(
                LensIntegrationOperationID.UTF8String,
                self.selectedWindowID,
                32,
                4096,
                4,
                1024,
                4096
            )
        );
        NSArray<NSString *> *diagnostics = afterRelease[@"diagnostics"];
        BOOL unavailable = released
            && [afterRelease[@"quality"] isEqualToString:@"unavailable"]
            && [diagnostics containsObject:
                @"The exact operation-scoped selected window is unavailable."];
        dispatch_async(dispatch_get_main_queue(), ^{
            self.registrationUnavailableAfterRelease = unavailable;
            [self finish];
        });
    });
}

- (void)finish {
    if (self.finished) {
        return;
    }
    self.finished = YES;
    if (self.observationStarted && !self.paused) {
        lens_stop_window_observation(
            LensIntegrationOperationID.UTF8String,
            LensIntegrationSourceID.UTF8String
        );
    }
    lens_release_window_operation(LensIntegrationOperationID.UTF8String);
    BOOL passed = self.failure == nil
        && self.selectedWindowID != 0
        && self.initialPromotionSucceeded
        && self.observationStarted
        && self.titleCallbackReceived
        && self.callbacksBeforeStop >= 1
        && self.authorityValid
        && self.registeredRefreshSucceeded
        && self.registeredTitleCurrent
        && self.registeredFrameCurrent
        && self.registeredCaptureSucceeded
        && self.captureWindowBoundsCurrent
        && self.captureRegionGeometryCurrent
        && self.captureCenterPixelGreen
        && self.stopSucceeded
        && self.retainedSourceAvailableAfterStop
        && self.callbacksAfterFirstStop == 0
        && self.resumeStarted
        && self.resumeCallbackReceived
        && self.resumeFactsCurrent
        && self.finalStopSucceeded
        && self.registrationUnavailableAfterRelease
        && self.callbacksAfterFinalStop == 0
        && !self.deadlineWon;
    NSDictionary *result = @{
        @"status": passed ? @"passed" : @"failed",
        @"trusted": @(AXIsProcessTrusted()),
        @"screen_capture_access": @(CGPreflightScreenCaptureAccess()),
        @"selected_window_id": @(self.selectedWindowID),
        @"promotion_attempts": @(self.promotionAttempts),
        @"last_promotion_diagnostics": self.lastPromotionDiagnostics,
        @"initial_promotion_succeeded": @(self.initialPromotionSucceeded),
        @"observation_started": @(self.observationStarted),
        @"callbacks_before_stop": @(self.callbacksBeforeStop),
        @"callbacks_after_first_stop": @(self.callbacksAfterFirstStop),
        @"callbacks_after_final_stop": @(self.callbacksAfterFinalStop),
        @"authority_valid": @(self.authorityValid),
        @"title_callback_received": @(self.titleCallbackReceived),
        @"registered_refresh_succeeded": @(self.registeredRefreshSucceeded),
        @"registered_title_current": @(self.registeredTitleCurrent),
        @"registered_frame_current": @(self.registeredFrameCurrent),
        @"registered_capture_succeeded": @(self.registeredCaptureSucceeded),
        @"capture_window_bounds_current": @(self.captureWindowBoundsCurrent),
        @"capture_region_geometry_current": @(self.captureRegionGeometryCurrent),
        @"capture_center_pixel_green": @(self.captureCenterPixelGreen),
        @"center_pixel_rgba": self.centerPixelRGBA,
        @"stop_succeeded": @(self.stopSucceeded),
        @"retained_source_available_after_stop":
            @(self.retainedSourceAvailableAfterStop),
        @"resume_started": @(self.resumeStarted),
        @"resume_callback_received": @(self.resumeCallbackReceived),
        @"resume_facts_current": @(self.resumeFactsCurrent),
        @"final_stop_succeeded": @(self.finalStopSucceeded),
        @"registration_unavailable_after_release":
            @(self.registrationUnavailableAfterRelease),
        @"deadline_won": @(self.deadlineWon),
        @"probe_duration_ms":
            @(round((CFAbsoluteTimeGetCurrent() - self.probeStartedAt) * 1000.0)),
        @"callback_events": self.callbackEvents,
        @"failure": self.failure ?: [NSNull null],
    };
    NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
    NSString *jsonString = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    printf("LENS_NATIVE_INTEGRATION_PROBE=%s\n", jsonString.UTF8String);
    fflush(stdout);
    LensNativeIntegrationProbeExitCode = passed ? 0 : 1;
    if (self.targetTask.running) {
        [self.targetTask terminate];
    }
    LensActiveNativeIntegrationProbe = nil;
    [NSApp terminate:nil];
}

@end

static void LensNativeIntegrationObservationCallback(
    const char *json,
    void *_Nullable context
) {
    if (context == NULL) {
        return;
    }
    LensNativeIntegrationProbe *probe = (__bridge LensNativeIntegrationProbe *)context;
    [probe receiveObservationJSON:json];
}

static NSWindow *_Nullable LensControlledTargetWindow;
static dispatch_source_t _Nullable LensControlledTargetSignalSource;

static int LensRunControlledTarget(void) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    LensControlledTargetWindow = [[NSWindow alloc]
        initWithContentRect:NSMakeRect(140.0, 140.0, 520.0, 280.0)
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                    backing:NSBackingStoreBuffered
                      defer:NO];
    LensControlledTargetWindow.title = LensIntegrationInitialTitle;
    LensControlledTargetWindow.sharingType = NSWindowSharingReadOnly;
    LensControlledTargetWindow.contentView.wantsLayer = YES;
    LensControlledTargetWindow.contentView.layer.backgroundColor =
        NSColor.systemBlueColor.CGColor;

    signal(SIGUSR1, SIG_IGN);
    LensControlledTargetSignalSource = dispatch_source_create(
        DISPATCH_SOURCE_TYPE_SIGNAL,
        SIGUSR1,
        0,
        dispatch_get_main_queue()
    );
    __block NSUInteger mutation = 0;
    dispatch_source_set_event_handler(LensControlledTargetSignalSource, ^{
        mutation += 1;
        switch (mutation) {
            case 1: {
                LensControlledTargetWindow.title = LensIntegrationChangedTitle;
                NSRect frame = LensControlledTargetWindow.frame;
                frame.origin.x += 64.0;
                frame.origin.y += 48.0;
                frame.size.width += 120.0;
                frame.size.height += 80.0;
                [LensControlledTargetWindow setFrame:frame display:YES];
                LensControlledTargetWindow.contentView.layer.backgroundColor =
                    NSColor.systemGreenColor.CGColor;
                [LensControlledTargetWindow.contentView setNeedsDisplay:YES];
                break;
            }
            case 2:
                LensControlledTargetWindow.title = LensIntegrationAfterStopTitle;
                break;
            case 3:
                LensControlledTargetWindow.title = LensIntegrationResumeTitle;
                break;
            default:
                LensControlledTargetWindow.title = LensIntegrationFinalStopTitle;
                break;
        }
    });
    dispatch_resume(LensControlledTargetSignalSource);
    [LensControlledTargetWindow makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    [NSApp run];
    return 0;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc == 2 && strcmp(argv[1], "--controlled-target") == 0) {
            return LensRunControlledTarget();
        }
        if (!AXIsProcessTrusted() || !CGPreflightScreenCaptureAccess()) {
            NSDictionary *result = @{
                @"status": @"permission_required",
                @"trusted": @(AXIsProcessTrusted()),
                @"screen_capture_access": @(CGPreflightScreenCaptureAccess()),
            };
            NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
            NSString *jsonString = [[NSString alloc]
                initWithData:json
                    encoding:NSUTF8StringEncoding];
            printf("LENS_NATIVE_INTEGRATION_PROBE=%s\n", jsonString.UTF8String);
            return 2;
        }

        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        LensActiveNativeIntegrationProbe = [[LensNativeIntegrationProbe alloc] init];
        dispatch_async(dispatch_get_main_queue(), ^{
            [LensActiveNativeIntegrationProbe start];
        });
        [NSApp run];
        return LensNativeIntegrationProbeExitCode;
    }
}
