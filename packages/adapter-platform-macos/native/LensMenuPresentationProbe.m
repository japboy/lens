#import <AppKit/AppKit.h>
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "LensNative.m"
#pragma clang diagnostic pop

static void Require(BOOL value, NSString *message) {
    if (!value) {
        NSLog(@"FAIL: %@", message);
        exit(1);
    }
}

static NSString *FixturePNG(void) {
    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc]
        initWithBitmapDataPlanes:NULL pixelsWide:2 pixelsHigh:1 bitsPerSample:8
        samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace
        bytesPerRow:8 bitsPerPixel:32];
    memset(bitmap.bitmapData, 255, 8);
    return [[bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}]
        base64EncodedStringWithOptions:0];
}

static NSDictionary *Row(NSString *title, id tooltip, id image, id children) {
    return @{@"title": title, @"tooltip": tooltip, @"icon": image, @"children": children};
}

static BOOL Apply(NSMenu *root, NSUInteger index, NSString *title, NSArray *rows) {
    NSData *json = [NSJSONSerialization dataWithJSONObject:rows options:0 error:NULL];
    NSString *text = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    return lens_set_menu_presentation((__bridge void *)root, index, title.UTF8String, text.UTF8String);
}

int main(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        // Deliberately never create an NSStatusItem or attach this root to a tray.
        NSMenu *root = [[NSMenu alloc] initWithTitle:@"Root"];
        NSMenuItem *parent = [[NSMenuItem alloc] initWithTitle:@"Agents" action:NULL keyEquivalent:@""];
        NSMenu *submenu = [[NSMenu alloc] initWithTitle:@"Agents"];
        parent.submenu = submenu;
        [root addItem:parent];
        NSMenuItem *first = [[NSMenuItem alloc] initWithTitle:@"Duplicate" action:NULL keyEquivalent:@""];
        NSMenuItem *second = [[NSMenuItem alloc] initWithTitle:@"Duplicate" action:NULL keyEquivalent:@""];
        first.state = NSControlStateValueOn;
        first.enabled = NO;
        [submenu addItem:first];
        [submenu addItem:second];
        NSString *png = FixturePNG();
        NSArray *rows = @[Row(@"Duplicate", @"First", png, NSNull.null),
                          Row(@"Duplicate", @"Second", NSNull.null, NSNull.null)];
        Require(Apply(root, 0, @"Agents", rows), @"unattached root supports native presentation");
        Require([first.toolTip isEqualToString:@"First"] && [second.toolTip isEqualToString:@"Second"], @"duplicate title identity remains positional");
        Require(first.image.template && first.image.size.height == 18 && first.image.size.width == 36, @"PNG becomes aspect-preserving template image");
        Require(first.state == NSControlStateValueOn && !first.enabled, @"selection and enabled state are unchanged");
        NSImage *originalImage = first.image;
        NSArray *badRows = @[Row(@"Duplicate", @"Must not apply", NSNull.null, NSNull.null),
                             Row(@"Duplicate", @"Second", @"invalid PNG", NSNull.null)];
        Require(!Apply(root, 0, @"Agents", badRows), @"invalid image is rejected");
        Require(first.image == originalImage && [first.toolTip isEqualToString:@"First"], @"validation failure leaves all earlier rows unchanged");
        Require(!Apply(root, 1, @"Agents", rows), @"invalid root index is rejected");
        Require(!Apply(root, 0, @"Wrong", rows), @"wrong parent title is rejected");
        Require(!Apply(root, 0, @"Agents", @[rows[0]]), @"wrong row count is rejected");
        Require(!Apply(root, 0, @"Agents", @[Row(@"Wrong", NSNull.null, NSNull.null, NSNull.null), rows[1]]), @"wrong child title is rejected");

        NSMenu *nested = [[NSMenu alloc] initWithTitle:@"Nested"];
        NSMenuItem *leaf = [[NSMenuItem alloc] initWithTitle:@"Leaf" action:NULL keyEquivalent:@""];
        [nested addItem:leaf];
        second.submenu = nested;
        NSArray *nestedRows = @[Row(@"Duplicate", NSNull.null, NSNull.null, NSNull.null),
            Row(@"Duplicate", @"Group", NSNull.null, @[Row(@"Leaf", @"Leaf help", png, NSNull.null)])];
        Require(Apply(root, 0, @"Agents", nestedRows), @"nested item appearance is applied");
        Require(!first.image && !first.toolTip && leaf.image.template && [leaf.toolTip isEqualToString:@"Leaf help"], @"clearing and child metadata are applied");
        Require(!Apply(root, 0, @"Agents", rows), @"missing declared child tree is rejected");
        Require(!lens_set_menu_presentation(NULL, 0, "Agents", "[]"), @"null root is rejected");
        puts("PASS: unattached root, duplicate identity, template images, nested metadata and atomic rejection");
    }
    return 0;
}
