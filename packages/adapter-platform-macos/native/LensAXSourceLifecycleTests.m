#import <ApplicationServices/ApplicationServices.h>
#import <Foundation/Foundation.h>

// Only AX attribute reads are replaced. Registration, identity isolation,
// current-frame state and release use the production metadata-only AX registry.
static AXError LensTestCopyAttributeValue(AXUIElementRef element, CFStringRef attribute, CFTypeRef *value);
#define AXUIElementCopyAttributeValue LensTestCopyAttributeValue
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop
#undef AXUIElementCopyAttributeValue

static AXUIElementRef TestWindow;
static CGRect TestFrame;
static BOOL TestFrameAvailable;

static AXError LensTestCopyAttributeValue(AXUIElementRef element, CFStringRef attribute, CFTypeRef *value) {
    *value = NULL;
    if (element != TestWindow || !TestFrameAvailable) return kAXErrorCannotComplete;
    if (CFEqual(attribute, kAXPositionAttribute)) *value = AXValueCreate(kAXValueTypeCGPoint, &TestFrame.origin);
    if (CFEqual(attribute, kAXSizeAttribute)) *value = AXValueCreate(kAXValueTypeCGSize, &TestFrame.size);
    return *value ? kAXErrorSuccess : kAXErrorAttributeUnsupported;
}

@interface LensTestPromotedSource : LensNativeWindowSource
@end
@implementation LensTestPromotedSource
- (AXUIElementRef)copyResolvedWindow { return (AXUIElementRef)CFRetain(TestWindow); }
@end

int main(void) {
    @autoreleasepool {
        const char *operation = "11111111-1111-1111-1111-111111111111";
        const char *other = "22222222-2222-2222-2222-222222222222";
        double x = 901, y = 902, width = 903, height = 904;
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == -1, @"missing identity fails");
        NSCAssert(!lens_register_ax_window_for_operation(operation, 0, 1, "", "", 0, 0, 10, 10), @"zero window rejected");
        NSCAssert(!lens_register_ax_window_for_operation(operation, 7, 0, "", "", 0, 0, 10, 10), @"invalid pid rejected");
        NSCAssert(!lens_register_ax_window_for_operation(NULL, 7, 1, "", "", 0, 0, 10, 10), @"null operation rejected");
        for (NSUInteger index = 0; index < 4; index++) {
            double fields[] = {0, 0, 10, 10};
            fields[index] = NAN;
            NSCAssert(!lens_register_ax_window_for_operation(operation, 7, 1, "", "", fields[0], fields[1], fields[2], fields[3]), @"NaN metadata rejected");
            fields[index] = INFINITY;
            NSCAssert(!lens_register_ax_window_for_operation(operation, 7, 1, "", "", fields[0], fields[1], fields[2], fields[3]), @"infinite metadata rejected");
        }
        NSCAssert(!lens_register_ax_window_for_operation(operation, 7, 1, "", "", DBL_MAX, 0, DBL_MAX, 10), @"overflowed bounds rejected");
        NSCAssert(lens_register_ax_window_for_operation(operation, 7, 1, "Chosen", "Fixture", -10, 20, 300, 200), @"metadata registration");
        NSCAssert(!lens_register_ax_window_for_operation(operation, 7, 2, "Other", "Other", 0, 0, 100, 100), @"existing authority cannot be rebound");
        LensNativeWindowSource *stored = LensWindowSourceForIdentity([NSString stringWithUTF8String:operation], 7);
        NSCAssert(stored.pid == 1 && [stored.pickerTitle isEqualToString:@"Chosen"], @"AX owns metadata only; safe Rust owns SCWindow");
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == 0, @"never-promoted allows screenshot fallback without AX");
        NSCAssert(x == 901 && y == 902 && width == 903 && height == 904, @"no uninitialized output published");
        NSCAssert(lens_register_ax_window_for_operation(other, 7, 2, "Other", "Fixture", 0, 0, 100, 100), @"same numeric ID isolated across operations");

        TestWindow = AXUIElementCreateApplication(1);
        LensTestPromotedSource *promoted = [LensTestPromotedSource new];
        promoted.operationID = [NSString stringWithUTF8String:operation];
        promoted.windowID = 7;
        promoted.pid = 1;
        LensWindowSourceRegistry[LensWindowRegistryKey(promoted.operationID, 7)] = promoted;
        TestFrame = CGRectMake(-20.5, 40, 150, 90);
        TestFrameAvailable = YES;
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == 1, @"promoted current frame");
        NSCAssert(x == -20.5 && y == 40 && width == 150 && height == 90, @"exact promoted frame");
        TestFrameAvailable = NO;
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == -1, @"promoted read failure cannot fall back");
        TestFrameAvailable = YES;
        TestFrame.origin.x = NAN;
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == -1, @"nonfinite promoted frame rejected");
        TestFrame = CGRectMake(0, 0, 0, 100);
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == -1, @"empty promoted frame rejected");
        NSCAssert(lens_release_registered_window(operation, 7), @"explicit release");
        NSCAssert(!lens_release_registered_window(operation, 7), @"double release is absent");
        NSCAssert(lens_registered_ax_window_frame(operation, 7, &x, &y, &width, &height) == -1, @"released authority is unavailable");
        NSCAssert(lens_registered_ax_window_frame(other, 7, &x, &y, &width, &height) == 0, @"other operation remains intact");
        NSCAssert(lens_release_window_operation(other), @"operation release");
        NSCAssert(!lens_release_window_operation(other), @"empty operation is absent");
        // Exercise the real stop receipt comparison without registering an OS observer.
        // Public operation/source IDs and epochs can be reused; live refcons differ.
        int oldContext = 1, replacementContext = 2;
        NSString *sourceID = @"33333333-3333-3333-3333-333333333333";
        NSString *observationKey = LensObservationRegistryKey(promoted.operationID, sourceID);
        LensNativeWindowSource *replacement = [LensNativeWindowSource new];
        replacement.operationID = promoted.operationID;
        replacement.sourceRegistrationID = sourceID;
        replacement.observerEpoch = 1;
        replacement.observationContext = &replacementContext;
        LensObservationSourceRegistry[observationKey] = replacement;
        NSCAssert(!lens_stop_window_observation(operation, sourceID.UTF8String, &oldContext), @"stale receipt cannot stop replacement with reused public identity");
        NSCAssert(LensObservationSourceRegistry[observationKey] == replacement && replacement.observationContext == &replacementContext, @"stale stop leaves current owner and refcon untouched");
        NSCAssert(!lens_stop_window_observation(operation, sourceID.UTF8String, NULL), @"null receipt rejected");
        NSCAssert(lens_stop_window_observation(operation, sourceID.UTF8String, &replacementContext), @"matching live receipt stops current owner");
        NSCAssert(LensObservationSourceRegistry[observationKey] == nil && replacement.observationContext == NULL, @"matching stop removes registry and refcon");
        NSCAssert(!lens_stop_window_observation(operation, sourceID.UTF8String, &replacementContext), @"repeated receipt stop is absent");
        CFRelease(TestWindow);
        puts("Native AX metadata lifecycle: finite registration, immutable authority, current-frame states and isolated release passed");
        return 0;
    }
}
