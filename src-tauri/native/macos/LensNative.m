#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

typedef void (*LensPickerCallback)(const char *_Nullable json, void *_Nullable context);

static NSDictionary *LensFrameDictionary(CGRect frame);

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

@interface LensContentPickerCoordinator : NSObject <SCContentSharingPickerObserver>
@property(nonatomic, assign) LensPickerCallback callback;
@property(nonatomic, assign) void *callbackContext;
@property(nonatomic, assign) BOOL observing;
+ (instancetype)shared;
- (BOOL)presentWithCallback:(LensPickerCallback)callback context:(void *)context;
@end

@implementation LensContentPickerCoordinator

+ (instancetype)shared {
    static LensContentPickerCoordinator *coordinator;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        coordinator = [[LensContentPickerCoordinator alloc] init];
    });
    return coordinator;
}

- (void)deliver:(NSDictionary *)payload {
    LensPickerCallback callback = self.callback;
    void *context = self.callbackContext;
    self.callback = NULL;
    self.callbackContext = NULL;

    if (callback != NULL) {
        char *json = LensCopyJSONString(payload);
        callback(json, context);
        free(json);
    }
}

- (BOOL)presentWithCallback:(LensPickerCallback)callback context:(void *)context {
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
            @"title": LensStringOrEmpty(window.title),
            @"application_name": LensStringOrEmpty(application.applicationName),
            @"bundle_id": LensStringOrEmpty(application.bundleIdentifier),
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
        @"message": @"Lens requires macOS 15.2 or later for deterministic SCWindow resolution."
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

bool lens_accessibility_is_trusted(void) {
    return AXIsProcessTrusted();
}

bool lens_accessibility_request_trust(void) {
    NSDictionary *options = @{ (__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES };
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
}

bool lens_present_window_picker(LensPickerCallback callback, void *context) {
    if (callback == NULL) {
        return false;
    }

    if ([NSThread isMainThread]) {
        return [[LensContentPickerCoordinator shared] presentWithCallback:callback context:context];
    }

    __block BOOL presented = NO;
    dispatch_sync(dispatch_get_main_queue(), ^{
        presented = [[LensContentPickerCoordinator shared] presentWithCallback:callback context:context];
    });
    return presented;
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

char *lens_capture_window_regions_json(
    uint32_t windowID,
    const char *requestsJSON,
    uint32_t maxLongEdge,
    uint32_t maxPixels,
    uint32_t maxAttachmentBytes,
    uint32_t maxTotalBytes
) {
    @autoreleasepool {
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
                requestError.localizedDescription
                    ?: @"Image capture requests must be a JSON array."
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

        dispatch_semaphore_t completion = dispatch_semaphore_create(0);
        __block NSDictionary *result = nil;
        [SCShareableContent getShareableContentWithCompletionHandler:^(
            SCShareableContent *shareableContent,
            NSError *shareableError
        ) {
            if (shareableContent == nil) {
                result = LensImageCaptureFailure(
                    shareableError.localizedDescription
                        ?: @"ScreenCaptureKit returned no shareable content."
                );
                dispatch_semaphore_signal(completion);
                return;
            }

            SCWindow *selectedWindow = nil;
            for (SCWindow *window in shareableContent.windows) {
                if (window.windowID == windowID) {
                    selectedWindow = window;
                    break;
                }
            }
            if (selectedWindow == nil) {
                result = LensImageCaptureFailure(
                    @"The picker-authoritative window is no longer available to ScreenCaptureKit."
                );
                dispatch_semaphore_signal(completion);
                return;
            }

            CGFloat sourceWidth = MAX(selectedWindow.frame.size.width, 1.0);
            CGFloat sourceHeight = MAX(selectedWindow.frame.size.height, 1.0);
            CGFloat scale = MIN(2.0, MIN(4096.0 / sourceWidth, 4096.0 / sourceHeight));
            SCStreamConfiguration *configuration = [[SCStreamConfiguration alloc] init];
            configuration.width = (size_t)MAX(1.0, floor(sourceWidth * scale));
            configuration.height = (size_t)MAX(1.0, floor(sourceHeight * scale));
            configuration.scalesToFit = YES;
            configuration.preservesAspectRatio = YES;
            configuration.showsCursor = NO;
            configuration.ignoreShadowsSingleWindow = YES;
            configuration.shouldBeOpaque = YES;
            SCContentFilter *filter = [[SCContentFilter alloc]
                initWithDesktopIndependentWindow:selectedWindow];
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

                    CGRect windowFrame = selectedWindow.frame;
                    CGFloat pixelScaleX = (CGFloat)CGImageGetWidth(image) / sourceWidth;
                    CGFloat pixelScaleY = (CGFloat)CGImageGetHeight(image) / sourceHeight;
                    NSUInteger totalBytes = 0;
                    NSMutableArray<NSDictionary *> *captures = [NSMutableArray array];
                    NSMutableArray<NSDictionary *> *omissions = [NSMutableArray array];
                    for (NSDictionary *request in requests) {
                        NSString *attachmentID = request[@"id"];
                        NSString *scope = request[@"scope"];
                        if (attachmentID.length == 0 || scope.length == 0) {
                            [omissions addObject:@{
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
                        CGRect capturedBounds = CGRectIntersection(windowFrame, requestedBounds);
                        if (CGRectIsNull(capturedBounds) || CGRectIsEmpty(capturedBounds)) {
                            [omissions addObject:@{
                                @"attachment_id": attachmentID,
                                @"reason": @"outside_window",
                                @"detail": @"AX image region does not intersect the current selected-window frame."
                            }];
                            continue;
                        }

                        CGFloat localMinX =
                            (CGRectGetMinX(capturedBounds) - CGRectGetMinX(windowFrame)) * pixelScaleX;
                        CGFloat localMinY =
                            (CGRectGetMinY(capturedBounds) - CGRectGetMinY(windowFrame)) * pixelScaleY;
                        CGFloat localMaxX =
                            (CGRectGetMaxX(capturedBounds) - CGRectGetMinX(windowFrame)) * pixelScaleX;
                        CGFloat localMaxY =
                            (CGRectGetMaxY(capturedBounds) - CGRectGetMinY(windowFrame)) * pixelScaleY;
                        CGFloat pixelMinX = MAX(0.0, floor(localMinX));
                        CGFloat pixelMinY = MAX(0.0, floor(localMinY));
                        CGRect pixelRect = CGRectMake(
                            pixelMinX,
                            pixelMinY,
                            MIN((CGFloat)CGImageGetWidth(image), ceil(localMaxX)) - pixelMinX,
                            MIN((CGFloat)CGImageGetHeight(image), ceil(localMaxY)) - pixelMinY
                        );
                        if (CGRectIsEmpty(pixelRect)) {
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
                        if (cropped != NULL) CGImageRelease(cropped);
                        if (bounded == NULL) {
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
                            totalBytes + png.length > maxTotalBytes) {
                            [omissions addObject:@{
                                @"attachment_id": attachmentID,
                                @"reason": @"byte_budget",
                                @"detail": @"Encoded PNG exceeded the versioned per-attachment or total media byte budget."
                            }];
                            continue;
                        }
                        totalBytes += png.length;
                        BOOL capturedFullRegion = CGRectEqualToRect(capturedBounds, requestedBounds);
                        [captures addObject:@{
                            @"attachment_id": attachmentID,
                            @"source_bounds": LensFrameDictionary(requestedBounds),
                            @"captured_bounds": LensFrameDictionary(capturedBounds),
                            @"window_bounds": LensFrameDictionary(windowFrame),
                            @"coverage": capturedFullRegion ? @"full_region" : @"visible_subregion",
                            @"mime_type": @"image/png",
                            @"pixel_width": @(pixelWidth),
                            @"pixel_height": @(pixelHeight),
                            @"encoded_bytes": @(png.length),
                            @"data": [png base64EncodedStringWithOptions:0]
                        }];
                    }
                    result = @{
                        @"captures": captures,
                        @"omissions": omissions,
                        @"diagnostics": @[]
                    };
                    dispatch_semaphore_signal(completion);
                }];
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
        CFRelease(application);
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

char *lens_extract_window_json(
    int32_t pid,
    const char *selectedTitleCString,
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
        BOOL hasUsefulContent = text.length > 0 || resourceRefCount > 0;
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

        NSDictionary *result = @{
            @"quality": quality,
            @"resolved_window": @{
                @"title": resolvedTitle,
                @"bounds": LensFrameDictionary(resolvedFrame),
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
        return LensCopyJSONString(result);
    }
}

void lens_free_string(char *value) {
    free(value);
}
