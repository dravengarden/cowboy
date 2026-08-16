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
#import <SafariServices/SafariServices.h>
#import <UIKit/UIKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <WebKit/WebKit.h>

#include "CowboyKeyboardGeometry.h"

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
    UIImpactFeedbackGenerator *_legacyImpactGen;
    UIImpactFeedbackGenerator *_drawerImpactGen;
}
- (instancetype)init {
    if ((self = [super init])) {
        _legacyImpactGen =
            [[UIImpactFeedbackGenerator alloc] initWithStyle:UIImpactFeedbackStyleLight];
        // The drawer is a low-priority spatial affordance, not a crisp picker
        // selection. Keep a soft generator alive and prewarm it at touch-down;
        // the deliberately low intensity makes the threshold feel like gentle
        // magnetic resistance instead of a button click.
        _drawerImpactGen =
            [[UIImpactFeedbackGenerator alloc] initWithStyle:UIImpactFeedbackStyleSoft];
    }
    return self;
}
- (void)userContentController:(WKUserContentController *)ucc
      didReceiveScriptMessage:(WKScriptMessage *)message {
    NSString *intent = [message.body isKindOfClass:[NSString class]]
        ? (NSString *)message.body
        : @"legacy-impact";
    if ([intent isEqualToString:@"prepare-selection"]) {
        [_drawerImpactGen prepare];
        return;
    }
    if ([intent isEqualToString:@"selection"]) {
        [_drawerImpactGen impactOccurredWithIntensity:0.68];
        // Keep the engine warm for a quick threshold reversal.
        [_drawerImpactGen prepare];
        return;
    }
    // Backward compatibility for older web bundles.
    [_legacyImpactGen prepare];
    [_legacyImpactGen impactOccurred];
}
@end

// (2b) Native clipboard bridge. A remote-origin WKWebView cannot reliably inspect
// or read clipboard items through navigator.clipboard. Keep metadata and payload
// access separate: the Mobile dock probes image/text capability to render an exact
// enabled state without fetching content, then an explicit Paste tap asks for the
// selected payload. A payload read is the only operation that may show iOS's
// "Pasted from …" affordance. The text reply remains compatible with older web.
@interface CowboyClipboardHandler : NSObject <WKScriptMessageHandlerWithReply>
@end

static NSString *cowboyProviderImageTypeIdentifier(NSItemProvider *provider) {
    for (NSString *identifier in provider.registeredTypeIdentifiers) {
        UTType *type = [UTType typeWithIdentifier:identifier];
        if (type != nil && [type conformsToType:UTTypeImage]) {
            return identifier;
        }
    }
    return nil;
}

static BOOL cowboyProviderCanLoadText(NSItemProvider *provider) {
    // `UIPasteboard.hasStrings` covers eager NSString values, but source apps
    // may publish text lazily through NSItemProvider. Capability checks do not
    // load the payload, so they retain the metadata-only availability contract.
    return [provider canLoadObjectOfClass:NSString.class] ||
           [provider canLoadObjectOfClass:NSAttributedString.class] ||
           [provider canLoadObjectOfClass:NSURL.class];
}

static BOOL cowboyPasteboardHasText(UIPasteboard *pasteboard) {
    if (pasteboard.hasStrings || pasteboard.hasURLs) return YES;
    for (NSItemProvider *provider in pasteboard.itemProviders) {
        if (cowboyProviderCanLoadText(provider)) return YES;
    }
    return NO;
}

static NSUInteger cowboyPasteboardImageCount(UIPasteboard *pasteboard) {
    NSUInteger count = 0;
    // Some apps publish copied photos as lazy NSItemProviders instead of the
    // eager PNG/JPEG representations covered by UIPasteboard.hasImages. Asking
    // whether a provider can vend UIImage or declares a UTType image inspects
    // capability only; it does not load bytes or trigger the paste affordance.
    for (NSItemProvider *provider in pasteboard.itemProviders) {
        if ([provider canLoadObjectOfClass:UIImage.class] ||
            cowboyProviderImageTypeIdentifier(provider) != nil) {
            count += 1;
        }
    }
    if (count > 0) return count;
    if (!pasteboard.hasImages) return 0;
    NSInteger itemCount = pasteboard.numberOfItems;
    return itemCount > 0 ? (NSUInteger)itemCount : 1;
}

static NSArray<NSDictionary *> *cowboyClipboardImagePayloads(
    NSArray<UIImage *> *images
) {
    NSMutableArray<NSDictionary *> *payloads =
        [[NSMutableArray alloc] initWithCapacity:images.count];
    NSUInteger index = 0;
    for (UIImage *image in images) {
        @autoreleasepool {
            NSData *data = UIImagePNGRepresentation(image);
            if (data == nil) continue;
            index += 1;
            [payloads addObject:@{
                @"name": [NSString stringWithFormat:@"pasted-image-%lu.png",
                                                 (unsigned long)index],
                @"mimeType": @"image/png",
                @"data": [data base64EncodedStringWithOptions:0],
            }];
        }
    }
    return payloads;
}

typedef void (^CowboyProviderImageCompletion)(UIImage *_Nullable image);
typedef void (^CowboyProviderTextCompletion)(NSString *_Nullable text);

static void cowboyCompleteProviderText(
    NSString *_Nullable text,
    CowboyProviderTextCompletion completion
) {
    dispatch_async(dispatch_get_main_queue(), ^{
        completion(text);
    });
}

static void cowboyLoadProviderText(
    NSArray<NSItemProvider *> *providers,
    NSUInteger providerIndex,
    CowboyProviderTextCompletion completion
) {
    if (providerIndex >= providers.count) {
        cowboyCompleteProviderText(nil, completion);
        return;
    }

    NSItemProvider *provider = providers[providerIndex];
    void (^continueWith)(NSString *_Nullable) = ^(NSString *_Nullable text) {
        if (text.length > 0) {
            cowboyCompleteProviderText(text, completion);
        } else {
            cowboyLoadProviderText(providers, providerIndex + 1, completion);
        }
    };

    if ([provider canLoadObjectOfClass:NSString.class]) {
        [provider loadObjectOfClass:NSString.class
                  completionHandler:^(id<NSItemProviderReading> object,
                                      __unused NSError *error) {
                      continueWith([object isKindOfClass:NSString.class]
                                       ? (NSString *)object
                                       : nil);
                  }];
        return;
    }
    if ([provider canLoadObjectOfClass:NSAttributedString.class]) {
        [provider loadObjectOfClass:NSAttributedString.class
                  completionHandler:^(id<NSItemProviderReading> object,
                                      __unused NSError *error) {
                      continueWith([object isKindOfClass:NSAttributedString.class]
                                       ? ((NSAttributedString *)object).string
                                       : nil);
                  }];
        return;
    }
    if ([provider canLoadObjectOfClass:NSURL.class]) {
        [provider loadObjectOfClass:NSURL.class
                  completionHandler:^(id<NSItemProviderReading> object,
                                      __unused NSError *error) {
                      continueWith([object isKindOfClass:NSURL.class]
                                       ? ((NSURL *)object).absoluteString
                                       : nil);
                  }];
        return;
    }
    cowboyLoadProviderText(providers, providerIndex + 1, completion);
}

static void cowboyCompleteProviderImage(
    UIImage *_Nullable image,
    CowboyProviderImageCompletion completion
) {
    dispatch_async(dispatch_get_main_queue(), ^{
        completion(image);
    });
}

static void cowboyLoadProviderImageFile(
    NSItemProvider *provider,
    NSString *typeIdentifier,
    CowboyProviderImageCompletion completion
) {
    [provider loadFileRepresentationForTypeIdentifier:typeIdentifier
                                    completionHandler:^(NSURL *url, NSError *error) {
        (void)error;
        NSData *data = url != nil ? [NSData dataWithContentsOfURL:url] : nil;
        UIImage *image = data != nil ? [UIImage imageWithData:data] : nil;
        cowboyCompleteProviderImage(image, completion);
    }];
}

static void cowboyLoadProviderImageRepresentation(
    NSItemProvider *provider,
    NSString *_Nullable typeIdentifier,
    CowboyProviderImageCompletion completion
) {
    if (typeIdentifier == nil) {
        cowboyCompleteProviderImage(nil, completion);
        return;
    }
    [provider loadDataRepresentationForTypeIdentifier:typeIdentifier
                                    completionHandler:^(NSData *data, NSError *error) {
        (void)error;
        UIImage *image = data != nil ? [UIImage imageWithData:data] : nil;
        if (image != nil) {
            cowboyCompleteProviderImage(image, completion);
            return;
        }
        // Screenshot/IME providers sometimes advertise UIImage but vend only a
        // registered file representation. Read that temporary URL while the
        // provider completion still owns it.
        cowboyLoadProviderImageFile(
            provider,
            typeIdentifier,
            completion
        );
    }];
}

static void cowboyLoadProviderImage(
    NSItemProvider *provider,
    CowboyProviderImageCompletion completion
) {
    NSString *typeIdentifier = cowboyProviderImageTypeIdentifier(provider);
    if (![provider canLoadObjectOfClass:UIImage.class]) {
        cowboyLoadProviderImageRepresentation(
            provider,
            typeIdentifier,
            completion
        );
        return;
    }
    [provider loadObjectOfClass:UIImage.class
              completionHandler:^(__kindof id<NSItemProviderReading> object,
                                  NSError *error) {
        (void)error;
        if ([object isKindOfClass:UIImage.class]) {
            cowboyCompleteProviderImage((UIImage *)object, completion);
            return;
        }
        cowboyLoadProviderImageRepresentation(
            provider,
            typeIdentifier,
            completion
        );
    }];
}

static void cowboyLoadProviderImages(
    NSArray<NSItemProvider *> *providers,
    NSUInteger providerIndex,
    NSMutableArray<UIImage *> *images,
    void (^completion)(NSArray<UIImage *> *)
) {
    if (providerIndex >= providers.count) {
        completion([images copy]);
        return;
    }

    NSItemProvider *provider = providers[providerIndex];
    if (![provider canLoadObjectOfClass:UIImage.class] &&
        cowboyProviderImageTypeIdentifier(provider) == nil) {
        cowboyLoadProviderImages(providers, providerIndex + 1, images, completion);
        return;
    }
    cowboyLoadProviderImage(provider, ^(UIImage *image) {
        if (image != nil) [images addObject:image];
        cowboyLoadProviderImages(
            providers,
            providerIndex + 1,
            images,
            completion
        );
    });
}

@implementation CowboyClipboardHandler
- (void)userContentController:(WKUserContentController *)ucc
      didReceiveScriptMessage:(WKScriptMessage *)message
                 replyHandler:(void (^)(id _Nullable, NSString *_Nullable))replyHandler {
    (void)ucc;
    UIPasteboard *pasteboard = UIPasteboard.generalPasteboard;
    NSString *action = nil;
    if ([message.body isKindOfClass:[NSDictionary class]]) {
        id rawAction = ((NSDictionary *)message.body)[@"action"];
        if ([rawAction isKindOfClass:[NSString class]]) {
            action = (NSString *)rawAction;
        }
    }

    if ([action isEqualToString:@"image-status"]) {
        NSUInteger imageCount = cowboyPasteboardImageCount(pasteboard);
        replyHandler(@{
            @"hasImages": @(imageCount > 0),
            @"hasText": @(cowboyPasteboardHasText(pasteboard)),
            @"imageCount": @(imageCount),
            @"changeCount": @(pasteboard.changeCount),
        }, nil);
        return;
    }

    if ([action isEqualToString:@"read-images"]) {
        NSArray<UIImage *> *images = pasteboard.images ?: @[];
        NSInteger changeCount = pasteboard.changeCount;
        if (images.count > 0) {
            replyHandler(@{
                @"images": cowboyClipboardImagePayloads(images),
                @"changeCount": @(changeCount),
            }, nil);
            return;
        }

        // A provider-backed image may advertise capability while the eager
        // `images` convenience property stays empty. This path runs only from
        // the explicit Paste image tap, so loading bytes retains user intent.
        NSArray<NSItemProvider *> *providers = pasteboard.itemProviders ?: @[];
        cowboyLoadProviderImages(
            providers,
            0,
            [[NSMutableArray alloc] init],
            ^(NSArray<UIImage *> *providerImages) {
                replyHandler(@{
                    @"images": cowboyClipboardImagePayloads(providerImages),
                    @"changeCount": @(changeCount),
                }, nil);
            }
        );
        return;
    }

    NSString *text = pasteboard.string;
    if (text.length > 0) {
        replyHandler(text, nil);
        return;
    }
    NSURL *url = pasteboard.URL;
    if (url.absoluteString.length > 0) {
        replyHandler(url.absoluteString, nil);
        return;
    }
    // Like provider-backed images, provider-backed strings/attributed text are
    // loaded only after the explicit Paste gesture. Keep the source provider
    // alive until its asynchronous representation finishes.
    cowboyLoadProviderText(
        pasteboard.itemProviders ?: @[],
        0,
        ^(NSString *providerText) {
            replyHandler(providerText ?: @"", nil);
        }
    );
}
@end

// The shell's main WKWebView, captured at creation (below) for the keyboard
// avoider and Provider authentication presenter. Weak: both just no-op if it is
// gone.
static __weak WKWebView *gCowboyWebView = nil;

// (2c) Provider authentication browser. Browser-code flows do not redirect
// back into Cowboy, so ASWebAuthenticationSession is the wrong lifecycle: the
// Cowboy dialog must keep polling while the user completes an arbitrary number
// of Provider pages. SFSafariViewController provides a trusted Safari surface
// with native Done, back, forward, and Open in Safari controls without replacing
// the shell's one WKWebView or losing its authentication state.
// Retain a user-dismissed browser until the web flow completes or is cancelled.
// This turns Done / swipe-down into a temporary collapse: tapping Open again
// resumes the same Safari controller, including its current page and history.
static SFSafariViewController *gCowboyAuthenticationBrowser = nil;
static NSURL *gCowboyAuthenticationURL = nil;

static UIViewController *cowboyTopViewController(void) {
    UIWindow *window = gCowboyWebView.window;
    UIViewController *controller = window.rootViewController;
    while (controller != nil) {
        UIViewController *next = controller.presentedViewController;
        if (next != nil) {
            controller = next;
            continue;
        }
        if ([controller isKindOfClass:UINavigationController.class]) {
            controller = ((UINavigationController *)controller).visibleViewController;
            continue;
        }
        if ([controller isKindOfClass:UITabBarController.class]) {
            controller = ((UITabBarController *)controller).selectedViewController;
            continue;
        }
        break;
    }
    return controller;
}

static void cowboyDismissAuthenticationBrowser(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        SFSafariViewController *browser = gCowboyAuthenticationBrowser;
        gCowboyAuthenticationBrowser = nil;
        gCowboyAuthenticationURL = nil;
        if (browser.presentingViewController != nil) {
            [browser dismissViewControllerAnimated:YES completion:nil];
        }
    });
}

static SFSafariViewController *cowboyNewAuthenticationBrowser(NSURL *url) {
    SFSafariViewController *browser =
        [[SFSafariViewController alloc] initWithURL:url];
    browser.dismissButtonStyle = SFSafariViewControllerDismissButtonStyleDone;
    browser.modalPresentationStyle = UIModalPresentationPageSheet;
    browser.modalInPresentation = NO;
    if (@available(iOS 15.0, *)) {
        UISheetPresentationController *sheet = browser.sheetPresentationController;
        sheet.detents = @[
            UISheetPresentationControllerDetent.mediumDetent,
            UISheetPresentationControllerDetent.largeDetent,
        ];
        sheet.selectedDetentIdentifier = UISheetPresentationControllerDetentIdentifierLarge;
        sheet.prefersGrabberVisible = YES;
        sheet.prefersScrollingExpandsWhenScrolledToEdge = YES;
        sheet.prefersEdgeAttachedInCompactHeight = YES;
        sheet.widthFollowsPreferredContentSizeWhenEdgeAttached = YES;
    }
    return browser;
}

static void cowboyPresentAuthenticationBrowser(NSURL *url) {
    if (url == nil ||
        !([url.scheme.lowercaseString isEqualToString:@"https"] ||
          [url.scheme.lowercaseString isEqualToString:@"http"])) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        void (^present)(void) = ^{
            UIViewController *presenter = cowboyTopViewController();
            if (presenter == nil) return;
            SFSafariViewController *browser = cowboyNewAuthenticationBrowser(url);
            gCowboyAuthenticationBrowser = browser;
            gCowboyAuthenticationURL = url;
            [presenter presentViewController:browser animated:YES completion:nil];
        };
        SFSafariViewController *existing = gCowboyAuthenticationBrowser;
        if (existing != nil && [gCowboyAuthenticationURL isEqual:url]) {
            if (existing.presentingViewController == nil) {
                UIViewController *presenter = cowboyTopViewController();
                if (presenter != nil) {
                    [presenter presentViewController:existing animated:YES completion:nil];
                }
            }
            return;
        }
        if (existing.presentingViewController != nil) {
            [existing dismissViewControllerAnimated:NO completion:^{
                gCowboyAuthenticationBrowser = nil;
                gCowboyAuthenticationURL = nil;
                present();
            }];
        } else {
            gCowboyAuthenticationBrowser = nil;
            gCowboyAuthenticationURL = nil;
            present();
        }
    });
}

@interface CowboyAuthenticationBrowserHandler : NSObject <WKScriptMessageHandler>
@end

@implementation CowboyAuthenticationBrowserHandler
- (void)userContentController:(WKUserContentController *)ucc
      didReceiveScriptMessage:(WKScriptMessage *)message {
    (void)ucc;
    if (![message.body isKindOfClass:NSDictionary.class]) return;
    NSDictionary *payload = (NSDictionary *)message.body;
    NSString *action = [payload[@"action"] isKindOfClass:NSString.class]
        ? payload[@"action"]
        : nil;
    if ([action isEqualToString:@"close"]) {
        cowboyDismissAuthenticationBrowser();
        return;
    }
    if (![action isEqualToString:@"open"]) return;
    NSString *rawUrl = [payload[@"url"] isKindOfClass:NSString.class]
        ? payload[@"url"]
        : nil;
    cowboyPresentAuthenticationBrowser(
        rawUrl.length > 0 ? [NSURL URLWithString:rawUrl] : nil
    );
}
@end

static CowboyAuthenticationBrowserHandler *cowboyAuthenticationBrowserHandler(void) {
    static CowboyAuthenticationBrowserHandler *handler;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        handler = [[CowboyAuthenticationBrowserHandler alloc] init];
    });
    return handler;
}

static NSString *cowboyAuthenticationBrowserScript(void) {
    return
        @"window.__cowboyOpenAuthenticationBrowser=function(url){try{"
        @"window.webkit.messageHandlers.cowboyAuthenticationBrowser."
        @"postMessage({action:'open',url:url});return true}"
        @"catch(e){return false}};"
        @"window.__cowboyCloseAuthenticationBrowser=function(){try{"
        @"window.webkit.messageHandlers.cowboyAuthenticationBrowser."
        @"postMessage({action:'close'})}catch(e){}};";
}

// A SideStore update can restore an already-running Release WKWebView whose
// document outlives the configuration-time user-script installation. Rebind
// both sides whenever UIKit brings Cowboy to the foreground: the handler on the
// current content controller and the callable functions in the current page.
static void cowboyRepairAuthenticationBrowserBridge(WKWebView *webView) {
    if (webView == nil) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        @try {
            WKUserContentController *ucc = webView.configuration.userContentController;
            [ucc removeScriptMessageHandlerForName:@"cowboyAuthenticationBrowser"];
            [ucc addScriptMessageHandler:cowboyAuthenticationBrowserHandler()
                                   name:@"cowboyAuthenticationBrowser"];
        } @catch (__unused NSException *e) {
        }
        [webView evaluateJavaScript:cowboyAuthenticationBrowserScript()
                  completionHandler:nil];
    });
}

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
                    @try {
                        [ucc addScriptMessageHandler:cowboyAuthenticationBrowserHandler()
                                               name:@"cowboyAuthenticationBrowser"];
                    } @catch (__unused NSException *e) {
                    }
                    NSString *baseJs =
                        @"window.__cowboyHaptic=function(){try{"
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage('legacy-impact')"
                        @"}catch(e){}};"
                        // Low-latency drawer detent: prepare on touch-down, fire
                        // the persistent native selection generator exactly when
                        // the web gesture crosses its commit threshold.
                        @"window.__cowboyPrepareSelectionHaptic=function(){try{"
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage("
                        @"'prepare-selection')}catch(e){}};"
                        @"window.__cowboySelectionHaptic=function(){try{"
                        @"window.webkit.messageHandlers.cowboyHaptic.postMessage('selection')"
                        @"}catch(e){}};"
                        // Native clipboard READ (see CowboyClipboardHandler): returns
                        // a Promise<string>. The web's Paste button prefers this in
                        // the shell (navigator.clipboard.readText is blocked here).
                        @"window.__cowboyReadClipboard=function(){try{"
                        @"return window.webkit.messageHandlers.cowboyClipboard.postMessage(0)}"
                        @"catch(e){return Promise.reject(e)}};"
                        // Clipboard availability is a metadata-only probe. Actual
                        // text/PNG payloads are read only from an explicit action.
                        @"window.__cowboyClipboardImageStatus=function(){try{"
                        @"return window.webkit.messageHandlers.cowboyClipboard.postMessage("
                        @"{action:'image-status'})}catch(e){return Promise.reject(e)}};"
                        @"window.__cowboyReadClipboardImages=function(){try{"
                        @"return window.webkit.messageHandlers.cowboyClipboard.postMessage("
                        @"{action:'read-images'})}catch(e){return Promise.reject(e)}};";
                    // Interactive Provider sign-in belongs in a system Safari
                    // sheet, never in the shell's sole WKWebView. Keep this
                    // fragment reusable so foreground repair installs the exact
                    // same page-world contract as document-start injection.
                    NSString *js = [baseJs stringByAppendingString:
                        cowboyAuthenticationBrowserScript()];
                    js = [js stringByAppendingString:
                        // ARM the web's native-shell gate (src/nativeShell.ts): the
                        // shell now does native keyboard avoidance (below), so the
                        // web drops its position:fixed/translateZ/IME-composition
                        // hacks. document-start, so it's set before the page's own
                        // boot script reads it. (cowboy-native-keyboard-ime)
                        @"window.__cowboyNativeShell=true;"];
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
        NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
        [nc
            addObserver:self
               selector:@selector(onDidBecomeActive:)
                   name:UIApplicationDidBecomeActiveNotification
                 object:nil];
        [nc addObserver:self
               selector:@selector(onPasteboardChanged:)
                   name:UIPasteboardChangedNotification
                 object:UIPasteboard.generalPasteboard];
        [nc addObserver:self
               selector:@selector(onPasteboardChanged:)
                   name:UIPasteboardRemovedNotification
                 object:UIPasteboard.generalPasteboard];
    }
    return self;
}

- (void)onDidBecomeActive:(NSNotification *)note {
    (void)note;
    WKWebView *wv = gCowboyWebView;
    if (wv == nil) return;
    cowboyRepairAuthenticationBrowserBridge(wv);
    [wv evaluateJavaScript:
            @"window.dispatchEvent(new Event('cowboy:native-resume'))"
         completionHandler:nil];
}

- (void)onPasteboardChanged:(NSNotification *)note {
    (void)note;
    WKWebView *wv = gCowboyWebView;
    if (wv == nil) return;
    [wv evaluateJavaScript:
            @"window.dispatchEvent(new Event('cowboy:clipboard-change'))"
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
    UIDeviceOrientation _lastDeviceOrientation;
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
        // Rotation invalidates both a pending keyboard notification frame and
        // any WebView height derived from the old interface coordinates. This
        // is especially visible on iPad: the first keyboard open after rotating
        // can otherwise leave the full-width keyboard covering the composer
        // until a second open delivers warm coordinates.
        [UIDevice.currentDevice beginGeneratingDeviceOrientationNotifications];
        [nc addObserver:self
               selector:@selector(onDeviceOrientationDidChange:)
                   name:UIDeviceOrientationDidChangeNotification
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
    // During the first keyboard presentation after an interface rotation,
    // UIKeyboardFrameEnd can briefly use the new full-width keyboard geometry
    // with an old-orientation bottom coordinate. Treat a substantial full-width
    // keyboard as docked as well. A truly floating iPad keyboard is narrow, so
    // it continues to overlay the document without resizing the WebView.
    CowboyKeyboardOverlapResult geometry =
        cowboyKeyboardOverlapForNotification((CowboyKeyboardOverlapInput) {
            .parentWidth = CGRectGetWidth(parent.bounds),
            .parentHeight = CGRectGetHeight(parent.bounds),
            .keyboardMinY = CGRectGetMinY(kbInParent),
            .keyboardMaxY = CGRectGetMaxY(kbInParent),
            .keyboardWidth = CGRectGetWidth(kbInParent),
            .keyboardHeight = CGRectGetHeight(kbInParent),
        });
    BOOL dockedToBottom =
        geometry.frameReachesBottom || geometry.fullWidthDockCandidate;
    // A full-width fallback exists specifically because the notification may
    // combine the new keyboard size with an old-orientation bottom coordinate.
    // The coordinate conversion can also swap the frame's axes. Bound both the
    // bottom intersection and fallback by the orientation-invariant short edge;
    // otherwise the old screen width becomes a false keyboard depth and leaves
    // most of the iPad as a blank region.
    CGFloat overlap = (CGFloat)geometry.overlap;
#if DEBUG
    NSLog(@"[cowboy] keyboard note frame=%@ parent=%@ docked=%d fullWidth=%d depth=%.1f overlap=%.1f",
          NSStringFromCGRect(kbInParent), NSStringFromCGRect(parent.bounds),
          dockedToBottom, geometry.fullWidthDockCandidate,
          geometry.frameDepth, overlap);
#endif
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
    // Follow the guide for a bounded two-second settling window. Four fixed
    // samples were enough for an ordinary first-open prediction, but an iPad
    // rotation can finish reconciling interface coordinates after the final
    // 700ms sample. Frequent reads are layout-only, generation-cancelled, and
    // stop after 2s; frame writes remain idempotent.
    for (NSUInteger step = 0; step <= 20; step++) {
        NSTimeInterval delay = (NSTimeInterval)step * 0.1;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW,
                          (int64_t)(delay * NSEC_PER_SEC)),
            dispatch_get_main_queue(), ^{
                [self applySettledLayoutGuide:generation];
            });
    }
}

- (void)onDeviceOrientationDidChange:(NSNotification *)note {
    (void)note;
    UIDeviceOrientation orientation = UIDevice.currentDevice.orientation;
    if (!UIDeviceOrientationIsPortrait(orientation) &&
        !UIDeviceOrientationIsLandscape(orientation)) {
        return;
    }
    if (orientation == _lastDeviceOrientation) return;
    _lastDeviceOrientation = orientation;
    ++_settleGeneration;
    WKWebView *wv = gCowboyWebView;
    UIView *parent = wv.superview;
    if (wv == nil || parent == nil || wv.window == nil) return;
    // Drop geometry owned by the previous orientation immediately. UIKit will
    // lay out the parent's new bounds independently; the next keyboard frame or
    // guide sample may then shrink only the current orientation.
    [UIView performWithoutAnimation:^{
        wv.frame = parent.bounds;
        [wv layoutIfNeeded];
    }];
    if (_keyboardVisible) {
        [self scheduleSettledCorrections];
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
