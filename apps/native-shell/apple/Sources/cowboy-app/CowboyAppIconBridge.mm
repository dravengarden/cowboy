// Core product appearance, deliberately separate from the Plugin capability ABI.
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

static NSString *const CowboyDefaultIcon = @"palette-103";

static BOOL cowboyIconTrustedMessage(WKScriptMessage *message) {
    WKSecurityOrigin *origin = message.frameInfo.securityOrigin;
    return message.frameInfo.isMainFrame &&
        [origin.protocol isEqualToString:@"https"] &&
        [origin.host isEqualToString:@"cowboy.stormbird.xyz"] &&
        (origin.port == 0 || origin.port == 443);
}

static NSDictionary *cowboyBundledAlternateIcons(void) {
    NSDictionary *info = NSBundle.mainBundle.infoDictionary;
    NSDictionary *icons = info[@"CFBundleIcons"];
    if (UIDevice.currentDevice.userInterfaceIdiom == UIUserInterfaceIdiomPad &&
        [info[@"CFBundleIcons~ipad"] isKindOfClass:NSDictionary.class]) {
        icons = info[@"CFBundleIcons~ipad"];
    }
    id alternates = icons[@"CFBundleAlternateIcons"];
    return [alternates isKindOfClass:NSDictionary.class] ? alternates : @{};
}

static NSDictionary *cowboyAppIconState(void) {
    UIApplication *app = UIApplication.sharedApplication;
    NSMutableSet *available = [NSMutableSet setWithObject:CowboyDefaultIcon];
    for (NSString *name in cowboyBundledAlternateIcons()) {
        if ([name hasPrefix:@"Cowboy-"]) [available addObject:[name substringFromIndex:7]];
    }
    NSString *name = app.alternateIconName;
    NSString *current = [name hasPrefix:@"Cowboy-"] ? [name substringFromIndex:7] : CowboyDefaultIcon;
    return @{@"ok":@YES, @"supported":@(app.supportsAlternateIcons),
             @"current":current, @"available":[[available allObjects] sortedArrayUsingSelector:@selector(compare:)]};
}

@interface CowboyAppIconHandler : NSObject <WKScriptMessageHandlerWithReply>
@property(nonatomic) BOOL applying;
@end

@implementation CowboyAppIconHandler
- (void)userContentController:(__unused WKUserContentController *)controller
      didReceiveScriptMessage:(WKScriptMessage *)message
                 replyHandler:(void (^)(id, NSString *))reply {
    if (!cowboyIconTrustedMessage(message)) {
        reply(@{@"ok":@NO, @"error":@"App icon requests require the trusted Cowboy main frame."}, nil);
        return;
    }
    if (![message.body isKindOfClass:NSDictionary.class]) {
        reply(@{@"ok":@NO, @"error":@"Invalid app icon request."}, nil);
        return;
    }
    NSDictionary *body = message.body;
    if ([body[@"action"] isEqual:@"state"]) { reply(cowboyAppIconState(), nil); return; }
    NSString *identifier = body[@"id"];
    if (![body[@"action"] isEqual:@"set"] || ![identifier isKindOfClass:NSString.class] || identifier.length > 64) {
        reply(@{@"ok":@NO, @"error":@"Invalid app icon request."}, nil);
        return;
    }
    NSString *name = [identifier isEqual:CowboyDefaultIcon] ? nil : [@"Cowboy-" stringByAppendingString:identifier];
    if (name != nil && cowboyBundledAlternateIcons()[name] == nil) {
        reply(@{@"ok":@NO, @"error":@"This icon is not bundled. Update the native Cowboy app."}, nil);
        return;
    }
    UIApplication *app = UIApplication.sharedApplication;
    if (!app.supportsAlternateIcons || app.applicationState != UIApplicationStateActive || self.applying) {
        reply(@{@"ok":@NO, @"error":@"Icon switching is unavailable. Keep Cowboy open and try again."}, nil);
        return;
    }
    if ((name == nil && app.alternateIconName == nil) || [app.alternateIconName isEqual:name]) {
        reply(cowboyAppIconState(), nil);
        return;
    }
    self.applying = YES;
    [app setAlternateIconName:name completionHandler:^(NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            self.applying = NO;
            reply(error ? @{@"ok":@NO, @"error":error.localizedDescription} : cowboyAppIconState(), nil);
        });
    }];
}
@end

__attribute__((constructor)) static void cowboyInstallAppIconBridge(void) {
    @autoreleasepool {
        Method method = class_getInstanceMethod(WKWebView.class, @selector(initWithFrame:configuration:));
        if (method == nil) return;
        IMP predecessor = method_getImplementation(method);
        CowboyAppIconHandler *handler = [[CowboyAppIconHandler alloc] init];
        IMP replacement = imp_implementationWithBlock(^WKWebView *(id receiver, CGRect frame, WKWebViewConfiguration *configuration) {
            @try {
                WKUserContentController *controller = configuration.userContentController;
                [controller addScriptMessageHandlerWithReply:handler contentWorld:WKContentWorld.pageWorld name:@"cowboyAppIcon"];
                NSString *source = @"Object.defineProperty(window,'__cowboyAppIcon',{value:function(request){"
                    @"return window.webkit.messageHandlers.cowboyAppIcon.postMessage(request)}});";
                [controller addUserScript:[[WKUserScript alloc] initWithSource:source injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES]];
            } @catch (__unused NSException *exception) { }
            return ((WKWebView *(*)(id, SEL, CGRect, WKWebViewConfiguration *))predecessor)(receiver, @selector(initWithFrame:configuration:), frame, configuration);
        });
        method_setImplementation(method, replacement);
    }
}
