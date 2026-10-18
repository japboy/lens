#import <ApplicationServices/ApplicationServices.h>
#import <Foundation/Foundation.h>

// Only external AX reads are fixtures. Geometry validation, traversal, registered
// extraction and JSON serialization use the production implementation.
static AXError LensTestCopyAttributeValue(AXUIElementRef, CFStringRef, CFTypeRef *);
static AXError LensTestGetAttributeValueCount(AXUIElementRef, CFStringRef, CFIndex *);
static AXError LensTestCopyAttributeValues(AXUIElementRef, CFStringRef, CFIndex, CFIndex, CFArrayRef *);
#define AXUIElementCopyAttributeValue LensTestCopyAttributeValue
#define AXUIElementGetAttributeValueCount LensTestGetAttributeValueCount
#define AXUIElementCopyAttributeValues LensTestCopyAttributeValues
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop
#undef AXUIElementCopyAttributeValue
#undef AXUIElementGetAttributeValueCount
#undef AXUIElementCopyAttributeValues

static AXUIElementRef LensTestRoot;
static AXUIElementRef LensTestChild;
static CGRect LensTestRootFrame;
static CGRect LensTestChildFrame;
static NSUInteger LensTestFailures;
static NSUInteger LensTestAssertions;

static void LensCheck(BOOL condition, const char *message) {
    LensTestAssertions++;
    if (!condition) {
        LensTestFailures++;
        fprintf(stderr, "FAIL: %s\n", message);
    }
}

static AXError LensTestCopyAttributeValue(
    AXUIElementRef element, CFStringRef attribute, CFTypeRef *value
) {
    *value = NULL;
    CGRect frame = element == LensTestRoot ? LensTestRootFrame : LensTestChildFrame;
    if (CFEqual(attribute, kAXPositionAttribute)) {
        *value = AXValueCreate(kAXValueTypeCGPoint, &frame.origin);
    } else if (CFEqual(attribute, kAXSizeAttribute)) {
        *value = AXValueCreate(kAXValueTypeCGSize, &frame.size);
    } else if (CFEqual(attribute, kAXRoleAttribute)) {
        *value = CFRetain(element == LensTestRoot ? kAXWindowRole : kAXStaticTextRole);
    } else if (CFEqual(attribute, kAXTitleAttribute) && element == LensTestRoot) {
        *value = CFBridgingRetain(@"Selected PDF");
    } else if (CFEqual(attribute, kAXValueAttribute) && element == LensTestChild) {
        *value = CFBridgingRetain(@"Document text survives invalid geometry");
    }
    return *value == NULL ? kAXErrorAttributeUnsupported : kAXErrorSuccess;
}

static AXError LensTestGetAttributeValueCount(
    AXUIElementRef element, CFStringRef attribute, CFIndex *count
) {
    *count = element == LensTestRoot && CFEqual(attribute, kAXChildrenAttribute) ? 1 : 0;
    return kAXErrorSuccess;
}

static AXError LensTestCopyAttributeValues(
    AXUIElementRef element, CFStringRef attribute, CFIndex index, CFIndex count, CFArrayRef *values
) {
    if (element == LensTestRoot && CFEqual(attribute, kAXChildrenAttribute) && index == 0 && count == 1) {
        *values = CFBridgingRetain(@[(__bridge id)LensTestChild]);
        return kAXErrorSuccess;
    }
    *values = NULL;
    return kAXErrorNoValue;
}

@interface LensGeometryTestSource : LensNativeWindowSource
@end
@implementation LensGeometryTestSource
- (BOOL)ensureResolvedWindowWithDiagnostics:(NSMutableArray<NSString *> *)diagnostics {
    (void)diagnostics;
    return YES;
}
- (AXUIElementRef)copyResolvedWindow {
    return (AXUIElementRef)CFRetain(LensTestRoot);
}
- (NSString *)pickerApplicationName {
    return @"Fixture";
}
@end

static NSDictionary *LensTestDecode(char *json) {
    LensCheck(json != NULL, "native serialization returns owned JSON");
    if (json == NULL) return nil;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    lens_free_string(json);
    NSDictionary *value = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    LensCheck([value isKindOfClass:NSDictionary.class], "response is valid JSON object");
    return value;
}

static void LensTestSerialization(CGFloat exceptional) {
    @try {
        NSDictionary *result = LensTestDecode(LensCopyJSONString(@{
            @"nested": @[@{@"number": @(exceptional)}]
        }));
        LensCheck([result[@"status"] isEqualToString:@"error"], "invalid JSON returns structured error");
        LensCheck([result[@"message"] length] > 0, "invalid JSON error explains failure");
    } @catch (NSException *exception) {
        fprintf(stderr, "Unexpected serialization exception: %s\n", exception.reason.UTF8String);
        LensCheck(NO, "invalid JSON must not raise Objective-C exception");
    }
}

static void LensTestInvalidFrame(CGFloat exceptional, NSUInteger component) {
    LensTestRootFrame = CGRectMake(-100, -50, 400, 300);
    LensTestChildFrame = CGRectMake(-90, -40, 100, 20);
    switch (component) {
        case 0: LensTestChildFrame.origin.x = exceptional; break;
        case 1: LensTestChildFrame.origin.y = exceptional; break;
        case 2: LensTestChildFrame.size.width = exceptional; break;
        case 3: LensTestChildFrame.size.height = exceptional; break;
    }
    CGRect output = CGRectMake(1, 2, 3, 4);
    LensCheck(!LensAXFrame(LensTestChild, &output), "non-finite AX component rejected");
    LensCheck(CGRectEqualToRect(output, CGRectMake(1, 2, 3, 4)), "rejected AX read leaves output untouched");
    @try {
        NSMutableArray *diagnostics = [NSMutableArray array];
        NSDictionary *extraction = LensExtractResolvedWindow(
            LensTestRoot, @"Selected PDF", LensTestRootFrame, @"Fixture", @"Selected PDF",
            LensTestRootFrame, 200, diagnostics, 10, 4096, 10, 4096, 4096
        );
        NSDictionary *result = LensTestDecode(LensCopyJSONString(extraction));
        NSArray *nodes = result[@"nodes"];
        LensCheck(nodes.count == 2, "invalid child geometry retains node graph");
        LensCheck(nodes.count == 2 && nodes[1][@"bounds"] == nil, "invalid child bounds omitted");
        LensCheck([result[@"text"] containsString:@"Document text survives"], "invalid child geometry retains text");
        LensCheck([result[@"quality"] isEqualToString:@"partial"], "invalid child geometry marks partial extraction");
        LensCheck([diagnostics.description containsString:@"non-finite"], "invalid child geometry diagnosed");
    } @catch (NSException *exception) {
        fprintf(stderr, "Unexpected extraction exception: %s\n", exception.reason.UTF8String);
        LensCheck(NO, "invalid child geometry must serialize without exception");
    }
    LensTestRootFrame = LensTestChildFrame;
    @try {
        NSMutableArray *diagnostics = [NSMutableArray array];
        NSDictionary *result = LensTestDecode(LensCopyJSONString(LensExtractResolvedWindow(
            LensTestRoot, @"Selected PDF", CGRectMake(-100, -50, 400, 300), @"Fixture", @"Selected PDF",
            CGRectZero, 200, diagnostics, 10, 4096, 10, 4096, 4096
        )));
        LensCheck([result[@"quality"] isEqualToString:@"unavailable"], "invalid root rejects legacy resolver placeholder geometry");
        LensCheck([result[@"nodes"] count] == 0, "invalid root traversal publishes no nodes");
        LensCheck([diagnostics.description containsString:@"non-finite"], "invalid root traversal diagnosed");
    } @catch (NSException *exception) {
        fprintf(stderr, "Unexpected root traversal exception: %s\n", exception.reason.UTF8String);
        LensCheck(NO, "invalid root traversal must return without exception");
    }
    @try {
        NSDictionary *result = LensTestDecode(lens_extract_registered_window_json(
            "geometry-regression", 42, 10, 4096, 10, 4096, 4096
        ));
        LensCheck([result[@"quality"] isEqualToString:@"unavailable"], "invalid root geometry returns unavailable");
        LensCheck([result[@"nodes"] count] == 0, "invalid root publishes no guessed bounds");
    } @catch (NSException *exception) {
        fprintf(stderr, "Unexpected root exception: %s\n", exception.reason.UTF8String);
        LensCheck(NO, "invalid root geometry must return without exception");
    }
}

int main(void) {
    @autoreleasepool {
        LensTestRoot = AXUIElementCreateApplication(1);
        LensTestChild = AXUIElementCreateApplication(2);
        LensEnsureWindowRegistries();
        LensWindowSourceRegistry[LensWindowRegistryKey(@"geometry-regression", 42)] = [LensGeometryTestSource new];
        CGFloat exceptional[] = { INFINITY, -INFINITY, NAN };
        for (NSUInteger number = 0; number < 3; number++) {
            LensTestSerialization(exceptional[number]);
            for (NSUInteger component = 0; component < 4; component++) {
                LensTestInvalidFrame(exceptional[number], component);
            }
        }
        LensTestRootFrame = CGRectMake(-100, -50, 400, 300);
        LensTestChildFrame = CGRectMake(-90, -40, 100, 20);
        CGRect frame = CGRectZero;
        LensCheck(LensAXFrame(LensTestRoot, &frame), "finite negative screen coordinates accepted");
        LensCheck(CGRectEqualToRect(frame, LensTestRootFrame), "finite frame preserved exactly");
        NSMutableArray *diagnostics = [NSMutableArray array];
        NSDictionary *result = LensTestDecode(LensCopyJSONString(LensExtractResolvedWindow(
            LensTestRoot, @"Selected PDF", LensTestRootFrame, @"Fixture", @"Selected PDF",
            LensTestRootFrame, 200, diagnostics, 10, 4096, 10, 4096, 4096
        )));
        LensCheck([result[@"quality"] isEqualToString:@"full"], "finite graph retains full extraction");
        LensCheck([result[@"nodes"][1][@"bounds"] isEqual:LensFrameDictionary(LensTestChildFrame)], "finite child bounds preserved");
        LensCheck(diagnostics.count == 0, "finite graph gains no invalid-geometry diagnostics");
        LensTestChildFrame = CGRectMake(-90, -40, 0, 0);
        LensCheck(LensAXFrame(LensTestChild, &frame), "finite zero-size node geometry remains available");
        LensCheck(CGRectEqualToRect(frame, LensTestChildFrame), "finite zero-size geometry preserved exactly");
        [LensWindowSourceRegistry removeAllObjects];
        CFRelease(LensTestChild);
        CFRelease(LensTestRoot);
        printf("Native geometry/JSON: %lu assertions, %lu failures\n", (unsigned long)LensTestAssertions, (unsigned long)LensTestFailures);
        return LensTestFailures == 0 ? 0 : 1;
    }
}
