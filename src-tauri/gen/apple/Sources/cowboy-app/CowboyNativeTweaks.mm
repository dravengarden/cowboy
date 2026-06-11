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

// The shell's main WKWebView, captured at creation (below) for the keyboard
// avoider. Weak: the avoider just no-ops if it's gone.
static __weak WKWebView *gCowboyWebView = nil;

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
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage(0)}catch(e){}};"
                        // ARM the web's native-shell gate (src/nativeShell.ts): the
                        // shell now does native keyboard avoidance (below), so the
                        // web drops its position:fixed/translateZ/IME-composition
                        // hacks. document-start, so it's set before the page's own
                        // boot script reads it. (cowboy-native-keyboard-ime)
                        @"window.__cowboyNativeShell=true;";
                    WKUserScript *script =
                        [[WKUserScript alloc] initWithSource:js
                                               injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                            forMainFrameOnly:NO];
                    [ucc addUserScript:script];
                } @catch (__unused NSException *e) {
                }
                WKWebView *cowboyWv =
                    ((WKWebView * (*)(id, SEL, CGRect, WKWebViewConfiguration *)) original)(
                        _self, sel, frame, config);
                // Capture the shell's web view for the keyboard avoider below. The
                // thin shell creates one main web view; the latest assignment wins.
                gCowboyWebView = cowboyWv;
                return cowboyWv;
            });
        method_setImplementation(m, replacement);
    }
}

// (3) Native keyboard avoidance — the Obsidian/Capacitor "resize: native" model.
// On keyboard show, shrink the WKWebView's frame by the keyboard's overlap so the
// web's LAYOUT viewport (and `vh`/`100%`) shrinks with it; restore on hide. This is
// the WHOLE point of going native for IME: with the viewport shrinking natively,
// the remote web UI can drop the PWA's position:fixed lock — and therefore the
// translateZ repaint hack that mis-paints iOS pinyin (the swallow/caret bugs). The
// web arms this path off `window.__cowboyNativeShell` (injected above). A pure-web
// PWA can't do this: WKWebView does NOT honour interactive-widget/visualViewport
// for the keyboard, so only the native owner can shrink the viewport.
// See tasks/active/cowboy-native-keyboard-ime.
@interface CowboyKeyboardAvoider : NSObject
@end

@implementation CowboyKeyboardAvoider

+ (instancetype)shared {
    static CowboyKeyboardAvoider *inst;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ inst = [[CowboyKeyboardAvoider alloc] init]; });
    return inst;
}

- (instancetype)init {
    if ((self = [super init])) {
        NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
        [nc addObserver:self
               selector:@selector(onKeyboardWillChangeFrame:)
                   name:UIKeyboardWillChangeFrameNotification
                 object:nil];
        // Explicit hide handler is the safety net for the documented WKWebView
        // "doesn't resize back / black strip on close" gotcha — force full there.
        [nc addObserver:self
               selector:@selector(onKeyboardWillHide:)
                   name:UIKeyboardWillHideNotification
                 object:nil];
    }
    return self;
}

// Trim the web view to `full.height − overlap` (overlap 0 == full), animated with
// the keyboard's own duration/curve so it tracks the slide. Keeps origin + width;
// only the height changes. The parent's bounds are the stable full extent (the
// keyboard never resizes the parent), so this is idempotent + safe to re-apply.
- (void)applyOverlap:(CGFloat)overlap userInfo:(NSDictionary *)info {
    WKWebView *wv = gCowboyWebView;
    UIView *parent = wv.superview;
    if (wv == nil || parent == nil || wv.window == nil) return;
    CGRect full = parent.bounds;

    NSTimeInterval duration =
        info != nil ? [info[UIKeyboardAnimationDurationUserInfoKey] doubleValue] : 0.25;
    UIViewAnimationCurve curve =
        info != nil
            ? (UIViewAnimationCurve)[info[UIKeyboardAnimationCurveUserInfoKey] integerValue]
            : UIViewAnimationCurveEaseInOut;

    [UIView animateWithDuration:duration
                          delay:0
                        options:(UIViewAnimationOptions)(curve << 16)
                     animations:^{
                         CGRect f = full;
                         f.size.height = MAX(0, full.size.height - overlap);
                         wv.frame = f;
                         [wv layoutIfNeeded];
                     }
                     completion:nil];
}

- (void)onKeyboardWillChangeFrame:(NSNotification *)note {
    WKWebView *wv = gCowboyWebView;
    UIView *parent = wv.superview;
    if (wv == nil || parent == nil || wv.window == nil) return;
    CGRect kbScreen = [note.userInfo[UIKeyboardFrameEndUserInfoKey] CGRectValue];
    // Keyboard frame → parent coords; overlap = how far it intrudes from the bottom.
    CGRect kbInParent = [parent convertRect:kbScreen fromView:nil];
    CGFloat overlap = MAX(0, CGRectGetMaxY(parent.bounds) - CGRectGetMinY(kbInParent));
    [self applyOverlap:overlap userInfo:note.userInfo];
}

- (void)onKeyboardWillHide:(NSNotification *)note {
    [self applyOverlap:0 userInfo:note.userInfo];
}

@end

__attribute__((constructor)) static void cowboyInstallKeyboardAvoider(void) {
    // Register the observers on the main thread (NotificationCenter delivery +
    // UIKit frame mutation must be main-thread). `+load`/constructors run early —
    // before any window — so defer to the main queue.
    dispatch_async(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            (void)[CowboyKeyboardAvoider shared];
        }
    });
}
