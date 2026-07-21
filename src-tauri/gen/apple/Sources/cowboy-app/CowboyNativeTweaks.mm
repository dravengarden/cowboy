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
#import <objc/message.h>
#import <objc/runtime.h>
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

// (1) Remove the iOS keyboard accessory bar (the ∧ ∨ + Done strip above the
// keyboard). It cannot be removed from a pure-web PWA — only a native WKWebView
// owner can, by making the private WKContentView return a nil inputAccessoryView.
// cowboy has its own in-UI send/compose affordances, so the bar is pure noise.
//
// A/B DIAGNOSTIC (work-items/archive/2026/07/cowboy-ios-native-shell-fixes,
// BUG 1). After the
// keyboard avoider (#3) was RULED OUT on-device, this swizzle is the prime suspect
// for the missing empty-area Paste callout: it mutates `WKContentView` — the SAME
// private view that hosts the caret / selection / edit-menu text interaction. Set
// to 1 to BUILD WITHOUT this swizzle (the ∧∨ Done bar comes back — expected for the
// test). If the empty-area long-press Paste menu then RETURNS → this swizzle is the
// culprit → replace it with a surgical hide that doesn't disturb the text
// interaction. If it STILL doesn't appear → it's not this; keep digging in wry.
//
// A/B RESULT (2026-06-12): disabling this swizzle did NOT bring back the empty-area
// Paste menu → this swizzle is RULED OUT too. With #3 (avoider) also ruled out and
// the web byte-identical to Obsidian, the empty-area Paste menu is an UPSTREAM
// wry/tao WKWebView limitation (wry-v0.55.1 is the latest; no fix in its releases).
// Re-enabled (=0); the dependable path is the in-UI Paste button (web v222/v224).
#define COWBOY_AB_DISABLE_INPUT_ACCESSORY_SWIZZLE 0
__attribute__((constructor)) static void cowboyStripKeyboardAccessoryBar(void) {
#if COWBOY_AB_DISABLE_INPUT_ACCESSORY_SWIZZLE
    NSLog(@"[cowboy] A/B: inputAccessoryView swizzle DISABLED for the empty-area Paste test");
    return;
#else
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
#endif
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

// (2b) Native clipboard READ bridge. iOS WKWebView does NOT grant
// `navigator.clipboard.readText()` (Safari does; a WKWebView app does not), so the
// web UI's in-composer Paste button silently no-op'd in the shell. Expose the
// system pasteboard via a reply-style script handler: JS calls
// `window.__cowboyReadClipboard()` and gets a Promise that resolves to
// `UIPasteboard.general.string`. Reply variant (WKScriptMessageHandlerWithReply,
// iOS 14+) is what lets the handler return a value to the JS Promise. The "Pasted
// from …" banner iOS shows on read is the expected, correct affordance. Text only;
// images still go through the attach button. (cowboy-ios-native-shell-fixes BUG 1
// fallback — the dependable paste path now actually works in the shell.)
@interface CowboyClipboardHandler : NSObject <WKScriptMessageHandlerWithReply>
@end

@implementation CowboyClipboardHandler
- (void)userContentController:(WKUserContentController *)ucc
      didReceiveScriptMessage:(WKScriptMessage *)message
                 replyHandler:(void (^)(id _Nullable, NSString *_Nullable))replyHandler {
    NSString *s = UIPasteboard.generalPasteboard.string;
    replyHandler(s != nil ? s : @"", nil);
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
                    // Clipboard read bridge (own @try: a re-add throw on a reused
                    // ucc must not block the haptic/native-shell wiring above, and
                    // the reply API is iOS 14+ so guard it).
                    @try {
                        if (@available(iOS 14.0, *)) {
                            static CowboyClipboardHandler *clip;
                            static dispatch_once_t clipOnce;
                            dispatch_once(&clipOnce, ^{ clip = [[CowboyClipboardHandler alloc] init]; });
                            [ucc addScriptMessageHandlerWithReply:clip
                                                     contentWorld:WKContentWorld.pageWorld
                                                             name:@"cowboyClipboard"];
                        }
                    } @catch (__unused NSException *e) {
                    }
                    NSString *js =
                        @"window.__cowboyHaptic=function(){try{"
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage(0)}catch(e){}};"
                        // Native clipboard READ (see CowboyClipboardHandler): returns
                        // a Promise<string>. The web's Paste button prefers this in
                        // the shell (navigator.clipboard.readText is blocked here).
                        @"window.__cowboyReadClipboard=function(){try{"
                        @"return window.webkit.messageHandlers.cowboyClipboard.postMessage(0)}"
                        @"catch(e){return Promise.reject(e)}};"
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
#if DEBUG
                // Opt into Safari inspection and install the headless simulator
                // eval bridge. Both are DEBUG-only: release/device distribution
                // builds expose neither an inspector nor a loopback listener.
                if (@available(iOS 16.4, macOS 13.3, *)) {
                    cowboyWv.inspectable = YES;
                }
                SEL devInstallSel = NSSelectorFromString(@"installOnWebView:");
                Class devCls = NSClassFromString(@"CowboyDevBridge");
                if (devCls && [devCls respondsToSelector:devInstallSel]) {
                    ((void (*)(id, SEL, id))objc_msgSend)(devCls, devInstallSel, cowboyWv);
                }
#endif
                return cowboyWv;
            });
        method_setImplementation(m, replacement);
    }
}

// (2c) Native foreground lifecycle bridge. WKWebView may keep the document
// `visible` while the app is backgrounded and resumed, emitting neither
// visibilitychange nor pageshow. Tell the web app explicitly whenever UIKit says
// the application became active so its service-worker coordinator can check for
// a freshly deployed Cowboy bundle immediately.
@interface CowboyLifecycleBridge : NSObject
@end

@implementation CowboyLifecycleBridge

+ (instancetype)shared {
    static CowboyLifecycleBridge *inst;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ inst = [[CowboyLifecycleBridge alloc] init]; });
    return inst;
}

- (instancetype)init {
    if ((self = [super init])) {
        [[NSNotificationCenter defaultCenter]
            addObserver:self
               selector:@selector(onDidBecomeActive:)
                   name:UIApplicationDidBecomeActiveNotification
                 object:nil];
    }
    return self;
}

- (void)onDidBecomeActive:(NSNotification *)note {
    (void)note;
    WKWebView *wv = gCowboyWebView;
    if (wv == nil) return;
    [wv evaluateJavaScript:
            @"window.dispatchEvent(new Event('cowboy:native-resume'))"
         completionHandler:nil];
}

@end

__attribute__((constructor)) static void cowboyInstallLifecycleBridge(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            (void)[CowboyLifecycleBridge shared];
        }
    });
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
// See work-items/archive/2026/07/cowboy-native-keyboard-ime.
@interface CowboyKeyboardAvoider : NSObject
@end

@implementation CowboyKeyboardAvoider {
    NSUInteger _settleGeneration;
    BOOL _keyboardVisible;
}

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
        // A 3rd-party keyboard (e.g. WeChat) reports its frame in PHASES on the
        // FIRST open: WillChangeFrame fires with a frame TALLER than what's actually
        // rendered that instant, so the avoider OVER-shrinks → a dead gap appears
        // above the keyboard (it self-heals on the 2nd open, when the keyboard is
        // warm and reports its settled frame). Re-measure + re-apply on DidShow,
        // once the final layout is in place, to correct that first-open over-shrink.
        // (cowboy-ios-native-shell-fixes BUG 2)
        [nc addObserver:self
               selector:@selector(onKeyboardDidShow:)
                   name:UIKeyboardDidShowNotification
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

// Compute the overlap from a keyboard notification's settled end-frame and apply
// it. Shared by WillChangeFrame (tracks the slide) and DidShow (corrects a
// 3rd-party keyboard's first-open over-shrink — its end-frame is only trustworthy
// once the keyboard is actually shown).
- (void)applyFromNote:(NSNotification *)note {
    WKWebView *wv = gCowboyWebView;
    UIView *parent = wv.superview;
    if (wv == nil || parent == nil || wv.window == nil) return;
    CGRect kbScreen = [note.userInfo[UIKeyboardFrameEndUserInfoKey] CGRectValue];
    // Keyboard frame → parent coords. Only a keyboard attached to the bottom
    // edge owns viewport resize. Floating/undocked iPad keyboards overlay the
    // document; trimming the whole WebView to their top creates a huge blank
    // region below the composer.
    CGRect kbInParent = [parent convertRect:kbScreen fromView:nil];
    CGFloat parentBottom = CGRectGetMaxY(parent.bounds);
    BOOL dockedToBottom = CGRectGetMaxY(kbInParent) >= parentBottom - 2;
    CGFloat overlap = dockedToBottom
        ? MAX(0, parentBottom - CGRectGetMinY(kbInParent))
        : 0;
    [self applyOverlap:overlap userInfo:note.userInfo];
}

// Notification end frames are predictions made before UIKit finishes laying out
// the keyboard. Predictive/IME bars and third-party keyboards can change that
// geometry after DidShow without another dependable frame notification. On iOS
// 15+, keyboardLayoutGuide is the native view hierarchy's current, authoritative
// keyboard edge. Reconcile the web view against it after each settling phase so
// an early over-tall prediction cannot leave a gray strip above the real keyboard.
- (void)applySettledLayoutGuide:(NSUInteger)generation {
    if (generation != _settleGeneration || !_keyboardVisible) return;
    WKWebView *wv = gCowboyWebView;
    UIView *parent = wv.superview;
    if (wv == nil || parent == nil || wv.window == nil) return;
    if (@available(iOS 15.0, *)) {
        // The default guide follows only a docked, full-width keyboard. iPad's
        // split/floating keyboard can therefore settle somewhere completely
        // different from UIKeyboardFrameEnd while the guide stays collapsed at
        // the safe area. Opt into the real undocked geometry before measuring.
        parent.keyboardLayoutGuide.followsUndockedKeyboard = YES;
        [parent layoutIfNeeded];
        CGRect keyboardFrame = parent.keyboardLayoutGuide.layoutFrame;
        CGFloat keyboardHeight = CGRectGetHeight(keyboardFrame);
        // A collapsed guide is transient/hidden, not authoritative. Keep the
        // notification animation until a real keyboard frame is available.
        if (keyboardHeight < 80) return;
        CGFloat parentBottom = CGRectGetMaxY(parent.bounds);
        BOOL dockedToBottom = CGRectGetMaxY(keyboardFrame) >= parentBottom - 2;
        // A genuinely floating keyboard must overlay the document instead of
        // shortening the entire WKWebView to its top edge. A docked split
        // keyboard still reaches the bottom and receives the normal overlap.
        CGFloat overlap = dockedToBottom
            ? MAX(0, parentBottom - CGRectGetMinY(keyboardFrame))
            : 0;
        [UIView performWithoutAnimation:^{
            CGRect frame = parent.bounds;
            frame.size.height = MAX(0, frame.size.height - overlap);
            wv.frame = frame;
            [wv layoutIfNeeded];
        }];
#if DEBUG
        NSLog(@"[cowboy] keyboard settled frame=%@ docked=%d overlap=%.1f webHeight=%.1f",
              NSStringFromCGRect(keyboardFrame), dockedToBottom,
              overlap, CGRectGetHeight(wv.frame));
#endif
    }
}

- (void)scheduleSettledCorrections {
    NSUInteger generation = ++_settleGeneration;
    // Cover the normal animation completion plus late predictive/third-party
    // keyboard phases. Generation checks make every older schedule harmless
    // after a new frame or hide event.
    for (NSNumber *delay in @[@0.0, @0.12, @0.35, @0.70]) {
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW,
                          (int64_t)(delay.doubleValue * NSEC_PER_SEC)),
            dispatch_get_main_queue(), ^{
                [self applySettledLayoutGuide:generation];
            });
    }
}

- (void)onKeyboardWillChangeFrame:(NSNotification *)note {
    _keyboardVisible = YES;
    [self applyFromNote:note];
    [self scheduleSettledCorrections];
}

// Re-apply once the keyboard has fully settled — fixes the WeChat-keyboard
// first-open gap (BUG 2). Idempotent with WillChangeFrame for the system keyboard
// (same settled frame → same overlap → no visible change).
- (void)onKeyboardDidShow:(NSNotification *)note {
    _keyboardVisible = YES;
    [self applyFromNote:note];
    [self scheduleSettledCorrections];
}

- (void)onKeyboardWillHide:(NSNotification *)note {
    _keyboardVisible = NO;
    ++_settleGeneration;
    [self applyOverlap:0 userInfo:note.userInfo];
}

@end

// A/B DIAGNOSTIC TOGGLE
// (work-items/archive/2026/07/cowboy-ios-native-shell-fixes, BUG 1).
// Set to 1 to BUILD WITHOUT the keyboard avoider, to test whether its `wv.frame`
// mutation is what suppresses the empty-area long-press Paste callout. With this
// =1 the keyboard will OVERLAP the composer (no native viewport shrink) — that's
// expected for the test build; it's purely to isolate the cause. If the Paste
// menu RETURNS on the empty area with the avoider off → the frame mutation is the
// culprit → rework `applyOverlap:` to resize via Auto-Layout constraints /
// scrollView.contentInset instead of `wv.frame =`. If it STILL doesn't appear →
// the cause is elsewhere (wry's WKWebView config) and we keep the avoider. Flip
// back to 0 once diagnosed.
// A/B RESULT (2026-06-12): disabling the avoider did NOT bring back the empty-area
// Paste menu → the `wv.frame` mutation is RULED OUT as the cause. Avoider re-enabled
// (=0); the empty-area Paste cause is in wry's WKWebView creation — see the new
// text-interaction tweak below.
#define COWBOY_AB_DISABLE_KB_AVOIDER 0

__attribute__((constructor)) static void cowboyInstallKeyboardAvoider(void) {
#if COWBOY_AB_DISABLE_KB_AVOIDER
    NSLog(@"[cowboy] A/B: keyboard avoider DISABLED for the empty-area Paste test");
    return;
#else
    // Register the observers on the main thread (NotificationCenter delivery +
    // UIKit frame mutation must be main-thread). `+load`/constructors run early —
    // before any window — so defer to the main queue.
    dispatch_async(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            (void)[CowboyKeyboardAvoider shared];
        }
    });
#endif
}
