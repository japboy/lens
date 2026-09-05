#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <math.h>

static NSString *const LensCaptureProbeInitialTitle = @"Lens Retained SCWindow Probe Initial";
static NSString *const LensCaptureProbeChangedTitle = @"Lens Retained SCWindow Probe Changed";

static BOOL LensCaptureProbeScalarEqual(CGFloat left, CGFloat right) {
    return fabs(left - right) <= 1.0;
}

static BOOL LensCopyCenterPixel(CGImageRef image, uint8_t pixel[4]) {
    size_t width = CGImageGetWidth(image);
    size_t height = CGImageGetHeight(image);
    if (width == 0 || height == 0) {
        return NO;
    }
    CGRect sampleRect = CGRectMake((CGFloat)(width / 2), (CGFloat)(height / 2), 1.0, 1.0);
    CGImageRef sample = CGImageCreateWithImageInRect(image, sampleRect);
    if (sample == NULL) {
        return NO;
    }
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(
        pixel,
        1,
        1,
        8,
        4,
        colorSpace,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big
    );
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) {
        CGImageRelease(sample);
        return NO;
    }
    CGContextDrawImage(context, CGRectMake(0.0, 0.0, 1.0, 1.0), sample);
    CGContextRelease(context);
    CGImageRelease(sample);
    return YES;
}

@class LensWindowCaptureProbe;
static LensWindowCaptureProbe *_Nullable LensActiveWindowCaptureProbe;
static int LensWindowCaptureProbeExitCode = 1;

@interface LensWindowCaptureProbe : NSObject
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) SCWindow *selectedWindow;
@property(nonatomic, assign) CGWindowID selectedWindowID;
@property(nonatomic, assign) CGRect selectedFrame;
@property(nonatomic, assign) CGRect changedFrame;
@property(nonatomic, assign) CGRect captureContentRect;
@property(nonatomic, assign) size_t capturedPixelWidth;
@property(nonatomic, assign) size_t capturedPixelHeight;
@property(nonatomic, assign) CFTimeInterval captureStartedAt;
@property(nonatomic, assign) CFTimeInterval captureDuration;
@property(nonatomic, assign) CFTimeInterval probeStartedAt;
@property(nonatomic, assign) CFTimeInterval probeDuration;
@property(nonatomic, assign) uint64_t captureGeneration;
@property(nonatomic, assign) BOOL completionWon;
@property(nonatomic, assign) BOOL timeoutWon;
@property(nonatomic, assign) BOOL probeTimeoutWon;
@property(nonatomic, assign) BOOL finished;
@property(nonatomic, assign) BOOL centerPixelGreen;
@property(nonatomic, assign) uint8_t centerRed;
@property(nonatomic, assign) uint8_t centerGreen;
@property(nonatomic, assign) uint8_t centerBlue;
@property(nonatomic, assign) uint8_t centerAlpha;
@property(nonatomic, copy) NSString *failure;
- (void)start;
@end

@implementation LensWindowCaptureProbe

- (void)start {
    NSAssert([NSThread isMainThread], @"SCWindow probe must start on the AppKit main thread");
    self.probeStartedAt = CFAbsoluteTimeGetCurrent();
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(8 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            if (self.finished) {
                return;
            }
            self.probeTimeoutWon = YES;
            self.failure = @"Retained SCWindow probe timed out after 8 seconds.";
            [self finish];
        }
    );
    [self createWindow];
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.20 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self selectRetainedWindow];
        }
    );
}

- (void)createWindow {
    NSRect frame = NSMakeRect(120.0, 120.0, 520.0, 280.0);
    self.window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                    backing:NSBackingStoreBuffered
                      defer:NO];
    self.window.title = LensCaptureProbeInitialTitle;
    self.window.sharingType = NSWindowSharingReadOnly;
    NSView *content = self.window.contentView;
    content.wantsLayer = YES;
    content.layer.backgroundColor = NSColor.systemBlueColor.CGColor;
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}

- (void)selectRetainedWindow {
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
                if (window.owningApplication.processID == getpid()
                    && [window.title isEqualToString:LensCaptureProbeInitialTitle]) {
                    self.selectedWindow = window;
                    break;
                }
            }
            if (self.selectedWindow == nil) {
                self.failure = @"The probe window was not present in SCShareableContent.";
                [self finish];
                return;
            }

            self.selectedWindowID = self.selectedWindow.windowID;
            self.selectedFrame = self.selectedWindow.frame;
            [self mutateWindowWithoutRequery];
        });
    }];
}

- (void)mutateWindowWithoutRequery {
    self.window.title = LensCaptureProbeChangedTitle;
    NSRect frame = self.window.frame;
    frame.origin.x += 64.0;
    frame.origin.y += 48.0;
    frame.size.width += 120.0;
    frame.size.height += 80.0;
    [self.window setFrame:frame display:YES];
    self.changedFrame = self.window.frame;
    self.window.contentView.layer.backgroundColor = NSColor.systemGreenColor.CGColor;

    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.20 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            [self captureRetainedWindow];
        }
    );
}

- (void)captureRetainedWindow {
    self.captureGeneration += 1;
    uint64_t generation = self.captureGeneration;
    self.captureStartedAt = CFAbsoluteTimeGetCurrent();

    SCContentFilter *filter = [[SCContentFilter alloc]
        initWithDesktopIndependentWindow:self.selectedWindow];
    self.captureContentRect = filter.contentRect;
    SCStreamConfiguration *configuration = [[SCStreamConfiguration alloc] init];
    configuration.width = (size_t)llround(self.changedFrame.size.width * 2.0);
    configuration.height = (size_t)llround(self.changedFrame.size.height * 2.0);
    configuration.scalesToFit = YES;
    configuration.preservesAspectRatio = YES;
    configuration.showsCursor = NO;
    configuration.ignoreShadowsSingleWindow = YES;
    configuration.shouldBeOpaque = YES;
    [SCScreenshotManager
        captureImageWithFilter:filter
        configuration:configuration
        completionHandler:^(CGImageRef image, NSError *error) {
            CGImageRef retainedImage = image == NULL ? NULL : CGImageRetain(image);
            dispatch_async(dispatch_get_main_queue(), ^{
                if (self.finished
                    || generation != self.captureGeneration
                    || self.timeoutWon
                    || self.completionWon) {
                    if (retainedImage != NULL) {
                        CGImageRelease(retainedImage);
                    }
                    return;
                }
                self.completionWon = YES;
                self.captureDuration = CFAbsoluteTimeGetCurrent() - self.captureStartedAt;
                if (retainedImage == NULL) {
                    self.failure = error.localizedDescription
                        ?: @"Retained SCWindow capture returned no image.";
                } else {
                    self.capturedPixelWidth = CGImageGetWidth(retainedImage);
                    self.capturedPixelHeight = CGImageGetHeight(retainedImage);
                    uint8_t pixel[4] = {0, 0, 0, 0};
                    if (LensCopyCenterPixel(retainedImage, pixel)) {
                        self.centerRed = pixel[0];
                        self.centerGreen = pixel[1];
                        self.centerBlue = pixel[2];
                        self.centerAlpha = pixel[3];
                        self.centerPixelGreen = self.centerGreen >= 100
                            && self.centerGreen > self.centerRed + 30
                            && self.centerGreen > self.centerBlue + 30
                            && self.centerAlpha >= 200;
                    }
                    CGImageRelease(retainedImage);
                }
                [self finish];
            });
        }];

    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{
            if (self.finished
                || generation != self.captureGeneration
                || self.completionWon
                || self.timeoutWon) {
                return;
            }
            self.timeoutWon = YES;
            self.captureDuration = CFAbsoluteTimeGetCurrent() - self.captureStartedAt;
            self.failure = @"Retained SCWindow capture timed out after 5 seconds.";
            [self finish];
        }
    );
}

- (void)finish {
    if (self.finished) {
        return;
    }
    self.finished = YES;
    self.probeDuration = CFAbsoluteTimeGetCurrent() - self.probeStartedAt;
    BOOL frameChanged = !CGRectEqualToRect(self.selectedFrame, self.changedFrame);
    BOOL originChanged = !LensCaptureProbeScalarEqual(
        self.selectedFrame.origin.x,
        self.changedFrame.origin.x
    ) || !LensCaptureProbeScalarEqual(
        self.selectedFrame.origin.y,
        self.changedFrame.origin.y
    );
    BOOL sizeChanged = !LensCaptureProbeScalarEqual(
        self.selectedFrame.size.width,
        self.changedFrame.size.width
    ) || !LensCaptureProbeScalarEqual(
        self.selectedFrame.size.height,
        self.changedFrame.size.height
    );
    BOOL filterRetainsSelectedSize = LensCaptureProbeScalarEqual(
        self.captureContentRect.size.width,
        self.selectedFrame.size.width
    ) && LensCaptureProbeScalarEqual(
        self.captureContentRect.size.height,
        self.selectedFrame.size.height
    );
    BOOL captureUsesCurrentSize = self.capturedPixelWidth
            == (size_t)llround(self.changedFrame.size.width * 2.0)
        && self.capturedPixelHeight
            == (size_t)llround(self.changedFrame.size.height * 2.0);
    BOOL titleChanged = [self.window.title isEqualToString:LensCaptureProbeChangedTitle];
    BOOL passed = self.failure == nil
        && self.selectedWindow != nil
        && self.selectedWindowID != 0
        && self.selectedWindow.windowID == self.selectedWindowID
        && titleChanged
        && frameChanged
        && originChanged
        && sizeChanged
        && captureUsesCurrentSize
        && self.completionWon
        && !self.timeoutWon
        && !self.probeTimeoutWon
        && self.capturedPixelWidth > 0
        && self.capturedPixelHeight > 0
        && self.centerPixelGreen;
    NSDictionary *result = @{
        @"status": passed ? @"passed" : @"failed",
        @"screen_capture_access": @(CGPreflightScreenCaptureAccess()),
        @"selected_window_id": @(self.selectedWindowID),
        @"retained_window_id_after_mutation": @(self.selectedWindow.windowID),
        @"title_changed_without_requery": @(titleChanged),
        @"frame_changed_without_requery": @(frameChanged),
        @"frame_origin_changed_without_requery": @(originChanged),
        @"frame_size_changed_without_requery": @(sizeChanged),
        @"retained_filter_matches_picker_size": @(filterRetainsSelectedSize),
        @"capture_uses_current_size": @(captureUsesCurrentSize),
        @"selected_frame": @{
            @"x": @(self.selectedFrame.origin.x),
            @"y": @(self.selectedFrame.origin.y),
            @"width": @(self.selectedFrame.size.width),
            @"height": @(self.selectedFrame.size.height),
        },
        @"changed_frame": @{
            @"x": @(self.changedFrame.origin.x),
            @"y": @(self.changedFrame.origin.y),
            @"width": @(self.changedFrame.size.width),
            @"height": @(self.changedFrame.size.height),
        },
        @"capture_filter_content_rect": @{
            @"x": @(self.captureContentRect.origin.x),
            @"y": @(self.captureContentRect.origin.y),
            @"width": @(self.captureContentRect.size.width),
            @"height": @(self.captureContentRect.size.height),
        },
        @"completion_won": @(self.completionWon),
        @"timeout_won": @(self.timeoutWon),
        @"probe_timeout_won": @(self.probeTimeoutWon),
        @"probe_duration_ms": @(round(self.probeDuration * 1000.0)),
        @"capture_duration_ms": @(round(self.captureDuration * 1000.0)),
        @"captured_pixel_width": @(self.capturedPixelWidth),
        @"captured_pixel_height": @(self.capturedPixelHeight),
        @"center_pixel_green": @(self.centerPixelGreen),
        @"center_pixel_rgba": @[
            @(self.centerRed),
            @(self.centerGreen),
            @(self.centerBlue),
            @(self.centerAlpha),
        ],
        @"failure": self.failure ?: [NSNull null],
    };
    NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
    NSString *jsonString = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    printf("LENS_SCWINDOW_PROBE=%s\n", jsonString.UTF8String);
    fflush(stdout);
    LensWindowCaptureProbeExitCode = passed ? 0 : 1;
    [self.window close];
    LensActiveWindowCaptureProbe = nil;
    [NSApp terminate:nil];
}

@end


int main(void) {
    @autoreleasepool {
        if (!CGPreflightScreenCaptureAccess()) {
            NSDictionary *result = @{
                @"status": @"permission_required",
                @"screen_capture_access": @NO,
            };
            NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
            NSString *jsonString = [[NSString alloc]
                initWithData:json
                    encoding:NSUTF8StringEncoding];
            printf("LENS_SCWINDOW_PROBE=%s\n", jsonString.UTF8String);
            return 2;
        }

        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        LensActiveWindowCaptureProbe = [[LensWindowCaptureProbe alloc] init];
        dispatch_async(dispatch_get_main_queue(), ^{
            [LensActiveWindowCaptureProbe start];
        });
        [NSApp run];
        return LensWindowCaptureProbeExitCode;
    }
}
