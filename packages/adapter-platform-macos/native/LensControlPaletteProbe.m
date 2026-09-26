// Standalone regression probe linked to production LensControlPalette.m.
// clang -fobjc-arc -framework AppKit -framework WebKit LensControlPalette{,Probe}.m -o /tmp/lens-palette-probe
#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>
#include <math.h>
#include <string.h>
#import "LensNative.h"

static void Require(BOOL condition, NSString *message) {
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", message.UTF8String);
        exit(1);
    }
}
// Independent WCAG calculation checks the serialized, quantized pair.
static double RelativeLuminance(const uint8_t *rgb) {
    double weights[3] = {0.2126, 0.7152, 0.0722}, result = 0;
    for (size_t i = 0; i < 3; i++) {
        double component = rgb[i] / 255.0;
        result += weights[i] * (component <= 0.04045 ? component / 12.92 : pow((component + 0.055) / 1.055, 2.4));
    }
    return result;
}
static double PairContrast(const uint8_t *fill, const uint8_t *foreground) {
    double a = RelativeLuminance(fill), b = RelativeLuminance(foreground);
    return (fmax(a, b) + 0.05) / (fmin(a, b) + 0.05);
}
static void CheckPrimary(const uint8_t *accent, bool increased) {
    double rgb[3] = {accent[0] / 255.0, accent[1] / 255.0, accent[2] / 255.0};
    uint8_t fill[4], foreground[4], repeatedFill[4], repeatedForeground[4];
    Require(lens_primary_button_colors(rgb, increased, fill, foreground), @"valid primary resolves");
    Require(fill[3] == 255 && foreground[3] == 255, @"primary pair is opaque");
    bool white = RelativeLuminance(accent) <= 0.5;
    Require(foreground[0] == (white ? 255 : 0) && foreground[0] == foreground[1] && foreground[1] == foreground[2],
        @"native-like foreground selection is independent of contrast adjustment");
    Require(PairContrast(fill, foreground) >= (increased ? 7.0 : 4.5), @"quantized primary meets its contrast floor");
    Require(lens_primary_button_colors(rgb, increased, repeatedFill, repeatedForeground)
        && memcmp(fill, repeatedFill, 4) == 0 && memcmp(foreground, repeatedForeground, 4) == 0,
        @"primary policy is deterministic");
    if (PairContrast(accent, foreground) >= (increased ? 7.0 : 4.5))
        Require(memcmp(fill, accent, 3) == 0, @"sufficient native accent remains unchanged");
}
static void CheckPrimaryPolicy(void) {
    // Synthetic color-family fixtures, not claims that every OS accent preference was toggled.
    const uint8_t accents[][3] = {
        {0, 122, 255}, {255, 45, 85}, {52, 199, 89}, {255, 204, 0},
        {255, 149, 0}, {175, 82, 222}, {128, 128, 128}, {0, 0, 0}, {255, 255, 255},
        {118, 118, 118}, {119, 119, 119}, {187, 187, 187}, {188, 188, 188},
    };
    for (size_t i = 0; i < sizeof(accents) / sizeof(accents[0]); i++)
        for (int increased = 0; increased < 2; increased++) CheckPrimary(accents[i], increased);
    // A bounded cube samples dark, saturated, mixed and near-boundary colors.
    for (unsigned r = 0; r <= 255; r += 17)
        for (unsigned g = 0; g <= 255; g += 17)
            for (unsigned b = 0; b <= 255; b += 17) {
                uint8_t rgb[3] = {r, g, b};
                CheckPrimary(rgb, false);
                CheckPrimary(rgb, true);
            }
    for (unsigned gray = 0; gray <= 255; gray++) {
        uint8_t rgb[3] = {gray, gray, gray};
        CheckPrimary(rgb, false); CheckPrimary(rgb, true);
    }
    double rounding[3] = {0.5, 0.5, 0.5};
    uint8_t fill[4], foreground[4];
    Require(lens_primary_button_colors(rounding, false, fill, foreground) && fill[0] == 118,
        @"rounding chooses passing gray118 rather than failing gray119");
    double invalid[3] = {NAN, 0, 0};
    Require(!lens_primary_button_colors(invalid, false, fill, foreground), @"nonfinite accent is rejected");
    invalid[0] = 1.1;
    Require(!lens_primary_button_colors(invalid, false, fill, foreground), @"out-of-range accent is rejected");
    Require(!lens_primary_button_colors(NULL, false, fill, foreground), @"null accent is rejected");
    Require(!lens_primary_button_colors(rounding, false, NULL, foreground), @"null primary output is rejected");
}

static void PumpUntil(BOOL (^ready)(void)) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:15];
    while (!ready() && deadline.timeIntervalSinceNow > 0)
        [NSRunLoop.currentRunLoop runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    Require(ready(), @"bounded WebKit operation timed out");
}
static id Evaluate(WKWebView *webview, NSString *script) {
    __block BOOL done = NO;
    __block id result = nil;
    [webview evaluateJavaScript:script completionHandler:^(id value, NSError *error) {
        Require(error == nil, error.localizedDescription ?: @"JavaScript failed");
        result = value;
        done = YES;
    }];
    PumpUntil(^BOOL { return done; });
    return result;
}
static NSDictionary *Palette(WKWebView *webview) {
    id value = Evaluate(webview, @"window.__LENS_CONTROL_PALETTE__");
    Require([value isKindOfClass:NSDictionary.class], @"complete native palette is present");
    NSDictionary *colors = value[@"colors"];
    Require([colors isKindOfClass:NSDictionary.class] && colors.count == 7, @"all semantic colors resolve atomically");
    Require([colors[@"control_surface"][3] intValue] == 255 && [colors[@"window_surface"][3] intValue] == 255,
        @"reference surfaces are opaque");
    for (NSString *key in @[@"button_fill", @"button_pressed_fill", @"separator"])
        Require([colors[key][3] intValue] > 0 && [colors[key][3] intValue] < 255, @"semantic fill keeps native alpha");
    uint8_t primaryFill[4], primaryForeground[4];
    for (size_t i = 0; i < 4; i++) {
        primaryFill[i] = [colors[@"primary_button_fill"][i] unsignedCharValue];
        primaryForeground[i] = [colors[@"primary_button_foreground"][i] unsignedCharValue];
    }
    Require(primaryFill[3] == 255 && primaryForeground[3] == 255, @"primary colors remain opaque over floating surfaces");
    Require(PairContrast(primaryFill, primaryForeground) >= ([value[@"increase_contrast"] boolValue] ? 7.0 : 4.5),
        @"actual native accent pair meets Lens contrast policy");
    Require([value count] == 4, @"palette contains colors and three independent state flags");
    Require([value[@"window_active"] boolValue] == (webview.window.isKeyWindow && NSApp.isActive),
        @"activity matches actual native window and application");
    Require([value[@"increase_contrast"] boolValue] == NSWorkspace.sharedWorkspace.accessibilityDisplayShouldIncreaseContrast,
        @"contrast preference matches real NSWorkspace");
    Require([value[@"reduce_transparency"] boolValue] == NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceTransparency,
        @"transparency preference matches real NSWorkspace");
    return value;
}
@interface PaletteNavigation : NSObject <WKNavigationDelegate>
@property(nonatomic) BOOL finished;
@end
@implementation PaletteNavigation
- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation { self.finished = YES; }
@end

int main(void) {
    @autoreleasepool {
        CheckPrimaryPolicy();
        fprintf(stdout, "PASS: primary contrast policy (13 fixtures, 4096 RGB samples, 256 grays at 4.5/7 floors), rounding, determinism and invalid inputs\n");
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
        LensControlPalette value;
        Require(!lens_control_palette(NULL, NULL), @"null output is rejected");
        Require(!lens_observe_control_palette(NULL, NULL), @"null handles are rejected");
        Require(lens_control_palette(NULL, &value), @"application palette resolves");

        NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 500, 400)
            styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
        window.releasedWhenClosed = NO;
        window.opaque = NO;
        window.backgroundColor = NSColor.clearColor;
        window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameAqua];
        WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
        WKUserContentController *controller = configuration.userContentController;
        WKUserScript *seed = [[WKUserScript alloc] initWithSource:@"window.__LENS_CONTROL_PALETTE__={stale:true}"
            injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES];
        WKUserScript *unrelated = [[WKUserScript alloc] initWithSource:@"window.unrelated=42"
            injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:NO
            inContentWorld:[WKContentWorld worldWithName:@"unrelated"]];
        [controller addUserScript:seed];
        [controller addUserScript:unrelated];
        WKWebView *webview = [[WKWebView alloc] initWithFrame:window.contentView.bounds configuration:configuration];
        [window.contentView addSubview:webview];
        Require(!lens_configure_floating_window_radius(NULL, 12), @"null shape target is rejected");
        Require(!lens_configure_floating_window_radius((__bridge void *)window, NAN), @"nonfinite radius is rejected");
        Require(!lens_configure_floating_window_radius((__bridge void *)window, 65), @"oversized radius is rejected");
        Require(lens_configure_floating_window_radius((__bridge void *)window, 12), @"floating parent clipping installs");
        NSView *contentParent = window.contentView;
        Require(contentParent.layer.cornerRadius == 12 && contentParent.layer.masksToBounds,
            @"parent clips WebView content at the constructor radius");
        Require(webview.superview == contentParent, @"web content belongs to the clipped parent");
        for (NSValue *size in @[[NSValue valueWithSize:NSMakeSize(360, 320)],
                               [NSValue valueWithSize:NSMakeSize(900, 700)],
                               [NSValue valueWithSize:NSMakeSize(500, 400)]]) {
            [window setContentSize:size.sizeValue];
            [contentParent layoutSubtreeIfNeeded];
            Require(NSEqualSizes(contentParent.bounds.size, size.sizeValue),
                @"native content boundary follows resize");
            Require(contentParent.layer.cornerRadius == 12 && contentParent.layer.masksToBounds,
                @"resize retains the single parent shape boundary");
        }
        fprintf(stdout, "PASS: native floating parent clip, invalid geometry and small/large resize\\n");
        webview.underPageBackgroundColor = NSColor.clearColor;
        PaletteNavigation *navigation = [PaletteNavigation new];
        webview.navigationDelegate = navigation;

        Require(lens_observe_control_palette((__bridge void *)window, (__bridge void *)webview), @"observer attaches");
        Require(controller.userScripts.count == 3, @"one script added");
        Require(lens_observe_control_palette((__bridge void *)window, (__bridge void *)webview), @"observer replacement attaches");
        Require(controller.userScripts.count == 3, @"replacement releases previous owned script");
        Require(controller.userScripts[0] == seed && controller.userScripts[1] == unrelated,
            @"original scripts and content worlds preserved by identity");
        NSString *html = @"<script>window.firstPalette=window.__LENS_CONTROL_PALETTE__;window.changes=0;addEventListener('lens-control-palette',()=>window.changes++);</script><iframe srcdoc='<p>child</p>'></iframe>";
        NSURL *page = [NSURL fileURLWithPath:[NSTemporaryDirectory()
            stringByAppendingPathComponent:[NSString stringWithFormat:@"lens-palette-%@.html", NSUUID.UUID.UUIDString]]];
        Require([html writeToURL:page atomically:YES encoding:NSUTF8StringEncoding error:nil], @"probe HTML is written");
        [webview loadFileURL:page allowingReadAccessToURL:page];
        PumpUntil(^BOOL { return navigation.finished; });
        NSDictionary *light = Palette(webview);
        Require([Evaluate(webview, @"window.firstPalette") isEqual:light], @"current palette available before first page script");
        Require([Evaluate(webview, @"typeof frames[0].__LENS_CONTROL_PALETTE__") isEqual:@"undefined"], @"palette is main-frame only");

        window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        NSDictionary *dark = Palette(webview);
        Require(![dark[@"colors"][@"control_surface"] isEqual:light[@"colors"][@"control_surface"]], @"window effectiveAppearance updates live");
        Require([Evaluate(webview, @"window.changes") intValue] >= 1, @"runtime update dispatches event");
        Require(window.backgroundColor.alphaComponent == 0 && webview.underPageBackgroundColor.alphaComponent == 0,
            @"palette updates preserve transparent window and under-page backgrounds");
        NSWindow *otherWindow = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 100, 100)
            styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
        otherWindow.releasedWhenClosed = NO;
        otherWindow.appearance = [NSAppearance appearanceNamed:NSAppearanceNameAqua];
        LensControlPalette otherPalette;
        Require(lens_control_palette((__bridge void *)otherWindow, &otherPalette), @"second window palette resolves");
        Require([light[@"colors"][@"control_surface"][0] intValue] == otherPalette.control_surface[0],
            @"separate windows resolve their own effective appearances");
        [otherWindow close];
        NSLog(@"Light %@; Dark %@", light, dark);
        navigation.finished = NO;
        [webview reload];
        PumpUntil(^BOOL { return navigation.finished; });
        Require([Evaluate(webview, @"window.firstPalette") isEqual:dark], @"reload starts with latest palette, not builder seed");

        int beforeColors = [Evaluate(webview, @"window.changes") intValue];
        [NSNotificationCenter.defaultCenter postNotificationName:NSSystemColorsDidChangeNotification object:nil];
        int afterColors = [Evaluate(webview, @"window.changes") intValue];
        Require(afterColors > beforeColors, @"system color notification refreshes snapshot");
        [NSWorkspace.sharedWorkspace.notificationCenter postNotificationName:NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification object:nil];
        Require([Evaluate(webview, @"window.changes") intValue] > afterColors, @"accessibility notification refreshes snapshot");
        Require(window.backgroundColor.alphaComponent == 0 && webview.underPageBackgroundColor.alphaComponent == 0,
            @"system and accessibility updates preserve transparent backgrounds");
        int beforeActivity = [Evaluate(webview, @"window.changes") intValue];
        [NSNotificationCenter.defaultCenter postNotificationName:NSApplicationDidResignActiveNotification object:NSApp];
        Require([Evaluate(webview, @"window.changes") intValue] > beforeActivity, @"application activity refreshes snapshot");
        Palette(webview); // Actual activity is queried; posting a notification does not fake OS state.

        // Process-local fault injection tests color failure without changing accessibility settings.
        Method fillMethod = class_getClassMethod(NSColor.class, @selector(secondarySystemFillColor));
        IMP unavailableFill = imp_implementationWithBlock(^NSColor *(id receiver) { return nil; });
        IMP originalFill = method_setImplementation(fillMethod, unavailableFill);
        [NSNotificationCenter.defaultCenter postNotificationName:NSSystemColorsDidChangeNotification object:nil];
        NSDictionary *unavailable = Evaluate(webview, @"window.__LENS_CONTROL_PALETTE__");
        Require(unavailable[@"colors"] == NSNull.null, @"color failure atomically clears only colors");
        Require([unavailable[@"increase_contrast"] boolValue] == NSWorkspace.sharedWorkspace.accessibilityDisplayShouldIncreaseContrast
            && [unavailable[@"reduce_transparency"] boolValue] == NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceTransparency
            && [unavailable[@"window_active"] boolValue] == (window.isKeyWindow && NSApp.isActive),
            @"all real state flags survive color failure");
        navigation.finished = NO;
        [webview reload];
        PumpUntil(^BOOL { return navigation.finished; });
        Require([Evaluate(webview, @"window.firstPalette") isEqual:unavailable], @"color failure remains safe on reload");
        method_setImplementation(fillMethod, originalFill);
        imp_removeBlock(unavailableFill);
        [NSNotificationCenter.defaultCenter postNotificationName:NSSystemColorsDidChangeNotification object:nil];
        Palette(webview);

        [window close];
        Require(controller.userScripts.count == 2 && controller.userScripts[0] == seed && controller.userScripts[1] == unrelated,
            @"close removes only owned script");
        int changes = [Evaluate(webview, @"window.changes") intValue];
        [NSNotificationCenter.defaultCenter postNotificationName:NSSystemColorsDidChangeNotification object:nil];
        [NSWorkspace.sharedWorkspace.notificationCenter postNotificationName:NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification object:nil];
        window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameAqua];
        Require([Evaluate(webview, @"window.changes") intValue] == changes, @"close releases notifications and appearance observer");
        [NSFileManager.defaultManager removeItemAtURL:page error:nil];
        fprintf(stdout, "PASS: palette resolution, appearance updates, atomic event, document-start reload, main-frame isolation, script identity, replacement and close cleanup\n");
    }
    return 0;
}
