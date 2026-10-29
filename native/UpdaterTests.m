// Exercise the bridge without loading or executing Sparkle.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "Updater.m"
#pragma clang diagnostic pop
#include <assert.h>
#include <stdio.h>

@interface MTFakeUpdater : NSObject
@property (nonatomic) BOOL canCheckForUpdates;
@property (nonatomic) BOOL sessionInProgress;
@end
@implementation MTFakeUpdater
@end

@interface MTFakeItem : NSObject
@property (nonatomic, copy) NSString *displayVersionString;
@end
@implementation MTFakeItem
@end

@interface MTTestAppDelegate : NSObject <NSApplicationDelegate>
@end
@implementation MTTestAppDelegate
@end

@interface MTExistingAppDelegate : NSObject <NSApplicationDelegate>
@end
@implementation MTExistingAppDelegate
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)application {
    (void)application;
    return NSTerminateCancel;
}
@end

@interface MTFakeApplication : NSObject
@property (nonatomic, strong) id<NSApplicationDelegate> delegate;
@end
@implementation MTFakeApplication
@end

static BOOL gTestCleanupAllowed;
static NSUInteger gTestCleanupAttempts;

static bool MTTestCleanupHandler(void) {
    gTestCleanupAttempts++;
    char *json = mt_updater_status(); // Rust may call back into bridge status.
    free(json);
    assert([NSApp.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
    return gTestCleanupAllowed;
}

static NSDictionary *MTTestStatus(void) {
    char *json = mt_updater_status();
    assert(json != NULL);
    NSData *data = [NSData dataWithBytes:json length:strlen(json)];
    free(json);
    NSDictionary *result = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    assert(result != nil);
    return result;
}

static void MTExpectState(NSString *value) {
    assert([MTTestStatus()[@"state"] isEqualToString:value]);
}

static void MTDrainMainQueue(void) {
    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
}

// Real AppKit termination in an isolated process with no windows and no Sparkle.
// Both requests originate on the GCD main queue, so a nested NSTerminateLater
// loop would deadlock. Synchronous cleanup must cancel once, then allow exit.
static void MTTestAppKitTermination(void) {
    NSApplication *application = NSApplication.sharedApplication;
    [application setActivationPolicy:NSApplicationActivationPolicyProhibited];
    MTTestAppDelegate *delegate = [[MTTestAppDelegate alloc] init];
    application.delegate = delegate;
    mt_updater_set_termination_handler(MTTestCleanupHandler);
    assert(MTInstallTerminationGuard(application));
    assert(MTTerminationGuardIsActive());
    gEnabled = YES;
    gInstallStarted = YES;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC), dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        fputs("AppKit termination test timed out\n", stderr);
        _Exit(1);
    });
    dispatch_async(dispatch_get_main_queue(), ^{
        [application terminate:nil];
        assert(gTestCleanupAttempts == 1);
        MTExpectState(@"error");
        gTestCleanupAllowed = YES;
        dispatch_async(dispatch_get_main_queue(), ^{
            puts("AppKit synchronous cleanup cancellation and retry approval: passed");
            fflush(stdout);
            [application terminate:nil];
            abort();
        });
    });
    [application run];
    (void)delegate;
    abort();
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc > 1 && strcmp(argv[1], "appkit") == 0) {
            MTTestAppKitTermination();
            return 1;
        }
        if (argc > 1 && strcmp(argv[1], "disabled") == 0) {
            mt_updater_initialize(false);
            MTExpectState(@"disabled");
            mt_updater_initialize(true);
            mt_updater_check();
            mt_updater_resume_install(true);
            MTExpectState(@"disabled");
            assert(gUpdaterController == nil && gSparkleBundle == nil);
            assert(!gInstallStarted && ![MTTestStatus()[@"can_check"] boolValue]);
            puts("disabled process latch: passed (no framework or guard loaded)");
            return 0;
        }
        if (argc > 1 && strcmp(argv[1], "config-disabled") == 0) {
            assert(setenv("MONOTERM_CONFIG_DIR", "", 1) == 0);
            mt_updater_initialize(true);
            MTExpectState(@"disabled");
            assert(unsetenv("MONOTERM_CONFIG_DIR") == 0);
            mt_updater_initialize(true);
            MTExpectState(@"disabled");
            assert(gUpdaterController == nil && gSparkleBundle == nil);
            assert(!gInstallStarted && ![MTTestStatus()[@"can_check"] boolValue]);
            puts("alternate config process latch: passed (including empty override)");
            return 0;
        }
        if (argc > 1 && strcmp(argv[1], "unavailable") == 0) {
            mt_updater_initialize(true);
            MTExpectState(@"unavailable");
            mt_updater_check();
            MTExpectState(@"unavailable");
            assert(gUpdaterController == nil && gSparkleBundle == nil);
            assert(![MTTestStatus()[@"can_check"] boolValue]);
            puts("non-app availability: passed (no framework loaded)");
            return 0;
        }

        MTFakeApplication *application = [[MTFakeApplication alloc] init];
        NSApp = (NSApplication *)application;
        application.delegate = [[MTExistingAppDelegate alloc] init];
        assert(!MTInstallTerminationGuard(NSApp));
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        application.delegate = [[MTTestAppDelegate alloc] init];
        assert(MTInstallTerminationGuard(NSApp));
        assert(MTInstallTerminationGuard(NSApp)); // Repeated initialization is safe.
        assert(MTTerminationGuardIsActive());
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateNow);
        mt_updater_set_termination_handler(MTTestCleanupHandler);

        gEnabled = YES;
        MTFakeUpdater *fake = [[MTFakeUpdater alloc] init];
        SPUUpdater *updater = (SPUUpdater *)fake;
        gUpdater = updater;
        MTUpdaterDelegate *delegate = [[MTUpdaterDelegate alloc] init];
        if (argc > 1 && strcmp(argv[1], "disable-queued") == 0) {
            __block BOOL invoked = NO;
            assert([delegate updater:updater shouldPostponeRelaunchForUpdate:nil untilInvokingBlock:^{ invoked = YES; }]);
            mt_updater_resume_install(true);
            mt_updater_initialize(false);
            MTDrainMainQueue();
            assert(!invoked && !gInstallApproved && !gResumeQueued && gPendingInstall == nil);
            assert(gTestCleanupAttempts == 0);
            MTExpectState(@"disabled");
            puts("disable cancels queued install approval without cleaning up sessions: passed");
            return 0;
        }
        if (argc > 1 && strcmp(argv[1], "finish-queued") == 0) {
            __block BOOL invoked = NO;
            assert([delegate updater:updater shouldPostponeRelaunchForUpdate:nil untilInvokingBlock:^{ invoked = YES; }]);
            mt_updater_resume_install(true);
            [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:nil];
            MTDrainMainQueue();
            assert(!invoked && !gInstallApproved && !gResumeQueued && gPendingInstall == nil);
            assert(gTestCleanupAttempts == 0);
            MTExpectState(@"ready");
            puts("finished cycle cancels stale queued continuation: passed");
            return 0;
        }
        if (argc > 1 && strcmp(argv[1], "guard-changed") == 0) {
            __block BOOL invoked = NO;
            assert([delegate updater:updater shouldPostponeRelaunchForUpdate:nil untilInvokingBlock:^{ invoked = YES; }]);
            mt_updater_resume_install(true);
            application.delegate = [[MTExistingAppDelegate alloc] init];
            MTDrainMainQueue();
            assert(!invoked && gRelaunchBlocked && !gInstallApproved);
            assert(gTestCleanupAttempts == 0);
            assert(![delegate updaterShouldRelaunchApplication:updater]);
            MTExpectState(@"error");
            assert(![MTTestStatus()[@"can_check"] boolValue]);
            puts("changed termination guard blocks queued install: passed");
            return 0;
        }
        MTFakeItem *fakeItem = [[MTFakeItem alloc] init];
        fakeItem.displayVersionString = @"0.1.1\"\né";
        SUAppcastItem *item = (SUAppcastItem *)fakeItem;
        NSError *error = nil;
        assert([delegate updater:updater mayPerformUpdateCheck:0 error:&error]);
        assert(![delegate updater:updater mayPerformUpdateCheck:1 error:&error]);
        assert(![delegate updater:updater mayPerformUpdateCheck:2 error:&error]);
        assert(error != nil);
        assert(![delegate updaterShouldPromptForPermissionToCheckForUpdates:updater]);
        assert([delegate allowedSystemProfileKeysForUpdater:updater].count == 0);

        [delegate updater:updater didFindValidUpdate:item];
        MTExpectState(@"available");
        assert([MTTestStatus()[@"version"] isEqualToString:fakeItem.displayVersionString]);
        NSError *noUpdate = [NSError errorWithDomain:@"SyntheticSparkle" code:1001 userInfo:nil];
        [delegate updaterDidNotFindUpdate:updater error:noUpdate];
        [delegate updater:updater didAbortWithError:noUpdate];
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:noUpdate];
        MTExpectState(@"current");
        assert(MTTestStatus()[@"version"] == nil);
        NSError *failure = [NSError errorWithDomain:@"SyntheticNetwork" code:1001 userInfo:@{NSLocalizedDescriptionKey: @"Offline"}];
        [delegate updater:updater didAbortWithError:failure];
        MTExpectState(@"error");
        fake.canCheckForUpdates = YES;
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:failure];
        assert([MTTestStatus()[@"can_check"] boolValue]);
        assert(gTestCleanupAttempts == 0);

        assert([delegate updaterShouldRelaunchApplication:updater]);
        assert(gInstallStarted);
        __block NSUInteger installs = 0;
        assert([delegate updater:updater shouldPostponeRelaunchForUpdate:item untilInvokingBlock:^{
            assert([delegate updaterShouldRelaunchApplication:updater]);
            installs++;
        }]);
        MTExpectState(@"install_pending");
        assert(![MTTestStatus()[@"can_check"] boolValue]);
        assert(gTestCleanupAttempts == 0);
        mt_updater_resume_install(false);
        MTExpectState(@"error");
        assert(installs == 0 && gPendingInstall != nil);
        assert([MTTestStatus()[@"can_check"] boolValue]);
        mt_updater_check();
        MTExpectState(@"install_pending");
        mt_updater_resume_install(true);
        MTExpectState(@"checking");
        assert(installs == 0 && gTestCleanupAttempts == 0);
        mt_updater_resume_install(true);
        MTDrainMainQueue();
        assert(installs == 1 && gPendingInstall == nil && !gInstallApproved);
        assert(gTestCleanupAttempts == 0); // Resuming never stops terminal sessions.

        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestCleanupAttempts == 1);
        MTExpectState(@"error");
        gTestCleanupAllowed = YES;
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateNow);
        assert(gTestCleanupAttempts == 2);
        gTestCleanupAllowed = NO;
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestCleanupAttempts == 3);
        mt_updater_set_termination_handler(NULL);
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestCleanupAttempts == 3);
        mt_updater_set_termination_handler(MTTestCleanupHandler);
        [delegate updater:updater didAbortWithError:failure];
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:failure];
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestCleanupAttempts == 4);

        assert(![delegate updaterShouldRelaunchApplication:updater]);
        MTExpectState(@"error");
        assert(gRelaunchBlocked && ![MTTestStatus()[@"can_check"] boolValue]);
        mt_updater_initialize(false);
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestCleanupAttempts == 5); // Disabling cannot remove an armed cleanup guard.
        assert(![delegate updater:updater mayPerformUpdateCheck:0 error:&error]);
        puts("synthetic statuses, retries, deferred installation and actual termination cleanup: passed");
        NSApp = nil;
    }
    return 0;
}
