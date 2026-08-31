#import <AuthenticationServices/AuthenticationServices.h>
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

// This bridge is deliberately ceremony-only. JavaScript obtains the challenge
// from Cowboy and submits the standard WebAuthn JSON result back to Cowboy;
// native code never receives a cookie, password, account role, or plugin state.

typedef void (^CowboyPasskeyReply)(id _Nullable, NSString *_Nullable);

static NSString *cowboyBase64Url(NSData *data) {
    NSString *encoded = [data base64EncodedStringWithOptions:0];
    encoded = [encoded stringByReplacingOccurrencesOfString:@"+" withString:@"-"];
    encoded = [encoded stringByReplacingOccurrencesOfString:@"/" withString:@"_"];
    return [encoded stringByTrimmingCharactersInSet:
        [NSCharacterSet characterSetWithCharactersInString:@"="]];
}

static NSData *cowboyDecodeBase64Url(id value) {
    if (![value isKindOfClass:NSString.class]) return nil;
    NSString *encoded = [(NSString *)value
        stringByReplacingOccurrencesOfString:@"-" withString:@"+"];
    encoded = [encoded stringByReplacingOccurrencesOfString:@"_" withString:@"/"];
    NSUInteger remainder = encoded.length % 4;
    if (remainder != 0) {
        encoded = [encoded stringByPaddingToLength:encoded.length + (4 - remainder)
                                        withString:@"="
                                   startingAtIndex:0];
    }
    return [[NSData alloc] initWithBase64EncodedString:encoded options:0];
}

static NSDictionary *cowboyFailure(NSString *code, NSString *message) {
    return @{
        @"ok": @NO,
        @"error": @{
            @"code": code ?: @"native_failure",
            @"message": message ?: @"Native Passkey verification failed.",
        },
    };
}

static BOOL cowboyConfiguredForRelyingParty(NSString *rpID) {
    if (rpID.length == 0) return NO;
    id configured = [NSBundle.mainBundle
        objectForInfoDictionaryKey:@"CowboyNativePasskeyRelyingPartyIdentifiers"];
    if (![configured isKindOfClass:NSArray.class]) return NO;
    for (id candidate in (NSArray *)configured) {
        if ([candidate isKindOfClass:NSString.class] &&
            [(NSString *)candidate isEqualToString:rpID]) return YES;
    }
    return NO;
}

static BOOL cowboyMessageMatchesRelyingParty(
    WKScriptMessage *message,
    NSString *rpID
) {
    WKSecurityOrigin *origin = message.frameInfo.securityOrigin;
    return [origin.protocol isEqualToString:@"https"] &&
        [origin.host isEqualToString:rpID];
}

static ASAuthorizationPublicKeyCredentialUserVerificationPreference
cowboyUserVerification(id value) API_AVAILABLE(ios(15.0)) {
    if ([value isEqual:@"discouraged"]) {
        return ASAuthorizationPublicKeyCredentialUserVerificationPreferenceDiscouraged;
    }
    if ([value isEqual:@"preferred"]) {
        return ASAuthorizationPublicKeyCredentialUserVerificationPreferencePreferred;
    }
    // Cowboy sessions require a fresh local user-verification signal. An
    // omitted or unknown preference therefore fails to the strongest mode.
    return ASAuthorizationPublicKeyCredentialUserVerificationPreferenceRequired;
}

static NSArray<ASAuthorizationPlatformPublicKeyCredentialDescriptor *> *
cowboyCredentialDescriptors(id value) API_AVAILABLE(ios(15.0)) {
    if (![value isKindOfClass:NSArray.class]) return @[];
    NSMutableArray *descriptors = [NSMutableArray array];
    for (id candidate in (NSArray *)value) {
        if (![candidate isKindOfClass:NSDictionary.class]) continue;
        NSData *credentialID = cowboyDecodeBase64Url(candidate[@"id"]);
        if (credentialID.length == 0) continue;
        [descriptors addObject:[[ASAuthorizationPlatformPublicKeyCredentialDescriptor alloc]
            initWithCredentialID:credentialID]];
    }
    return descriptors;
}

static UIWindow *cowboyPasskeyWindow(void) {
    for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
        if (![scene isKindOfClass:UIWindowScene.class] ||
            scene.activationState != UISceneActivationStateForegroundActive) continue;
        for (UIWindow *window in ((UIWindowScene *)scene).windows) {
            if (window.isKeyWindow) return window;
        }
    }
    return UIApplication.sharedApplication.windows.firstObject;
}

@class CowboyPasskeyHandler;

@interface CowboyPasskeyCoordinator : NSObject
    <ASAuthorizationControllerDelegate,
     ASAuthorizationControllerPresentationContextProviding>
@property(nonatomic, copy) NSString *action;
@property(nonatomic, copy) CowboyPasskeyReply reply;
@property(nonatomic, strong) ASAuthorizationController *controller;
@property(nonatomic, weak) CowboyPasskeyHandler *owner;
@end

@interface CowboyPasskeyHandler : NSObject <WKScriptMessageHandlerWithReply>
@property(nonatomic, strong) CowboyPasskeyCoordinator *active;
- (void)finishCoordinator:(CowboyPasskeyCoordinator *)coordinator
                    reply:(NSDictionary *)reply;
@end

// SideStore/free-team signatures cannot carry Cowboy's Associated Domains
// entitlement, so direct AuthenticationServices Passkeys correctly report as
// unavailable. The secure fallback still needs a native lifecycle owner:
// ASWebAuthenticationSession observes a fixed callback from Cowboy's external
// WebAuthn page and closes itself on success or cancellation, even while the
// initiating WKWebView is suspended underneath it.
@class CowboyPasskeyBrowserHandler;

@interface CowboyPasskeyBrowserCoordinator : NSObject
    <ASWebAuthenticationPresentationContextProviding>
@property(nonatomic, copy) CowboyPasskeyReply reply;
@property(nonatomic, strong) ASWebAuthenticationSession *session;
@property(nonatomic, weak) CowboyPasskeyBrowserHandler *owner;
@end

@interface CowboyPasskeyBrowserHandler : NSObject <WKScriptMessageHandlerWithReply>
@property(nonatomic, strong) CowboyPasskeyBrowserCoordinator *active;
- (void)finishCoordinator:(CowboyPasskeyBrowserCoordinator *)coordinator
               callbackURL:(NSURL *_Nullable)callbackURL
                     error:(NSError *_Nullable)error;
@end

static NSInteger cowboyDefaultPort(NSString *scheme) {
    if ([scheme caseInsensitiveCompare:@"https"] == NSOrderedSame) return 443;
    if ([scheme caseInsensitiveCompare:@"http"] == NSOrderedSame) return 80;
    return 0;
}

static BOOL cowboyExternalPasskeyURLAllowed(
    WKScriptMessage *message,
    NSURL *url
) {
    if (url == nil || !message.frameInfo.isMainFrame) return NO;
    WKSecurityOrigin *origin = message.frameInfo.securityOrigin;
    NSString *scheme = url.scheme.lowercaseString;
    NSString *originScheme = origin.protocol.lowercaseString;
    if (!([scheme isEqualToString:@"https"] || [scheme isEqualToString:@"http"]) ||
        ![scheme isEqualToString:originScheme] ||
        url.host.length == 0 || origin.host.length == 0 ||
        [url.host caseInsensitiveCompare:origin.host] != NSOrderedSame ||
        ![url.path isEqualToString:@"/passkey.html"] ||
        url.query.length != 0) return NO;

    NSInteger originPort = origin.port > 0
        ? origin.port
        : cowboyDefaultPort(originScheme);
    NSInteger urlPort = url.port != nil
        ? url.port.integerValue
        : cowboyDefaultPort(scheme);
    if (originPort != urlPort) return NO;

    NSURLComponents *fragment = [[NSURLComponents alloc] init];
    fragment.query = url.fragment;
    NSString *transaction = nil;
    NSString *callback = nil;
    NSArray<NSURLQueryItem *> *items = fragment.queryItems ?: @[];
    if (items.count != 2) return NO;
    for (NSURLQueryItem *item in items) {
        if ([item.name isEqualToString:@"transaction"]) transaction = item.value;
        if ([item.name isEqualToString:@"callback"]) callback = item.value;
    }
    if (transaction.length != 64 || ![callback isEqualToString:@"native"]) return NO;
    NSCharacterSet *nonHex =
        [[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdef"] invertedSet];
    return [transaction rangeOfCharacterFromSet:nonHex].location == NSNotFound;
}

static NSString *cowboyPasskeyBrowserCallbackStatus(NSURL *url) {
    if (url == nil ||
        ![url.scheme.lowercaseString isEqualToString:@"cowboy-passkey"] ||
        ![url.host.lowercaseString isEqualToString:@"complete"]) return nil;
    NSURLComponents *components = [NSURLComponents componentsWithURL:url
                                             resolvingAgainstBaseURL:NO];
    for (NSURLQueryItem *item in components.queryItems ?: @[]) {
        if (![item.name isEqualToString:@"status"]) continue;
        if ([item.value isEqualToString:@"complete"] ||
            [item.value isEqualToString:@"cancelled"] ||
            [item.value isEqualToString:@"failed"]) return item.value;
    }
    return nil;
}

@implementation CowboyPasskeyBrowserCoordinator

- (ASPresentationAnchor)presentationAnchorForWebAuthenticationSession:
    (__unused ASWebAuthenticationSession *)session {
    return cowboyPasskeyWindow();
}

@end


@implementation CowboyPasskeyBrowserHandler

- (void)finishCoordinator:(CowboyPasskeyBrowserCoordinator *)coordinator
               callbackURL:(NSURL *)callbackURL
                     error:(NSError *)error {
    if (coordinator == nil || coordinator != self.active) return;
    CowboyPasskeyReply reply = coordinator.reply;
    coordinator.reply = nil;
    coordinator.session = nil;
    self.active = nil;
    if (reply == nil) return;

    if ([error.domain isEqualToString:ASWebAuthenticationSessionErrorDomain] &&
        error.code == ASWebAuthenticationSessionErrorCodeCanceledLogin) {
        reply(@{ @"ok": @YES, @"status": @"cancelled" }, nil);
        return;
    }
    if (error != nil) {
        reply(cowboyFailure(
            @"browser_failure",
            @"The system Passkey browser could not complete the request."
        ), nil);
        return;
    }
    NSString *status = cowboyPasskeyBrowserCallbackStatus(callbackURL);
    if (status == nil) {
        reply(cowboyFailure(
            @"invalid_callback",
            @"The system Passkey browser returned an invalid callback."
        ), nil);
        return;
    }
    reply(@{ @"ok": @YES, @"status": status }, nil);
}

- (void)userContentController:(__unused WKUserContentController *)userContentController
      didReceiveScriptMessage:(WKScriptMessage *)message
                  replyHandler:(CowboyPasskeyReply)replyHandler {
    if (![message.body isKindOfClass:NSDictionary.class]) {
        replyHandler(cowboyFailure(
            @"invalid_request",
            @"Invalid native Passkey browser request."
        ), nil);
        return;
    }
    NSDictionary *body = message.body;
    NSString *action = [body[@"action"] isKindOfClass:NSString.class]
        ? body[@"action"] : @"";
    if ([action isEqualToString:@"close"]) {
        [self.active.session cancel];
        replyHandler(@{ @"ok": @YES }, nil);
        return;
    }
    NSString *rawURL = [body[@"url"] isKindOfClass:NSString.class]
        ? body[@"url"] : @"";
    NSURL *url = rawURL.length > 0 ? [NSURL URLWithString:rawURL] : nil;
    if (![action isEqualToString:@"open"] ||
        !cowboyExternalPasskeyURLAllowed(message, url)) {
        replyHandler(cowboyFailure(
            @"invalid_request",
            @"Cowboy rejected an invalid Passkey browser URL."
        ), nil);
        return;
    }
    if (self.active != nil) {
        replyHandler(cowboyFailure(
            @"busy",
            @"Another Passkey browser request is already active."
        ), nil);
        return;
    }

    CowboyPasskeyBrowserCoordinator *coordinator =
        [[CowboyPasskeyBrowserCoordinator alloc] init];
    coordinator.reply = replyHandler;
    coordinator.owner = self;
    __weak CowboyPasskeyBrowserHandler *weakSelf = self;
    coordinator.session = [[ASWebAuthenticationSession alloc]
        initWithURL:url
        callbackURLScheme:@"cowboy-passkey"
        completionHandler:^(NSURL *callbackURL, NSError *error) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [weakSelf finishCoordinator:coordinator
                                callbackURL:callbackURL
                                      error:error];
            });
        }];
    coordinator.session.presentationContextProvider = coordinator;
    // The handoff page is bound by an opaque, one-use transaction and does not
    // need browser cookies. Keep it isolated from the user's Safari session.
    coordinator.session.prefersEphemeralWebBrowserSession = YES;
    self.active = coordinator;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (![coordinator.session start]) {
            [weakSelf finishCoordinator:coordinator
                            callbackURL:nil
                                  error:[NSError
                                    errorWithDomain:ASWebAuthenticationSessionErrorDomain
                                               code:ASWebAuthenticationSessionErrorCodePresentationContextNotProvided
                                           userInfo:nil]];
        }
    });
}

@end


static CowboyPasskeyBrowserHandler *cowboyPasskeyBrowserHandler(void) {
    static CowboyPasskeyBrowserHandler *handler;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ handler = [[CowboyPasskeyBrowserHandler alloc] init]; });
    return handler;
}

@implementation CowboyPasskeyCoordinator

- (ASPresentationAnchor)presentationAnchorForAuthorizationController:
    (__unused ASAuthorizationController *)controller {
    return cowboyPasskeyWindow();
}

- (void)authorizationController:(__unused ASAuthorizationController *)controller
    didCompleteWithAuthorization:(ASAuthorization *)authorization {
    id credential = authorization.credential;
    NSDictionary *response = nil;
    NSData *credentialID = nil;

    if (@available(iOS 15.0, *)) {
        if ([self.action isEqualToString:@"create"] &&
            [credential isKindOfClass:ASAuthorizationPlatformPublicKeyCredentialRegistration.class]) {
            id<ASAuthorizationPublicKeyCredentialRegistration> registration = credential;
            if (registration.rawAttestationObject != nil) {
                credentialID = registration.credentialID;
                response = @{
                    @"clientDataJSON": cowboyBase64Url(registration.rawClientDataJSON),
                    @"attestationObject": cowboyBase64Url(registration.rawAttestationObject),
                };
            }
        } else if ([self.action isEqualToString:@"assert"] &&
                   [credential isKindOfClass:ASAuthorizationPlatformPublicKeyCredentialAssertion.class]) {
            id<ASAuthorizationPublicKeyCredentialAssertion> assertion = credential;
            credentialID = assertion.credentialID;
            response = @{
                @"clientDataJSON": cowboyBase64Url(assertion.rawClientDataJSON),
                @"authenticatorData": cowboyBase64Url(assertion.rawAuthenticatorData),
                @"signature": cowboyBase64Url(assertion.signature),
                @"userHandle": assertion.userID.length > 0
                    ? cowboyBase64Url(assertion.userID)
                    : NSNull.null,
            };
        }
    }

    if (credentialID.length == 0 || response == nil) {
        [self.owner finishCoordinator:self reply:cowboyFailure(
            @"invalid_response",
            @"The system returned an invalid Passkey credential."
        )];
        return;
    }
    NSString *identifier = cowboyBase64Url(credentialID);
    [self.owner finishCoordinator:self reply:@{
        @"ok": @YES,
        @"credential": @{
            @"id": identifier,
            @"rawId": identifier,
            @"type": @"public-key",
            @"response": response,
            @"clientExtensionResults": @{},
        },
    }];
}

- (void)authorizationController:(__unused ASAuthorizationController *)controller
    didCompleteWithError:(NSError *)error {
    NSString *code = @"native_failure";
    NSString *message = @"Native Passkey verification failed.";
    BOOL notInteractive = NO;
    if (@available(iOS 15.0, *)) {
        notInteractive = error.code == ASAuthorizationErrorNotInteractive;
    }
    if ([error.domain isEqualToString:ASAuthorizationErrorDomain]) {
        if (error.code == ASAuthorizationErrorCanceled) {
            code = @"cancelled";
            message = @"Passkey verification was cancelled.";
        } else if (error.code == ASAuthorizationErrorNotHandled || notInteractive) {
            code = @"not_configured";
            message = @"This app signature is not configured for Cowboy Passkeys.";
        } else if (error.code == ASAuthorizationErrorInvalidResponse) {
            code = @"invalid_response";
            message = @"The system returned an invalid Passkey response.";
        }
    }
    [self.owner finishCoordinator:self reply:cowboyFailure(code, message)];
}

@end


@implementation CowboyPasskeyHandler

- (void)finishCoordinator:(CowboyPasskeyCoordinator *)coordinator
                    reply:(NSDictionary *)reply {
    if (coordinator == nil || coordinator != self.active) return;
    CowboyPasskeyReply callback = coordinator.reply;
    coordinator.reply = nil;
    coordinator.controller = nil;
    self.active = nil;
    if (callback != nil) callback(reply, nil);
}

- (void)userContentController:(__unused WKUserContentController *)userContentController
      didReceiveScriptMessage:(WKScriptMessage *)message
                  replyHandler:(CowboyPasskeyReply)replyHandler {
    if (![message.body isKindOfClass:NSDictionary.class]) {
        replyHandler(cowboyFailure(@"invalid_request", @"Invalid native Passkey request."), nil);
        return;
    }
    NSDictionary *body = message.body;
    NSString *action = [body[@"action"] isKindOfClass:NSString.class]
        ? body[@"action"] : @"";
    NSString *rpID = [body[@"rp_id"] isKindOfClass:NSString.class]
        ? body[@"rp_id"] : @"";
    BOOL configured = cowboyConfiguredForRelyingParty(rpID) &&
        cowboyMessageMatchesRelyingParty(message, rpID);

    if ([action isEqualToString:@"capabilities"]) {
        BOOL available = NO;
        if (@available(iOS 15.0, *)) available = configured;
        replyHandler(@{ @"ok": @YES, @"available": @(available) }, nil);
        return;
    }
    if (!configured) {
        replyHandler(cowboyFailure(
            @"not_configured",
            @"This app signature is not configured for Cowboy Passkeys."
        ), nil);
        return;
    }
    if (@available(iOS 15.0, *)) {
        if (self.active != nil) {
            replyHandler(cowboyFailure(
                @"busy",
                @"Another Passkey request is already active."
            ), nil);
            return;
        }
        NSDictionary *options = [body[@"public_key"] isKindOfClass:NSDictionary.class]
            ? body[@"public_key"] : nil;
        NSData *challenge = cowboyDecodeBase64Url(options[@"challenge"]);
        if (options == nil || challenge.length == 0) {
            replyHandler(cowboyFailure(
                @"invalid_request",
                @"Cowboy provided an invalid Passkey challenge."
            ), nil);
            return;
        }

        ASAuthorizationPlatformPublicKeyCredentialProvider *provider =
            [[ASAuthorizationPlatformPublicKeyCredentialProvider alloc]
                initWithRelyingPartyIdentifier:rpID];
        ASAuthorizationRequest *request = nil;
        if ([action isEqualToString:@"create"]) {
            NSDictionary *user = [options[@"user"] isKindOfClass:NSDictionary.class]
                ? options[@"user"] : nil;
            NSString *name = [user[@"name"] isKindOfClass:NSString.class]
                ? user[@"name"] : nil;
            NSData *userID = cowboyDecodeBase64Url(user[@"id"]);
            if (name.length == 0 || userID.length == 0) {
                replyHandler(cowboyFailure(
                    @"invalid_request",
                    @"Cowboy provided invalid Passkey user data."
                ), nil);
                return;
            }
            ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *registration =
                [provider createCredentialRegistrationRequestWithChallenge:challenge
                                                                       name:name
                                                                     userID:userID];
            if ([user[@"displayName"] isKindOfClass:NSString.class]) {
                registration.displayName = user[@"displayName"];
            }
            registration.userVerificationPreference = cowboyUserVerification(
                options[@"authenticatorSelection"][@"userVerification"]
            );
            registration.attestationPreference =
                ASAuthorizationPublicKeyCredentialAttestationKindNone;
            if (@available(iOS 17.4, *)) {
                registration.excludedCredentials = cowboyCredentialDescriptors(
                    options[@"excludeCredentials"]
                );
            }
            request = registration;
        } else if ([action isEqualToString:@"assert"]) {
            ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *assertion =
                [provider createCredentialAssertionRequestWithChallenge:challenge];
            assertion.userVerificationPreference = cowboyUserVerification(
                options[@"userVerification"]
            );
            assertion.allowedCredentials = cowboyCredentialDescriptors(
                options[@"allowCredentials"]
            );
            request = assertion;
        } else {
            replyHandler(cowboyFailure(@"invalid_request", @"Unknown Passkey action."), nil);
            return;
        }

        CowboyPasskeyCoordinator *coordinator = [[CowboyPasskeyCoordinator alloc] init];
        coordinator.action = action;
        coordinator.reply = replyHandler;
        coordinator.owner = self;
        coordinator.controller = [[ASAuthorizationController alloc]
            initWithAuthorizationRequests:@[request]];
        coordinator.controller.delegate = coordinator;
        coordinator.controller.presentationContextProvider = coordinator;
        self.active = coordinator;
        dispatch_async(dispatch_get_main_queue(), ^{
            [coordinator.controller performRequests];
        });
        return;
    }
    replyHandler(cowboyFailure(
        @"unsupported_os",
        @"This version of iOS does not support native Passkeys."
    ), nil);
}

@end


static CowboyPasskeyHandler *cowboyPasskeyHandler(void) {
    static CowboyPasskeyHandler *handler;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ handler = [[CowboyPasskeyHandler alloc] init]; });
    return handler;
}

__attribute__((constructor)) static void cowboyInstallPasskeyBridge(void) {
    @autoreleasepool {
        Class cls = WKWebView.class;
        SEL selector = @selector(initWithFrame:configuration:);
        Method method = class_getInstanceMethod(cls, selector);
        if (method == nil) return;
        IMP predecessor = method_getImplementation(method);
        IMP replacement = imp_implementationWithBlock(
            ^WKWebView *(id receiver, CGRect frame, WKWebViewConfiguration *configuration) {
                @try {
                    WKUserContentController *controller =
                        configuration.userContentController;
                    if (controller == nil) {
                        controller = [[WKUserContentController alloc] init];
                        configuration.userContentController = controller;
                    }
                    if (@available(iOS 14.0, *)) {
                        [controller addScriptMessageHandlerWithReply:cowboyPasskeyHandler()
                                                         contentWorld:WKContentWorld.pageWorld
                                                                 name:@"cowboyPasskey"];
                        [controller addScriptMessageHandlerWithReply:
                            cowboyPasskeyBrowserHandler()
                                                         contentWorld:WKContentWorld.pageWorld
                                                                 name:@"cowboyPasskeyBrowser"];
                        NSString *source =
                            @"window.__cowboyNativePasskeyBridgeVersion=1;"
                             @"window.__cowboyNativePasskey=function(request){try{"
                             @"return window.webkit.messageHandlers.cowboyPasskey.postMessage(request)"
                             @"}catch(error){return Promise.reject(error)}};"
                             @"window.__cowboyPasskeyBrowserBridgeVersion=1;"
                             @"window.__cowboyOpenPasskeyBrowser=function(url){try{"
                             @"return window.webkit.messageHandlers.cowboyPasskeyBrowser."
                             @"postMessage({action:'open',url:url})"
                             @"}catch(error){return Promise.reject(error)}};"
                             @"window.__cowboyClosePasskeyBrowser=function(){try{"
                             @"return window.webkit.messageHandlers.cowboyPasskeyBrowser."
                             @"postMessage({action:'close'})"
                             @"}catch(error){return Promise.reject(error)}};";
                        [controller addUserScript:[[WKUserScript alloc]
                            initWithSource:source
                            injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                            forMainFrameOnly:YES]];
                    }
                } @catch (__unused NSException *exception) {
                }
                return ((WKWebView *(*)(id, SEL, CGRect, WKWebViewConfiguration *))predecessor)(
                    receiver,
                    selector,
                    frame,
                    configuration
                );
            }
        );
        method_setImplementation(method, replacement);
    }
}
