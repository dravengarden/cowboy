// Native-layer tweaks for the cowboy WKWebView shell. These are the whole point
// of wrapping the web UI in Tauri: things a pure-web PWA on iOS cannot do.
//
// Compiled into the app (xcodegen includes everything under Sources/); the
// `+load` runs once at image load, before any WebView exists, so the swizzle is
// in place by the time the first text field is focused.
//
// NOTE: this file lives in the generated gen/apple tree but is hand-authored and
// committed. If you ever re-run `cargo tauri ios init`, confirm it survived.
#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

// (1) Remove the iOS keyboard accessory bar (the ∧ ∨ + Done strip above the
// keyboard). It cannot be removed from a pure-web PWA — only a native WKWebView
// owner can, by making the private WKContentView return a nil inputAccessoryView.
// cowboy has its own in-UI send/compose affordances, so the bar is pure noise.
__attribute__((constructor)) static void cowboyStripKeyboardAccessoryBar(void) {
    @autoreleasepool {
        Class cls = NSClassFromString(@"WKContentView");
        if (!cls) {
            return;
        }
        SEL sel = @selector(inputAccessoryView);
        IMP nilImp = imp_implementationWithBlock(^id(id _self) { return nil; });

        Method existing = class_getInstanceMethod(cls, sel);
        const char *types = existing ? method_getTypeEncoding(existing) : "@@:";

        // class_addMethod adds an override ONLY on WKContentView when the method
        // is inherited; it returns NO when WKContentView already defines its own,
        // in which case we replace that own implementation. Either way we never
        // touch a shared superclass implementation.
        if (!class_addMethod(cls, sel, nilImp, types)) {
            method_setImplementation(class_getInstanceMethod(cls, sel), nilImp);
        }
    }
}

// (2) Native haptic bridge. iOS web (Safari AND a WKWebView) has no Vibration
// API, so the web UI calls `window.__cowboyHaptic()` (see web/src/haptic.ts) and
// we fire a real Taptic-engine tap here. Wired by swizzling WKWebView's
// designated initializer so it lands on the shell's web view at creation: into
// each new view's configuration we (a) register a `cowboyHaptic` script-message
// handler that runs a UIImpactFeedbackGenerator, and (b) inject the JS shim that
// posts to it. Pure native — no Tauri IPC/command exposed to the remote origin.

@interface CowboyHapticHandler : NSObject <WKScriptMessageHandler>
@end

@implementation CowboyHapticHandler {
    UIImpactFeedbackGenerator *_gen;
}
- (instancetype)init {
    if ((self = [super init])) {
        _gen = [[UIImpactFeedbackGenerator alloc] initWithStyle:UIImpactFeedbackStyleLight];
    }
    return self;
}
- (void)userContentController:(WKUserContentController *)ucc
      didReceiveScriptMessage:(WKScriptMessage *)message {
    // `prepare` warms the engine so the tap fires with minimal latency.
    [_gen prepare];
    [_gen impactOccurred];
}
@end

__attribute__((constructor)) static void cowboyInstallHapticBridge(void) {
    @autoreleasepool {
        Class cls = [WKWebView class];
        SEL sel = @selector(initWithFrame:configuration:);
        Method m = class_getInstanceMethod(cls, sel);
        if (!m) {
            return;
        }
        IMP original = method_getImplementation(m);
        IMP replacement = imp_implementationWithBlock(
            ^WKWebView *(id _self, CGRect frame, WKWebViewConfiguration *config) {
                @try {
                    WKUserContentController *ucc = config.userContentController;
                    if (ucc == nil) {
                        ucc = [[WKUserContentController alloc] init];
                        config.userContentController = ucc;
                    }
                    static CowboyHapticHandler *handler;
                    static dispatch_once_t once;
                    dispatch_once(&once, ^{ handler = [[CowboyHapticHandler alloc] init]; });
                    // Re-adding the same name on a ucc throws; the @try swallows it
                    // (a webview reusing a ucc keeps the first registration).
                    [ucc addScriptMessageHandler:handler name:@"cowboyHaptic"];
                    NSString *js =
                        @"window.__cowboyHaptic=function(){try{"
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage(0)}catch(e){}};";
                    WKUserScript *script =
                        [[WKUserScript alloc] initWithSource:js
                                               injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                            forMainFrameOnly:NO];
                    [ucc addUserScript:script];
                } @catch (__unused NSException *e) {
                }
                return ((WKWebView * (*)(id, SEL, CGRect, WKWebViewConfiguration *)) original)(
                    _self, sel, frame, config);
            });
        method_setImplementation(m, replacement);
    }
}
