#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>
#include <math.h>
#import "LensNative.h"

// The observer retains its own script identity; other initialization scripts survive.
static NSString *const LensControlPaletteScriptPrefix = @"/* lens-control-palette */";
static const char LensControlPaletteAssociation;
static const char LensControlAppearanceObservation;

static bool LensOpaqueColor(NSColor *foreground, NSColor *background, uint8_t *rgba) {
    NSColor *source = [foreground colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    NSColor *destination = [background colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    if (!source || !destination || destination.alphaComponent != 1.0) return false;
    CGFloat alpha = source.alphaComponent;
    rgba[0] = (uint8_t)lround((source.redComponent * alpha + destination.redComponent * (1.0 - alpha)) * 255.0);
    rgba[1] = (uint8_t)lround((source.greenComponent * alpha + destination.greenComponent * (1.0 - alpha)) * 255.0);
    rgba[2] = (uint8_t)lround((source.blueComponent * alpha + destination.blueComponent * (1.0 - alpha)) * 255.0);
    rgba[3] = 255;
    return true;
}

static bool LensRawColor(NSColor *color, uint8_t *rgba) {
    NSColor *resolved = [color colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    if (!resolved) return false;
    rgba[0] = (uint8_t)lround(resolved.redComponent * 255.0);
    rgba[1] = (uint8_t)lround(resolved.greenComponent * 255.0);
    rgba[2] = (uint8_t)lround(resolved.blueComponent * 255.0);
    rgba[3] = (uint8_t)lround(resolved.alphaComponent * 255.0);
    return true;
}

static double LensLinearComponent(double value) {
    return value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4);
}
static double LensLuminance(const double *linear) {
    return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}
static double LensByteLuminance(const uint8_t *rgba) {
    double linear[3];
    for (size_t i = 0; i < 3; i++) linear[i] = LensLinearComponent(rgba[i] / 255.0);
    return LensLuminance(linear);
}
static void LensQuantizedFill(const double *linear, double destination, double amount, uint8_t *rgba) {
    for (size_t i = 0; i < 3; i++) {
        double value = linear[i] + (destination - linear[i]) * amount;
        double encoded = value <= 0.0031308 ? value * 12.92 : 1.055 * pow(value, 1.0 / 2.4) - 0.055;
        rgba[i] = (uint8_t)lround(fmin(1.0, fmax(0.0, encoded)) * 255.0);
    }
    rgba[3] = 255;
}
static bool LensPrimaryContrastPasses(const uint8_t *fill, bool whiteText, double required) {
    double luminance = LensByteLuminance(fill);
    double contrast = whiteText ? 1.05 / (luminance + 0.05) : (luminance + 0.05) / 0.05;
    return contrast >= required;
}

bool lens_primary_button_colors(const double *accentRGB, bool increaseContrast,
                                uint8_t *fillRGBA, uint8_t *foregroundRGBA) {
    if (!accentRGB || !fillRGBA || !foregroundRGBA) return false;
    double linear[3];
    for (size_t i = 0; i < 3; i++) {
        if (!isfinite(accentRGB[i]) || accentRGB[i] < 0.0 || accentRGB[i] > 1.0) return false;
        linear[i] = LensLinearComponent(accentRGB[i]);
    }
    // WebKit's native submit painter uses this 0.5 luminance split, unlike AccentColorText.
    // Contrast floors and minimal background adjustment are explicit Lens policy.
    bool whiteText = LensLuminance(linear) <= 0.5;
    double required = increaseContrast ? 7.0 : 4.5;
    double destination = whiteText ? 0.0 : 1.0;
    LensQuantizedFill(linear, destination, 0.0, fillRGBA);
    if (!LensPrimaryContrastPasses(fillRGBA, whiteText, required)) {
        double failing = 0.0, passing = 1.0;
        LensQuantizedFill(linear, destination, passing, fillRGBA);
        // Search the monotonic, byte-quantized path itself: rounding cannot undo the floor.
        for (size_t iteration = 0; iteration < 40; iteration++) {
            double amount = (failing + passing) / 2.0;
            uint8_t candidate[4];
            LensQuantizedFill(linear, destination, amount, candidate);
            if (LensPrimaryContrastPasses(candidate, whiteText, required)) {
                passing = amount;
                for (size_t i = 0; i < 4; i++) fillRGBA[i] = candidate[i];
            } else {
                failing = amount;
            }
        }
    }
    for (size_t i = 0; i < 3; i++) foregroundRGBA[i] = whiteText ? 255 : 0;
    foregroundRGBA[3] = 255;
    return LensPrimaryContrastPasses(fillRGBA, whiteText, required);
}

static bool LensPrimaryColors(bool increaseContrast, uint8_t *fill, uint8_t *foreground) {
    NSColor *accent = [NSColor.controlAccentColor colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    if (!accent || accent.alphaComponent != 1.0) return false;
    double rgb[3] = {accent.redComponent, accent.greenComponent, accent.blueComponent};
    return lens_primary_button_colors(rgb, increaseContrast, fill, foreground);
}

bool lens_control_palette(void *windowPointer, LensControlPalette *palette) {
    if (![NSThread isMainThread] || !NSApp || !palette) return false;
    NSWindow *window = (__bridge NSWindow *)windowPointer;
    __block LensControlPalette resolved = {0};
    resolved.increase_contrast = NSWorkspace.sharedWorkspace.accessibilityDisplayShouldIncreaseContrast;
    resolved.reduce_transparency = NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceTransparency;
    resolved.window_active = window.isKeyWindow && NSApp.isActive;
    if (@available(macOS 14.0, *)) {
        NSAppearance *appearance = window ? window.effectiveAppearance : NSApp.effectiveAppearance;
        [appearance performAsCurrentDrawingAppearance:^{
            NSColor *background = NSColor.windowBackgroundColor;
            resolved.colors_available =
                LensOpaqueColor(background, background, resolved.window_surface)
                && LensOpaqueColor(NSColor.quaternarySystemFillColor, background, resolved.control_surface)
                && LensRawColor(NSColor.secondarySystemFillColor, resolved.button_fill)
                && LensRawColor(NSColor.systemFillColor, resolved.button_pressed_fill)
                && LensRawColor(NSColor.separatorColor, resolved.separator)
                && LensPrimaryColors(resolved.increase_contrast, resolved.primary_button_fill, resolved.primary_button_foreground);
        }];
    }
    *palette = resolved;
    return true;
}

static NSArray<NSNumber *> *LensPaletteRGBA(const uint8_t *rgba) {
    return @[@(rgba[0]), @(rgba[1]), @(rgba[2]), @(rgba[3])];
}

static NSString *LensControlPaletteScript(NSWindow *window) {
    LensControlPalette palette;
    NSString *json = @"null";
    if (lens_control_palette((__bridge void *)window, &palette)) {
        id colors = palette.colors_available ? @{
            @"control_surface": LensPaletteRGBA(palette.control_surface),
            @"window_surface": LensPaletteRGBA(palette.window_surface),
            @"button_fill": LensPaletteRGBA(palette.button_fill),
            @"button_pressed_fill": LensPaletteRGBA(palette.button_pressed_fill),
            @"separator": LensPaletteRGBA(palette.separator),
            @"primary_button_fill": LensPaletteRGBA(palette.primary_button_fill),
            @"primary_button_foreground": LensPaletteRGBA(palette.primary_button_foreground),
        } : NSNull.null;
        NSDictionary *value = @{
            @"colors": colors,
            @"increase_contrast": @(palette.increase_contrast),
            @"reduce_transparency": @(palette.reduce_transparency),
            @"window_active": @(palette.window_active),
        };
        NSData *data = [NSJSONSerialization dataWithJSONObject:value options:NSJSONWritingSortedKeys error:nil];
        json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"null";
    }
    return [NSString stringWithFormat:
        @"%@\nif(window===window.top){window.__LENS_CONTROL_PALETTE__=%@;window.dispatchEvent(new CustomEvent('lens-control-palette',{detail:window.__LENS_CONTROL_PALETTE__}));}",
        LensControlPaletteScriptPrefix, json];
}

@interface LensControlPaletteObserver : NSObject
@property(nonatomic, weak) NSWindow *window;
@property(nonatomic, weak) WKWebView *webview;
@property(nonatomic, strong) NSMutableArray *appTokens;
@property(nonatomic, strong) id workspaceToken;
@property(nonatomic, strong) WKUserScript *paletteScript;
@property(nonatomic) BOOL observingAppearance;
@property(nonatomic) BOOL invalidated;
- (instancetype)initWithWindow:(NSWindow *)window webview:(WKWebView *)webview;
- (void)refresh;
- (void)invalidate;
@end

@implementation LensControlPaletteObserver
- (instancetype)initWithWindow:(NSWindow *)window webview:(WKWebView *)webview {
    self = [super init];
    if (!self) return nil;
    _window = window;
    _webview = webview;
    _appTokens = [NSMutableArray array];
    __weak LensControlPaletteObserver *weakSelf = self;
    NSNotificationCenter *center = NSNotificationCenter.defaultCenter;
    for (NSNotificationName name in @[NSSystemColorsDidChangeNotification,
                                     NSApplicationDidBecomeActiveNotification, NSApplicationDidResignActiveNotification,
                                     NSWindowDidBecomeKeyNotification, NSWindowDidResignKeyNotification]) {
        BOOL windowNotification = [name isEqualToString:NSWindowDidBecomeKeyNotification]
            || [name isEqualToString:NSWindowDidResignKeyNotification];
        id token = [center addObserverForName:name
            object:windowNotification ? window : nil
            queue:NSOperationQueue.mainQueue usingBlock:^(__unused NSNotification *notification) {
                [weakSelf refresh];
            }];
        [_appTokens addObject:token];
    }
    [_appTokens addObject:[center addObserverForName:NSWindowWillCloseNotification object:window
        queue:NSOperationQueue.mainQueue usingBlock:^(__unused NSNotification *notification) {
            [weakSelf invalidate];
        }]];
    _workspaceToken = [NSWorkspace.sharedWorkspace.notificationCenter
        addObserverForName:NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification object:nil
        queue:NSOperationQueue.mainQueue usingBlock:^(__unused NSNotification *notification) {
            [weakSelf refresh];
        }];
    [window addObserver:self forKeyPath:@"effectiveAppearance" options:0
        context:(void *)&LensControlAppearanceObservation];
    _observingAppearance = YES;
    [self refresh];
    return self;
}

- (void)observeValueForKeyPath:(NSString *)keyPath ofObject:(id)object
                       change:(NSDictionary *)change context:(void *)context {
    if (context == &LensControlAppearanceObservation) {
        [self refresh];
    } else {
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
    }
}

- (void)refresh {
    if (self.invalidated) return;
    NSWindow *window = self.window;
    WKWebView *webview = self.webview;
    if (!window || !webview) return;
    NSString *source = LensControlPaletteScript(window);
    WKUserContentController *controller = webview.configuration.userContentController;
    // WK has no single-script removal API. Preserve the exact other objects/worlds and order.
    // The builder seed runs first; this current script replaces it before any page modules.
    NSArray<WKUserScript *> *scripts = [controller.userScripts mutableCopy];
    [controller removeAllUserScripts];
    for (WKUserScript *script in scripts) {
        if (script != self.paletteScript) [controller addUserScript:script];
    }
    self.paletteScript = [[WKUserScript alloc] initWithSource:source
        injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES];
    [controller addUserScript:self.paletteScript];
    [webview evaluateJavaScript:source completionHandler:nil];
}

- (void)invalidate {
    if (_invalidated) return;
    _invalidated = YES;
    WKUserContentController *controller = _webview.configuration.userContentController;
    if (_paletteScript && controller) {
        NSArray<WKUserScript *> *scripts = [controller.userScripts mutableCopy];
        [controller removeAllUserScripts];
        for (WKUserScript *script in scripts) {
            if (script != _paletteScript) [controller addUserScript:script];
        }
        _paletteScript = nil;
    }
    if (_observingAppearance) {
        [_window removeObserver:self forKeyPath:@"effectiveAppearance"
            context:(void *)&LensControlAppearanceObservation];
        _observingAppearance = NO;
    }
    for (id token in _appTokens) [NSNotificationCenter.defaultCenter removeObserver:token];
    [_appTokens removeAllObjects];
    if (_workspaceToken) {
        [NSWorkspace.sharedWorkspace.notificationCenter removeObserver:_workspaceToken];
        _workspaceToken = nil;
    }
}
- (void)dealloc {
    [self invalidate];
}
@end

bool lens_observe_control_palette(void *windowPointer, void *webviewPointer) {
    if (![NSThread isMainThread] || !windowPointer || !webviewPointer || !NSApp) return false;
    NSWindow *window = (__bridge NSWindow *)windowPointer;
    WKWebView *webview = (__bridge WKWebView *)webviewPointer;
    LensControlPaletteObserver *previous = objc_getAssociatedObject(window, &LensControlPaletteAssociation);
    [previous invalidate];
    LensControlPaletteObserver *observer = [[LensControlPaletteObserver alloc] initWithWindow:window webview:webview];
    objc_setAssociatedObject(window, &LensControlPaletteAssociation, observer, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    return observer != nil;
}
