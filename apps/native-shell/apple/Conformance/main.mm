#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

// Exercise the production bridge in a real WKWebView. This app has no network
// inputs, credentials, relying-party allowlist, or Associated Domains entitlement.
@interface CowboyPluginConformance : UIResponder
    <UIApplicationDelegate, UIWindowSceneDelegate, WKNavigationDelegate, WKScriptMessageHandler>
@property(nonatomic, strong) UIWindow *window;
@property(nonatomic, strong) WKWebView *webView;
@property(nonatomic, strong) NSTimer *rootSampler;
@property(nonatomic) CGFloat rootMaxDrift;
@property(nonatomic) NSUInteger rootSamples;
@property(nonatomic, strong) NSDictionary *rootChecks;
@end

@implementation CowboyPluginConformance
- (BOOL)application:(__unused UIApplication *)application
    didFinishLaunchingWithOptions:(__unused NSDictionary *)options {
    return YES;
}

- (UISceneConfiguration *)application:(__unused UIApplication *)application
    configurationForConnectingSceneSession:(UISceneSession *)session
    options:(__unused UISceneConnectionOptions *)options {
    UISceneConfiguration *configuration =
        [[UISceneConfiguration alloc] initWithName:@"Conformance" sessionRole:session.role];
    configuration.delegateClass = CowboyPluginConformance.class;
    return configuration;
}

- (void)scene:(UIScene *)scene willConnectToSession:(__unused UISceneSession *)session
    options:(__unused UISceneConnectionOptions *)options {
    self.window = [[UIWindow alloc] initWithWindowScene:(UIWindowScene *)scene];
    UIViewController *controller = [[UIViewController alloc] init];
    self.window.rootViewController = controller;
    WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
    [configuration.userContentController addScriptMessageHandler:self name:@"conformance"];
    self.webView = [[WKWebView alloc] initWithFrame:self.window.bounds configuration:configuration];
    self.webView.navigationDelegate = self;
    [controller.view addSubview:self.webView];
    [self.window makeKeyAndVisible];
    [self.webView loadHTMLString:@"<!doctype html><meta name='viewport' content='width=device-width,initial-scale=1,viewport-fit=cover'><title>Cowboy Plugin Conformance</title>"
                         baseURL:[NSURL URLWithString:@"https://cowboy-conformance.invalid/"]];
}

- (void)webView:(WKWebView *)webView didFinishNavigation:(__unused WKNavigation *)navigation {
    UIScrollView *root = webView.scrollView;
    CGPoint rest = CGPointMake(-root.adjustedContentInset.left, -root.adjustedContentInset.top);
    root.contentOffset = CGPointMake(0, 335);
    CGPoint directOffset = root.contentOffset;
    BOOL rootPinned = CGPointEqualToPoint(root.contentOffset, rest);
    [root setContentOffset:CGPointMake(0, -228) animated:YES];
    UIScrollView *child = [[UIScrollView alloc] initWithFrame:CGRectMake(0, 0, 100, 100)];
    child.contentSize = CGSizeMake(100, 1000);
    child.contentOffset = CGPointMake(0, 335);
    BOOL childScrolls = child.contentOffset.y == 335;
    NSString *script =
        @"(async()=>{const tests=[];const check=(name,value)=>{if(!value)throw Error(name);tests.push(name)};"
         @"try{const host=window.__COWBOY_NATIVE_PLUGIN_HOST;"
         @"check('versioned immutable native ABI',host?.version==='1.0.0'&&Object.isFrozen(host));"
         @"check('native root rejects focus reveal while child scrollers retain offsets',window.__nativeRootScrollChecks===true);"
         @"check('native tweaks coexist with Passkey swizzle',window.__cowboyNativeShell===true&&typeof window.__cowboySelectionHaptic==='function'&&window.__cowboyAuthenticationBrowserBridgeVersion===2);"
         @"let clipboardDenied=false;try{await window.__cowboyReadClipboard()}catch{clipboardDenied=true}"
         @"check('foreign origin cannot read clipboard',clipboardDenied);"
         @"check('app icon bridge coexists',typeof window.__cowboyAppIcon==='function');"
         @"check('installed bundle version beats compile-time version',window.__cowboyNativeApp?.version==='9.8.7'&&window.__cowboyNativeApp?.build==='987'&&Object.isFrozen(window.__cowboyNativeApp));"
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
         @"const initialHeight=innerHeight;"
         @"document.documentElement.style.cssText='height:100%;overflow:hidden';"
         @"document.body.style.cssText='height:100%;overflow:hidden;margin:0;display:flex;flex-direction:column';"
         @"document.body.innerHTML='<div id=nav style=\"height:70px;flex-shrink:0\">Navigation</div><div id=flow style=\"flex:1;min-height:0;overflow:auto\"><div id=editor contenteditable=true style=\"white-space:pre-wrap\"></div></div><div id=bar style=\"height:50px;flex-shrink:0\">Formatting</div>';"
         @"const editor=document.getElementById('editor'),flow=document.getElementById('flow');"
         @"editor.textContent=Array.from({length:80},(_,i)=>'Editable line '+i).join('\\n');"
         @"const range=document.createRange();range.selectNodeContents(editor);range.collapse(false);"
         @"const selection=getSelection();selection.removeAllRanges();selection.addRange(range);"
         @"const offsets=[];let recording=true;const sample=()=>{offsets.push(scrollY);if(recording)requestAnimationFrame(sample)};sample();"
         @"window.webkit.messageHandlers.conformance.postMessage({event:'focus-ready'});"
         @"await new Promise(resolve=>setTimeout(resolve,1200));recording=false;"
         @"check('keyboard reveals the caret in the child without panning the root',"
         @"innerHeight<initialHeight-100&&offsets.length>10&&offsets.every(y=>Math.abs(y)<1)&&flow.scrollTop>0&&document.activeElement===editor&&selection.anchorNode===editor.firstChild&&selection.anchorOffset===editor.textContent.length);"
         @"window.webkit.messageHandlers.conformance.postMessage({ok:true,tests});"
         @"}catch(error){window.webkit.messageHandlers.conformance.postMessage({ok:false,tests,error:String(error)})}})();";
    // Read after the animation can advance, not synchronously at its origin.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 600 * NSEC_PER_MSEC),
                   dispatch_get_main_queue(), ^{
        BOOL settled = CGPointEqualToPoint(root.contentOffset, rest);
        self.rootChecks = @{
            @"class": NSStringFromClass(root.class), @"zoom": @(root.zoomScale),
            @"direct": @(rootPinned), @"child": @(childScrolls), @"animated": @(settled),
            @"direct_y": @(directOffset.y), @"rest_y": @(rest.y),
            @"settled_y": @(root.contentOffset.y)
        };
        NSString *checks = rootPinned && childScrolls && settled ? @"true" : @"false";
        [webView evaluateJavaScript:[@"window.__nativeRootScrollChecks=" stringByAppendingString:checks]
                 completionHandler:nil];
        [webView evaluateJavaScript:script completionHandler:nil];
    });
}

- (void)userContentController:(__unused WKUserContentController *)controller
      didReceiveScriptMessage:(WKScriptMessage *)message {
    if ([message.body isKindOfClass:NSDictionary.class] &&
        [message.body[@"event"] isEqualToString:@"focus-ready"]) {
        // A fresh native evaluation supplies the focus gesture after the ABI
        // checks' asynchronous promises, like the simulator inspection bridge.
        self.rootMaxDrift = 0;
        self.rootSamples = 0;
        self.rootSampler = [NSTimer scheduledTimerWithTimeInterval:1.0 / 60.0
            repeats:YES block:^(__unused NSTimer *timer) {
                UIScrollView *root = self.webView.scrollView;
                CGFloat drift = MAX(fabs(root.contentOffset.x + root.adjustedContentInset.left),
                                    fabs(root.contentOffset.y + root.adjustedContentInset.top));
                self.rootMaxDrift = MAX(self.rootMaxDrift, drift);
                self.rootSamples++;
            }];
        [self.webView evaluateJavaScript:@"document.getElementById('editor').focus()"
                      completionHandler:nil];
        return;
    }
    [self.rootSampler invalidate];
    self.rootSampler = nil;
    NSMutableDictionary *report = [message.body mutableCopy];
    report[@"native_root_max_drift"] = @(self.rootMaxDrift);
    report[@"native_root_samples"] = @(self.rootSamples);
    report[@"native_offset_checks"] = self.rootChecks;
    if ([report[@"ok"] boolValue] && (self.rootMaxDrift > 1 || self.rootSamples < 10)) {
        report[@"ok"] = @NO;
        report[@"error"] = @"Native root moved during keyboard animation or sampling did not run";
    }
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:report
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
