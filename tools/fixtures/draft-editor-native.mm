// Disposable WKWebView runner for the real shared editor's native input matrix.
// Source is exchanged through Git. No real account or app container is used.
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
@interface DraftInputApp : UIResponder <UIApplicationDelegate, WKScriptMessageHandler>
@property(nonatomic,strong) UIWindow *window;
@end
@implementation DraftInputApp
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)options {
  WKWebViewConfiguration *config = [WKWebViewConfiguration new];
  [config.userContentController addScriptMessageHandler:self name:@"report"];
  [config.userContentController addScriptMessageHandler:self name:@"clipboard"];
  UIViewController *controller = [UIViewController new];
  WKWebView *web = [[WKWebView alloc] initWithFrame:UIScreen.mainScreen.bounds configuration:config];
  web.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
  controller.view = web;
  self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
  self.window.rootViewController = controller; [self.window makeKeyAndVisible];
  NSString *url = NSProcessInfo.processInfo.environment[@"COWBOY_EDITOR_FIXTURE_URL"];
  if (![url hasPrefix:@"http://127.0.0.1:"]) abort();
  [web loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:url]]];
  return YES;
}
- (void)userContentController:(WKUserContentController *)controller didReceiveScriptMessage:(WKScriptMessage *)message {
  if ([message.name isEqualToString:@"clipboard"]) {
    if ([message.body isEqual:@"text"]) UIPasteboard.generalPasteboard.string = @"粘贴中文 clipboard text";
    else {
      UIGraphicsBeginImageContextWithOptions(CGSizeMake(64, 48), NO, 1);
      [UIColor.systemBlueColor setFill]; UIRectFill(CGRectMake(0, 0, 64, 48));
      UIPasteboard.generalPasteboard.image = UIGraphicsGetImageFromCurrentImageContext(); UIGraphicsEndImageContext();
    }
    return;
  }
  NSData *data = [NSJSONSerialization dataWithJSONObject:message.body options:NSJSONWritingPrettyPrinted error:nil];
  NSString *directory = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES).firstObject;
  [data writeToFile:[directory stringByAppendingPathComponent:@"input.json"] atomically:YES];
}
@end
int main(int argc, char **argv) { @autoreleasepool { return UIApplicationMain(argc, argv, nil, NSStringFromClass(DraftInputApp.class)); } }
