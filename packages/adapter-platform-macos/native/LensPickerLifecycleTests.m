#import <objc/runtime.h>
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop

// Substitute only the OS singleton. Production coordinator, callback ownership,
// operation release and observer handlers run without presenting system UI.
@interface LensTestPicker : NSObject
@property(nonatomic) BOOL active;
@property(nonatomic, strong) SCContentSharingPickerConfiguration *defaultConfiguration;
@property(nonatomic, strong) NSNumber *maximumStreamCount;
@property(nonatomic) NSUInteger presentations;
@property(nonatomic, strong) NSMutableSet *observers;
@end
@implementation LensTestPicker
- (void)addObserver:(id)observer { [self.observers addObject:observer]; }
- (void)removeObserver:(id)observer { [self.observers removeObject:observer]; }
- (void)presentPickerUsingContentStyle:(SCShareableContentStyle)style {
    NSCAssert(style == SCShareableContentStyleWindow, @"single-window style");
    self.presentations += 1;
}
@end

@interface LensTestRetainedSource : LensNativeWindowSource
@property(nonatomic) NSUInteger releases;
@end
@implementation LensTestRetainedSource
- (void)releaseAllRetainedObjects { self.releases += 1; }
@end

static LensTestPicker *TestPicker;
static id TestSharedPicker(id receiver, SEL selector) {
    (void)receiver;
    (void)selector;
    return TestPicker;
}
typedef struct { NSUInteger calls; __unsafe_unretained NSString *expected; } TestReply;
static void ReceiveReply(const char *json, void *context) {
    TestReply *reply = context;
    reply->calls += 1;
    NSDictionary *payload = [NSJSONSerialization JSONObjectWithData:
        [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding]
        options:0 error:NULL];
    NSCAssert([payload[@"status"] isEqualToString:reply->expected], @"terminal status");
}

int main(void) {
    @autoreleasepool {
        TestPicker = [LensTestPicker new];
        TestPicker.observers = [NSMutableSet set];
        Method singleton = class_getClassMethod(SCContentSharingPicker.class, @selector(sharedPicker));
        IMP original = method_setImplementation(singleton, (IMP)TestSharedPicker);
        const char *operation = "11111111-1111-1111-1111-111111111111";
        const char *other = "22222222-2222-2222-2222-222222222222";

        TestReply cancelled = {0, @"cancelled"};
        NSCAssert(lens_present_window_picker_for_operation(operation, ReceiveReply, &cancelled), @"present");
        LensContentPickerCoordinator *old = LensActiveContentPickerCoordinator;
        NSCAssert(!lens_cancel_window_picker_for_operation(other), @"other operation is untouched");
        NSCAssert(cancelled.calls == 0 && TestPicker.active, @"still pending");
        NSCAssert(lens_release_window_operation(operation), @"release cancels without selected sources");
        NSCAssert(cancelled.calls == 1 && !TestPicker.active && TestPicker.observers.count == 0, @"fully detached");
        NSCAssert(LensActiveContentPickerCoordinator == nil, @"reentry available");

        // A new invocation with the same operation is protected from all old terminals.
        TestReply next = {0, @"cancelled"};
        NSCAssert(lens_present_window_picker_for_operation(operation, ReceiveReply, &next), @"present again");
        [old contentSharingPicker:(id)TestPicker didUpdateWithFilter:(id)[NSObject new] forStream:nil];
        [old contentSharingPicker:(id)TestPicker didCancelForStream:nil];
        [old contentSharingPickerStartDidFailWithError:[NSError errorWithDomain:@"test" code:1 userInfo:nil]];
        [old deliver:@{ @"status": @"cancelled" }];
        NSCAssert(cancelled.calls == 1 && next.calls == 0 && TestPicker.active, @"stale terminal cannot finish replacement");
        NSCAssert(TestPicker.observers.count == 1, @"replacement observer remains");

        LensEnsureWindowRegistries();
        LensTestRetainedSource *source = [LensTestRetainedSource new];
        source.operationID = [NSString stringWithUTF8String:operation];
        LensWindowSourceRegistry[@"fixture"] = source;
        NSCAssert(lens_cancel_window_picker_for_operation(operation), @"cancel pending Add");
        NSCAssert(next.calls == 1 && source.releases == 0 && LensWindowSourceRegistry.count == 1, @"reviewed targets preserved");
        NSCAssert(!lens_cancel_window_picker_for_operation(operation), @"second cancel is a no-op");
        NSCAssert(lens_release_window_operation(operation) && source.releases == 1, @"full release frees target");

        TestReply failed = {0, @"error"};
        NSCAssert(lens_present_window_picker_for_operation(other, ReceiveReply, &failed), @"present before start error");
        [LensActiveContentPickerCoordinator contentSharingPickerStartDidFailWithError:
            [NSError errorWithDomain:@"test" code:2 userInfo:nil]];
        NSCAssert(failed.calls == 1 && LensActiveContentPickerCoordinator == nil && !TestPicker.active, @"start error releases ownership");
        method_setImplementation(singleton, original);
        puts("Picker lifecycle: cancellation, same-operation reentry, stale callbacks, retained Add targets and start failure passed");
    }
    return 0;
}
