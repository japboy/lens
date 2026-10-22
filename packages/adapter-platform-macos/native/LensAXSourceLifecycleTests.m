#import <ApplicationServices/ApplicationServices.h>
#import <Foundation/Foundation.h>

// Registration, identity isolation and release use the production AX registry.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop

int main(void) {
    @autoreleasepool {
        const char *operation = "11111111-1111-1111-1111-111111111111";
        const char *other = "22222222-2222-2222-2222-222222222222";
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
        NSCAssert(lens_register_ax_window_for_operation(other, 7, 2, "Other", "Fixture", 0, 0, 100, 100), @"same numeric ID isolated across operations");

        NSCAssert(lens_release_registered_window(operation, 7), @"explicit release");
        NSCAssert(!lens_release_registered_window(operation, 7), @"double release is absent");
        NSCAssert(LensWindowSourceForIdentity([NSString stringWithUTF8String:operation], 7) == nil, @"released authority is unavailable");
        NSCAssert(LensWindowSourceForIdentity([NSString stringWithUTF8String:other], 7) != nil, @"other operation remains intact");
        NSCAssert(lens_release_window_operation(other), @"operation release");
        NSCAssert(!lens_release_window_operation(other), @"empty operation is absent");
        // Exercise the real stop receipt comparison without registering an OS observer.
        // Public operation/source IDs and epochs can be reused; live refcons differ.
        int oldContext = 1, replacementContext = 2;
        NSString *sourceID = @"33333333-3333-3333-3333-333333333333";
        NSString *operationID = [NSString stringWithUTF8String:operation];
        NSString *observationKey = LensObservationRegistryKey(operationID, sourceID);
        LensNativeWindowSource *replacement = [LensNativeWindowSource new];
        replacement.operationID = operationID;
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
        puts("Native AX metadata lifecycle: finite registration, immutable authority and isolated release passed");
        return 0;
    }
}
