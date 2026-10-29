#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import <objc/runtime.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

@class SPUUpdater;
@class SUAppcastItem;

// Sparkle 2.10's public Objective-C ABI, resolved only after loading the framework.
// These declarations introduce no Sparkle class or symbol references to the linker.
@interface SPUStandardUpdaterController : NSObject
- (instancetype)initWithStartingUpdater:(BOOL)startUpdater
                       updaterDelegate:(nullable id)updaterDelegate
                    userDriverDelegate:(nullable id)userDriverDelegate;
@property (nonatomic, readonly) SPUUpdater *updater;
- (void)checkForUpdates:(nullable id)sender;
@end

@interface SPUUpdater : NSObject
@property (nonatomic) BOOL automaticallyChecksForUpdates;
@property (nonatomic) BOOL automaticallyDownloadsUpdates;
@property (nonatomic) BOOL sendsSystemProfile;
@property (nonatomic, readonly) BOOL canCheckForUpdates;
@property (nonatomic, readonly) BOOL sessionInProgress;
- (BOOL)startUpdater:(NSError * __autoreleasing _Nullable *)error;
@end

@interface SUAppcastItem : NSObject
@property (nonatomic, readonly, nullable) NSString *displayVersionString;
@end

@class MTUpdaterDelegate;

static NSBundle *gSparkleBundle;
static SPUStandardUpdaterController *gUpdaterController;
static MTUpdaterDelegate *gUpdaterDelegate;
static SPUUpdater *gUpdater;
static NSString *gState = @"unavailable";
static NSString *gMessage = @"Updater has not been initialized.";
static NSString *gVersion;
// Sparkle objects and installation state are main-thread-only. The status snapshot
// uses this class lock so Rust may read it without entering Sparkle off-thread.
static BOOL gEnabled;
static BOOL gPermanentlyDisabled;
static BOOL gCanCheck;
static BOOL gPostponementObserved;
static BOOL gInstallApproved;
static BOOL gResumeQueued;
static BOOL gRelaunchBlocked;
static BOOL gInstallStarted;
static BOOL gTerminationCallbackActive;
static bool (*gTerminationHandler)(void);
static NSError *gNoUpdateError;
static void (^gPendingInstall)(void);

// Sparkle's installer sends a normal AppKit quit event, potentially long after
// its continuation or via its own retry handler. GPUI's quit observers are not
// invoked by that event, so gate every actual termination once an install starts.
static NSApplicationTerminateReply MTApplicationShouldTerminate(id delegate, SEL selector, NSApplication *application) {
    (void)delegate;
    (void)selector;
    (void)application;
    bool (*handler)(void);
    @synchronized ([NSObject class]) {
        if (!gInstallStarted) return NSTerminateNow;
        if (!NSThread.isMainThread || gTerminationCallbackActive || gTerminationHandler == NULL) return NSTerminateCancel;
        handler = gTerminationHandler;
        gTerminationCallbackActive = YES;
    }
    // No native lock crosses into Rust. The handler catches panic and reentrant
    // GPUI borrows, returning false if local terminal cleanup cannot complete.
    bool allowed = handler();
    @synchronized ([NSObject class]) {
        gTerminationCallbackActive = NO;
        if (!allowed) {
            gState = @"error";
            gMessage = @"Termination was cancelled because terminal cleanup did not complete. Retry the installation when the application is ready.";
            gCanCheck = !gRelaunchBlocked && gEnabled && gUpdater.canCheckForUpdates;
        }
    }
    return allowed ? NSTerminateNow : NSTerminateCancel;
}

static BOOL MTInstallTerminationGuard(NSApplication *application) {
    id<NSApplicationDelegate> delegate = application.delegate;
    if (application == nil || delegate == nil) return NO;
    Class delegateClass = object_getClass(delegate);
    SEL selector = @selector(applicationShouldTerminate:);
    Method existingMethod = class_getInstanceMethod(delegateClass, selector);
    if (existingMethod != NULL) {
        return method_getImplementation(existingMethod) == (IMP)MTApplicationShouldTerminate;
    }
    // Do not replace a host delegate's implementation or forwarding behavior.
    if ([delegate respondsToSelector:selector]) return NO;
    struct objc_method_description signature = protocol_getMethodDescription(@protocol(NSApplicationDelegate), selector, NO, YES);
    if (signature.types == NULL || !class_addMethod(delegateClass, selector, (IMP)MTApplicationShouldTerminate, signature.types)) return NO;
    // AppKit can cache which optional delegate methods are implemented.
    application.delegate = nil;
    application.delegate = delegate;
    return YES;
}

static BOOL MTTerminationGuardIsActive(void) {
    id delegate = NSApp.delegate;
    Method method = delegate == nil ? NULL : class_getInstanceMethod(object_getClass(delegate), @selector(applicationShouldTerminate:));
    return method != NULL && method_getImplementation(method) == (IMP)MTApplicationShouldTerminate;
}

static void MTSetStatus(NSString *state, NSString *message, NSString * _Nullable version, BOOL canCheck) {
    @synchronized ([NSObject class]) {
        gState = [state copy];
        gMessage = [message copy];
        gVersion = [version copy];
        gCanCheck = canCheck;
    }
}

static BOOL MTIsEnabled(void) {
    @synchronized ([NSObject class]) {
        return gEnabled;
    }
}

@interface MTUpdaterDelegate : NSObject
@end

@implementation MTUpdaterDelegate

- (BOOL)updater:(SPUUpdater *)updater mayPerformUpdateCheck:(NSInteger)updateCheck error:(NSError * __autoreleasing _Nullable *)error {
    (void)updater;
    // SPUUpdateCheckUpdates is 0 in Sparkle 2.10. Decline background/probe checks
    // even if a preference is changed outside this bridge while it is running.
    if (MTIsEnabled() && updateCheck == 0) return YES;
    if (error != NULL) {
        *error = [NSError errorWithDomain:@"MonoTermUpdater" code:1 userInfo:@{
            NSLocalizedDescriptionKey: @"Only manual update checks are enabled."
        }];
    }
    return NO;
}

- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater {
    (void)updater;
    return NO;
}

- (NSArray<NSString *> *)allowedSystemProfileKeysForUpdater:(SPUUpdater *)updater {
    (void)updater;
    return @[];
}

- (void)updater:(SPUUpdater *)updater didFindValidUpdate:(SUAppcastItem *)item {
    if (!MTIsEnabled()) return;
    NSString *version = item.displayVersionString;
    MTSetStatus(@"available", version.length ? [NSString stringWithFormat:@"Version %@ is available.", version] : @"An update is available.", version, updater.canCheckForUpdates);
}

- (void)updaterDidNotFindUpdate:(SPUUpdater *)updater error:(NSError *)error {
    if (!MTIsEnabled()) return;
    gNoUpdateError = error;
    MTSetStatus(@"current", @"No compatible update is currently available.", nil, updater.canCheckForUpdates);
}

- (BOOL)updater:(SPUUpdater *)updater shouldPostponeRelaunchForUpdate:(SUAppcastItem *)item untilInvokingBlock:(void (^)(void))installHandler {
    (void)updater;
    NSString *version = item.displayVersionString;
    @synchronized ([NSObject class]) {
        // Returning YES without invoking the block fails closed if updates were
        // disabled after this installation cycle began. No PTYs are stopped here.
        if (!gEnabled) return YES;
        gPostponementObserved = YES;
        gInstallApproved = NO;
        gPendingInstall = [installHandler copy];
        gState = @"install_pending";
        gVersion = [version copy];
        gMessage = @"The update is ready to install and relaunch.";
        gCanCheck = NO;
    }
    return YES;
}

- (BOOL)updaterShouldRelaunchApplication:(SPUUpdater *)updater {
    (void)updater;
    @synchronized ([NSObject class]) {
        if (!gEnabled) return NO;
        if (!MTTerminationGuardIsActive()) {
            gRelaunchBlocked = YES;
            gPendingInstall = nil;
            MTSetStatus(@"error", @"The application's termination guard changed; the update cannot safely relaunch.", nil, NO);
            return NO;
        }
        // Arm the final gate before permitting any path that could skip Sparkle's
        // postponement. Ordinary AppKit quits remain guarded from this point on.
        gInstallStarted = YES;
        // Sparkle 2.10 consults this before postponement, and resumed installer
        // drivers can skip postponement. Authorize only our exact continuation
        // after the first observed postponement, never a later untracked attempt.
        if (!gPostponementObserved) return YES;
        if (gInstallApproved) {
            gInstallApproved = NO;
            return YES;
        }
        gRelaunchBlocked = YES;
        gPendingInstall = nil; // Sparkle aborts this driver's cycle after NO.
        gState = @"error";
        gMessage = @"Relaunch was blocked because this installation attempt was not approved. Restart the application before retrying the update.";
        gCanCheck = NO;
        return NO;
    }
}

- (void)updater:(SPUUpdater *)updater didAbortWithError:(NSError *)error {
    if (!MTIsEnabled() || [error isEqual:gNoUpdateError]) return;
    @synchronized ([NSObject class]) {
        gPendingInstall = nil;
        gInstallApproved = NO;
        if (gRelaunchBlocked) return;
    }
    MTSetStatus(@"error", error.localizedDescription.length ? error.localizedDescription : @"The update check failed.", nil, updater.canCheckForUpdates);
}

- (void)updater:(SPUUpdater *)updater didFinishUpdateCycleForUpdateCheck:(NSInteger)updateCheck error:(NSError * _Nullable)error {
    (void)updateCheck;
    if (!MTIsEnabled()) return;
    @synchronized ([NSObject class]) {
        // A finished cycle invalidates the driver captured weakly by Sparkle's
        // deferred install block. Never retain an inert block for a later retry.
        gPendingInstall = nil;
        gInstallApproved = NO;
        gResumeQueued = NO;
        if (gRelaunchBlocked) return;
        gCanCheck = updater.canCheckForUpdates;
        if (error != nil && ![error isEqual:gNoUpdateError]) {
            gState = @"error";
            gMessage = error.localizedDescription.length ? error.localizedDescription : @"The update check failed.";
            gVersion = nil;
        } else if ([gState isEqualToString:@"checking"] || [gState isEqualToString:@"install_pending"]) {
            gState = @"ready";
            gMessage = @"The update session has finished. Check again to continue.";
            gVersion = nil;
        }
    }
}

@end

static void MTSetError(NSString *message) {
    MTSetStatus(@"error", message, nil, NO);
}

// Must run on the main thread before any checks. Disabling latches off for this
// process without loading Sparkle or writing its shared preferences.
void mt_updater_initialize(bool enabled) {
    @synchronized ([NSObject class]) {
        if (!enabled || getenv("MONOTERM_CONFIG_DIR") != NULL || gPermanentlyDisabled) {
            gEnabled = NO;
            gPermanentlyDisabled = YES;
            gInstallApproved = NO;
            gResumeQueued = NO;
            gPendingInstall = nil;
            MTSetStatus(@"disabled", @"Updates are disabled for this test instance.", nil, NO);
            return;
        }
    }
    if (![NSThread isMainThread]) {
        MTSetError(@"Updater initialization must run on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        gEnabled = YES;
        if (gUpdaterController != nil) return;
    }

    NSBundle *mainBundle = NSBundle.mainBundle;
    if (![mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"]) {
        MTSetStatus(@"unavailable", @"Updates require the packaged macOS application.", nil, NO);
        return;
    }

    id feedURL = [mainBundle objectForInfoDictionaryKey:@"SUFeedURL"];
    id publicKey = [mainBundle objectForInfoDictionaryKey:@"SUPublicEDKey"];
    if (![feedURL isKindOfClass:[NSString class]] || [feedURL length] == 0) {
        MTSetStatus(@"unavailable", @"SUFeedURL is missing from the application Info.plist.", nil, NO);
        return;
    }
    if (![publicKey isKindOfClass:[NSString class]] || [publicKey length] == 0) {
        MTSetStatus(@"unavailable", @"SUPublicEDKey is missing from the application Info.plist.", nil, NO);
        return;
    }

    NSString *frameworksPath = mainBundle.privateFrameworksPath;
    NSString *sparklePath = frameworksPath.length ? [frameworksPath stringByAppendingPathComponent:@"Sparkle.framework"] : nil;
    if (sparklePath == nil || ![[NSFileManager defaultManager] fileExistsAtPath:sparklePath]) {
        MTSetStatus(@"unavailable", @"Sparkle.framework is not embedded in this application.", nil, NO);
        return;
    }

    // Install before loading Sparkle, so no update can start without a final
    // AppKit termination gate. Existing host termination handlers are not replaced.
    if (gTerminationHandler == NULL || !MTInstallTerminationGuard(NSApp)) {
        MTSetStatus(@"unavailable", @"Updates require a registered terminal cleanup handler and a compatible application delegate.", nil, NO);
        return;
    }

    NSError *loadError = nil;
    NSBundle *sparkleBundle = [NSBundle bundleWithPath:sparklePath];
    if (sparkleBundle == nil || ![sparkleBundle loadAndReturnError:&loadError]) {
        NSString *detail = loadError.localizedDescription.length ? loadError.localizedDescription : @"The framework could not be loaded.";
        MTSetStatus(@"unavailable", [NSString stringWithFormat:@"Sparkle.framework is unavailable: %@", detail], nil, NO);
        return;
    }

    Class controllerClass = NSClassFromString(@"SPUStandardUpdaterController");
    if (controllerClass == Nil ||
        ![controllerClass instancesRespondToSelector:@selector(initWithStartingUpdater:updaterDelegate:userDriverDelegate:)] ||
        ![controllerClass instancesRespondToSelector:@selector(updater)] ||
        ![controllerClass instancesRespondToSelector:@selector(checkForUpdates:)]) {
        MTSetStatus(@"unavailable", @"The embedded Sparkle framework does not provide the required Sparkle 2 controller API.", nil, NO);
        return;
    }

    MTUpdaterDelegate *delegate = [[MTUpdaterDelegate alloc] init];
    SPUStandardUpdaterController *controller = [[controllerClass alloc] initWithStartingUpdater:NO updaterDelegate:delegate userDriverDelegate:nil];
    if (controller == nil) {
        MTSetStatus(@"unavailable", @"Sparkle could not create its standard updater controller.", nil, NO);
        return;
    }

    SPUUpdater *updater = controller.updater;
    if (![updater respondsToSelector:@selector(startUpdater:)] ||
        ![updater respondsToSelector:@selector(setAutomaticallyChecksForUpdates:)] ||
        ![updater respondsToSelector:@selector(setAutomaticallyDownloadsUpdates:)] ||
        ![updater respondsToSelector:@selector(setSendsSystemProfile:)] ||
        ![updater respondsToSelector:@selector(canCheckForUpdates)] ||
        ![updater respondsToSelector:@selector(sessionInProgress)]) {
        MTSetStatus(@"unavailable", @"The embedded Sparkle updater API is incompatible.", nil, NO);
        return;
    }
    updater.automaticallyChecksForUpdates = NO;
    updater.automaticallyDownloadsUpdates = NO;
    updater.sendsSystemProfile = NO;

    NSError *startError = nil;
    if (![updater startUpdater:&startError]) {
        NSString *detail = startError.localizedDescription.length ? startError.localizedDescription : @"Sparkle could not start.";
        MTSetStatus(@"unavailable", [NSString stringWithFormat:@"Sparkle could not start: %@", detail], nil, NO);
        return;
    }

    @synchronized ([NSObject class]) {
        gSparkleBundle = sparkleBundle;
        gUpdaterDelegate = delegate;
        gUpdaterController = controller;
        gUpdater = updater;
    }
    MTSetStatus(@"ready", @"Updates are ready for a manual check.", nil, updater.canCheckForUpdates);
}

// Main-thread-only. Shows Sparkle's standard UI and preserves a pending install
// so Rust can resume its continuation without starting a competing update check.
void mt_updater_check(void) {
    if (![NSThread isMainThread]) {
        MTSetError(@"Update checks must run on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        if (!gEnabled) {
            gState = @"disabled";
            gMessage = @"Updates are disabled for this test instance.";
            gCanCheck = NO;
            return;
        }
        if (gResumeQueued || gRelaunchBlocked) return;
        if (gPendingInstall != nil) {
            gState = @"install_pending";
            gMessage = @"The pending update is ready to install and relaunch.";
            gCanCheck = NO;
            return;
        }
    }

    if (gUpdaterController == nil || gUpdater == nil) return;
    if (!gUpdater.canCheckForUpdates) return;
    // During an existing session this action merely focuses Sparkle's UI.
    // Preserve its result (including the no-update NSError) until it completes.
    if (!gUpdater.sessionInProgress) {
        gNoUpdateError = nil;
        MTSetStatus(@"checking", @"Checking for updates…", nil, NO);
    }
    [gUpdaterController checkForUpdates:nil];
}

// A background caller receives the last snapshot; main-thread polling also
// refreshes Sparkle's asynchronous canCheckForUpdates. Free the strdup result.
char *mt_updater_status(void) {
    @autoreleasepool {
        NSDictionary *snapshot;
        @synchronized ([NSObject class]) {
            if (NSThread.isMainThread && gEnabled && gUpdater != nil && !gRelaunchBlocked && !gResumeQueued) {
                gCanCheck = gPendingInstall != nil ? [gState isEqualToString:@"error"] : gUpdater.canCheckForUpdates;
            }
            NSMutableDictionary *fields = [@{
                @"state": gState ?: @"unavailable",
                @"message": gMessage ?: @"Updater status is unavailable.",
                @"can_check": @(gCanCheck)
            } mutableCopy];
            if (gVersion != nil) fields[@"version"] = gVersion;
            snapshot = [fields copy];
        }

        NSData *json = [NSJSONSerialization dataWithJSONObject:snapshot options:0 error:NULL];
        NSString *text = json == nil ? nil : [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
        if (text == nil) return strdup("{\"state\":\"error\",\"message\":\"Could not serialize updater status.\",\"can_check\":false}");
        return strdup(text.UTF8String);
    }
}

// Main-thread-only. Resuming does not clean up terminal sessions; only an actual
// termination event does. A declined continuation remains available for retry.
void mt_updater_resume_install(bool allowed) {
    if (![NSThread isMainThread]) {
        MTSetError(@"Update installation must be resumed on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        if (!gEnabled) {
            gState = @"disabled";
            gMessage = @"Updates are disabled for this test instance.";
            gCanCheck = NO;
            return;
        }
        if (gResumeQueued || gRelaunchBlocked) return;
        if (gPendingInstall == nil) {
            gState = @"error";
            gMessage = @"There is no pending update installation to resume.";
            gCanCheck = gUpdater.canCheckForUpdates;
            return;
        }
        if (!allowed) {
            gInstallApproved = NO;
            gState = @"error";
            gMessage = @"The update remains pending. Check again to retry installation.";
            gCanCheck = YES;
            return;
        }
        gResumeQueued = YES;
        gState = @"checking";
        gMessage = @"Sparkle is continuing the installation.";
        gCanCheck = NO;
    }

    // Rust calls from inside a GPUI entity update. Defer out of that borrow before
    // Sparkle can send a termination event; never initiate termination ourselves.
    dispatch_async(dispatch_get_main_queue(), ^{
        void (^installHandler)(void) = nil;
        @synchronized ([NSObject class]) {
            if (!gResumeQueued) return;
            gResumeQueued = NO;
            if (!gEnabled || gRelaunchBlocked || gPendingInstall == nil) return;
            if (!MTTerminationGuardIsActive()) {
                gRelaunchBlocked = YES;
                MTSetStatus(@"error", @"The application's termination guard changed; the update cannot safely relaunch.", nil, NO);
                return;
            }
            installHandler = gPendingInstall;
            gPendingInstall = nil;
            gInstallStarted = YES;
            gInstallApproved = YES;
        }
        installHandler();
        // Approval is only valid while this exact continuation is executing.
        // Sparkle 2.10 synchronously consults the relaunch gate in this block.
        @synchronized ([NSObject class]) {
            gInstallApproved = NO;
        }
    });
}

// Main-thread-only. Register before enabled initialization. The callback must
// remain valid for the process lifetime and never unwind over the C ABI. It
// synchronously clears local sessions for every actual termination attempt.
// No updater lock is held while invoking it. NULL makes termination fail closed.
void mt_updater_set_termination_handler(bool (*handler)(void)) {
    if (!NSThread.isMainThread) return;
    @synchronized ([NSObject class]) {
        gTerminationHandler = handler;
    }
}
