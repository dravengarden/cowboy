// Disposable WKWebView runner for the real shared editor's native input matrix.
// Source is exchanged through Git. No real account or app container is used.
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
@interface DraftInputScene : UIResponder <UIWindowSceneDelegate, WKScriptMessageHandler>
@property(nonatomic,strong) UIWindow *window;
@end
@implementation DraftInputScene
- (void)scene:(UIScene *)scene willConnectToSession:(UISceneSession *)session options:(UISceneConnectionOptions *)options {
  WKWebViewConfiguration *config = [WKWebViewConfiguration new];
  [config.userContentController addScriptMessageHandler:self name:@"report"];
  [config.userContentController addScriptMessageHandler:self name:@"clipboard"];
  UIViewController *controller = [UIViewController new];
  WKWebView *web = [[WKWebView alloc] initWithFrame:UIScreen.mainScreen.bounds configuration:config];
  web.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
  controller.view = web;
  self.window = [[UIWindow alloc] initWithWindowScene:(UIWindowScene *)scene];
  self.window.rootViewController = controller; [self.window makeKeyAndVisible];
  NSString *url = NSProcessInfo.processInfo.environment[@"COWBOY_EDITOR_FIXTURE_URL"];
  if (![url hasPrefix:@"http://127.0.0.1:"]) abort();
  [web loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:url]]];
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
@interface DraftInputApp : UIResponder <UIApplicationDelegate>
@end
@implementation DraftInputApp
- (UISceneConfiguration *)application:(UIApplication *)application configurationForConnectingSceneSession:(UISceneSession *)session options:(UISceneConnectionOptions *)options {
  UISceneConfiguration *config = [[UISceneConfiguration alloc] initWithName:@"DraftInput" sessionRole:session.role];
  config.delegateClass = DraftInputScene.class; return config;
}
@end
int main(int argc, char **argv) { @autoreleasepool { return UIApplicationMain(argc, argv, nil, NSStringFromClass(DraftInputApp.class)); } }
