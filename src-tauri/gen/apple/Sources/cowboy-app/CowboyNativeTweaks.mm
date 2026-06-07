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
