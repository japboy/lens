#import <Foundation/Foundation.h>
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop

static void CheckPrefix(NSString *text, NSUInteger budget, NSString *expected) {
    NSMutableArray<NSString *> *fragments = [NSMutableArray array];
    NSMutableSet<NSString *> *seen = [NSMutableSet set];
    NSUInteger bytes = 0;
    BOOL truncated = NO;
    LensAppendTextFragment(fragments, seen, text, budget, &bytes, &truncated);
    NSString *actual = [fragments componentsJoinedByString:@""];
    NSData *encoded = [actual dataUsingEncoding:NSUTF8StringEncoding];
    NSCAssert(encoded != nil, @"invalid UTF-8: %@, budget %lu", text, (unsigned long)budget);
    NSCAssert([actual isEqualToString:expected], @"unexpected prefix: %@, budget %lu", actual, (unsigned long)budget);
    NSCAssert(bytes == encoded.length && bytes <= budget, @"incorrect accounting");
    NSCAssert(truncated == ([text lengthOfBytesUsingEncoding:NSUTF8StringEncoding] > budget), @"incorrect truncation flag");
}

int main(void) {
    @autoreleasepool {
        NSArray<NSString *> *emoji = @[@"", @"", @"", @"", @"😀", @"😀a"];
        NSArray<NSString *> *ascii = @[@"", @"a", @"ab", @"abc", @"abc", @"abc"];
        NSArray<NSString *> *japanese = @[@"", @"", @"", @"\u65e5", @"\u65e5", @"\u65e5"];
        NSArray<NSString *> *mixed = @[@"", @"a", @"a", @"a", @"a\u65e5", @"a\u65e5"];
        for (NSUInteger budget = 0; budget <= 5; budget++) {
            CheckPrefix(@"😀a", budget, emoji[budget]);
            CheckPrefix(@"abc", budget, ascii[budget]);
            CheckPrefix(@"\u65e5\u672c", budget, japanese[budget]);
            CheckPrefix(@"a\u65e5😀b", budget, mixed[budget]);
        }
        CheckPrefix(@"\u65e5\u672c", 6, @"\u65e5\u672c");
        CheckPrefix(@"a\u65e5😀b", 8, @"a\u65e5😀");
        CheckPrefix(@"a\u65e5😀b", 9, @"a\u65e5😀b");
        // Encodable prefixes may split a combining sequence; no grapheme promise.
        CheckPrefix(@"e\u0301x", 1, @"e");
        NSMutableArray<NSString *> *fragments = [NSMutableArray array];
        NSMutableSet<NSString *> *seen = [NSMutableSet set];
        NSUInteger bytes = 0;
        BOOL truncated = NO;
        LensAppendTextFragment(fragments, seen, @"  a  ", 6, &bytes, &truncated);
        LensAppendTextFragment(fragments, seen, @"a", 6, &bytes, &truncated);
        LensAppendTextFragment(fragments, seen, @"\n", 6, &bytes, &truncated);
        NSCAssert(bytes == 1 && fragments.count == 1 && seen.count == 1 && !truncated, @"deduplication changed");
        NSString *canonical = @"😀ab";
        LensAppendTextFragment(fragments, seen, canonical, 6, &bytes, &truncated);
        NSCAssert(bytes == 6 && truncated && [fragments.lastObject isEqualToString:@"😀a"], @"accumulated budget changed");
        LensAppendTextFragment(fragments, seen, @"later", 6, &bytes, &truncated);
        NSCAssert(bytes == 6 && fragments.count == 2 && seen.count == 1, @"truncation must stop later fragments");
        NSCAssert([canonical isEqualToString:@"😀ab"], @"source text must remain intact");
        puts("Native diagnostic text: 28 prefix boundaries and accumulation checks passed");
    }
    return 0;
}
