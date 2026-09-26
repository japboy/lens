#import <ApplicationServices/ApplicationServices.h>
#import <Foundation/Foundation.h>

// Replace only the external AX read. The production resolver, type checks,
// geometry scoring, retained result and diagnostics run unchanged, without TCC
// grants, window enumeration or interaction with a running application.
static AXError LensTestCopyAttributeValue(
    AXUIElementRef element, CFStringRef attribute, CFTypeRef *value
);
#define AXUIElementCopyAttributeValue LensTestCopyAttributeValue
#pragma clang diagnostic push
// Including an implementation makes Clang apply header-only nullability checks.
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop
#undef AXUIElementCopyAttributeValue

static AXUIElementRef LensTestApplication;
static AXUIElementRef LensTestWindow;
static NSString *LensTestTitle;
static CGRect LensTestFrame;

static AXError LensTestCopyAttributeValue(
    AXUIElementRef element, CFStringRef attribute, CFTypeRef *value
) {
    *value = NULL;
    if (element == LensTestApplication) {
        if (CFEqual(attribute, kAXRoleAttribute)) {
            *value = CFRetain(kAXApplicationRole);
        } else if (CFEqual(attribute, kAXWindowsAttribute)) {
            *value = CFBridgingRetain(@[(__bridge id)LensTestWindow]);
        }
    } else if (element == LensTestWindow) {
        if (CFEqual(attribute, kAXTitleAttribute)) {
            *value = CFBridgingRetain(LensTestTitle);
        } else if (CFEqual(attribute, kAXPositionAttribute)) {
            *value = AXValueCreate(kAXValueTypeCGPoint, &LensTestFrame.origin);
        } else if (CFEqual(attribute, kAXSizeAttribute)) {
            *value = AXValueCreate(kAXValueTypeCGSize, &LensTestFrame.size);
        }
    }
    return *value == NULL ? kAXErrorAttributeUnsupported : kAXErrorSuccess;
}

static BOOL LensTestSingleton(BOOL matching) {
    CGRect selectedFrame = CGRectMake(10, 20, 400, 300);
    LensTestTitle = matching ? @"Selected document" : @"Unselected private notes";
    LensTestFrame = matching ? selectedFrame : CGRectMake(1000, 1200, 200, 150);
    NSString *title = @"unchanged";
    CGRect frame = CGRectZero;
    double score = -1.0;
    NSMutableArray<NSString *> *diagnostics = [NSMutableArray array];
    AXUIElementRef resolved = LensCopyResolvedAXWindowForApplication(
        LensTestApplication, @"Selected document", selectedFrame,
        &title, &frame, &score, diagnostics
    );
    BOOL passed;
    if (matching) {
        passed = resolved == LensTestWindow
            && [title isEqualToString:@"Selected document"]
            && CGRectEqualToRect(frame, selectedFrame)
            && score == 200.0 && diagnostics.count == 0;
    } else {
        passed = resolved == NULL && [title isEqualToString:@"unchanged"]
            && CGRectEqualToRect(frame, CGRectZero) && score == -1.0
            && diagnostics.count == 1
            && [diagnostics.firstObject containsString:@"found 1 AXWindows"]
            && [diagnostics.firstObject containsString:@"best score 0.0"];
    }
    if (resolved != NULL) CFRelease(resolved);
    if (!passed) {
        fprintf(stderr, "%s singleton resolver regression failed: %s\n",
            matching ? "Matching" : "Mismatched", diagnostics.description.UTF8String);
    }
    return passed;
}

int main(void) {
    @autoreleasepool {
        // Construction supplies genuine CF AX types; all attribute reads above
        // are local fixtures, and these objects never send a process message.
        LensTestApplication = AXUIElementCreateApplication(1);
        LensTestWindow = AXUIElementCreateApplication(2);
        BOOL rejected = LensTestSingleton(NO);
        BOOL matched = LensTestSingleton(YES);
        CFRelease(LensTestWindow);
        CFRelease(LensTestApplication);
        if (!rejected || !matched) return 1;
        puts("Native window resolver: 2 tests passed");
        return 0;
    }
}
