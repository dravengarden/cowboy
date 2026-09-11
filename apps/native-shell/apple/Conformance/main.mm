#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

// Exercise the production bridge in a real WKWebView. This app has no network
// inputs, credentials, relying-party allowlist, or Associated Domains entitlement.
@interface CowboyPluginConformance : UIResponder
    <UIApplicationDelegate, WKNavigationDelegate, WKScriptMessageHandler>
@property(nonatomic, strong) UIWindow *window;
@property(nonatomic, strong) WKWebView *webView;
@end

@implementation CowboyPluginConformance
- (BOOL)application:(__unused UIApplication *)application
    didFinishLaunchingWithOptions:(__unused NSDictionary *)options {
    self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
    UIViewController *controller = [[UIViewController alloc] init];
    self.window.rootViewController = controller;
    WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
    [configuration.userContentController addScriptMessageHandler:self name:@"conformance"];
    self.webView = [[WKWebView alloc] initWithFrame:self.window.bounds configuration:configuration];
    self.webView.navigationDelegate = self;
    [controller.view addSubview:self.webView];
    [self.window makeKeyAndVisible];
    [self.webView loadHTMLString:@"<!doctype html><title>Cowboy Plugin Conformance</title>"
                         baseURL:[NSURL URLWithString:@"https://cowboy-conformance.invalid/"]];
    return YES;
}

- (void)webView:(WKWebView *)webView didFinishNavigation:(__unused WKNavigation *)navigation {
    NSString *script =
        @"(async()=>{const tests=[];const check=(name,value)=>{if(!value)throw Error(name);tests.push(name)};"
         @"try{const host=window.__COWBOY_NATIVE_PLUGIN_HOST;"
         @"check('versioned immutable native ABI',host?.version==='1.0.0'&&Object.isFrozen(host));"
         @"check('native tweaks coexist with Passkey swizzle',window.__cowboyNativeShell===true&&typeof window.__cowboySelectionHaptic==='function'&&window.__cowboyAuthenticationBrowserBridgeVersion===2);"
         @"let clipboardDenied=false;try{await window.__cowboyReadClipboard()}catch{clipboardDenied=true}"
         @"check('foreign origin cannot read clipboard',clipboardDenied);"
         @"check('app icon bridge coexists',typeof window.__cowboyAppIcon==='function');"
         @"const iconState=await window.__cowboyAppIcon({action:'state'});"
         @"check('foreign origin cannot read app icon state',iconState.ok===false);"
         @"const iconSet=await window.__cowboyAppIcon({action:'set',id:'palette-054'});"
         @"check('foreign origin cannot change app icon',iconSet.ok===false);"
         @"check('closed immutable capabilities',Object.isFrozen(host.capabilities)&&JSON.stringify(host.capabilities)==='[\"webauthn\"]');"
         @"let denied=false;try{await host.invoke('shell',{})}catch{denied=true}"
         @"check('unknown capability rejected',denied);"
         @"const capabilities=await host.invoke('webauthn',{action:'capabilities',rp_id:'cowboy-conformance.invalid'});"
         @"check('unentitled app does not claim direct passkeys',capabilities.ok===true&&capabilities.available===false);"
         @"const assertion=await host.invoke('webauthn',{action:'assert',rp_id:'cowboy-conformance.invalid'});"
         @"check('unconfigured assertion fails closed',assertion.ok===false&&assertion.error.code==='not_configured');"
         @"const malformed=await host.invoke('webauthn',null);"
         @"check('malformed native request rejected',malformed.ok===false&&malformed.error.code==='invalid_request');"
         @"const browser=await window.__cowboyOpenPasskeyBrowser('https://foreign.invalid/');"
         @"check('foreign authentication browser URL rejected',browser.ok===false&&browser.error.code==='invalid_request');"
         @"const closed=await window.__cowboyClosePasskeyBrowser();"
         @"check('closing absent browser is idempotent',closed.ok===true);"
         @"window.webkit.messageHandlers.conformance.postMessage({ok:true,tests});"
         @"}catch(error){window.webkit.messageHandlers.conformance.postMessage({ok:false,tests,error:String(error)})}})();";
    [webView evaluateJavaScript:script completionHandler:nil];
}

- (void)userContentController:(__unused WKUserContentController *)controller
      didReceiveScriptMessage:(WKScriptMessage *)message {
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:message.body
                                                 options:NSJSONWritingPrettyPrinted
                                                   error:&error];
    if (data == nil) return;
    NSURL *documents = [NSFileManager.defaultManager URLsForDirectory:NSDocumentDirectory
                                                           inDomains:NSUserDomainMask].firstObject;
    [data writeToURL:[documents URLByAppendingPathComponent:@"conformance.json"]
             options:NSDataWritingAtomic error:&error];
}
@end

int main(int argc, char *argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass(CowboyPluginConformance.class));
    }
}
