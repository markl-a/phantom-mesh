// ios_motion.m
//
// Native iOS multi-sensor one-shot read (CoreMotion + UIDevice), compiled by
// build.rs via the `cc` crate — same path as ios_location.m / ios_fetch.m — so
// the `spectyn_ios_sensors` symbol lives in our dylib. Exposed to Rust via
// extern "C"; see app/src-tauri/src/lib.rs (swift_get_sensors command).
//
// Returns a single JSON blob the AI consumes as "what the phone's sensors say
// right now" — the behaviour half of the life-partner (NORTH-STAR §Q2: judge
// alignment from location + behaviour). Mirrors ios_location.m's semaphore
// pattern: device-motion is async (needs a live sample), so we start updates,
// wait briefly for one sample, read it, stop.
//
// PERMISSIONS:
//   - raw device-motion (accel/gyro/attitude/magnetometer) + battery: NO prompt.
//   - pedometer (steps) + activity (walking/running/automotive/stationary):
//     require NSMotionUsageDescription in Info.plist + a one-time user prompt.
//     Best-effort here — if denied/unavailable the JSON simply omits them.

#import <Foundation/Foundation.h>
#import <CoreMotion/CoreMotion.h>
#import <UIKit/UIKit.h>

// Read battery via UIDevice (must enable monitoring; main thread).
static void spectyn_read_battery(NSMutableDictionary *out) {
    dispatch_sync(dispatch_get_main_queue(), ^{
        UIDevice *dev = [UIDevice currentDevice];
        dev.batteryMonitoringEnabled = YES;
        float lvl = dev.batteryLevel; // -1 if unknown
        NSString *state;
        switch (dev.batteryState) {
            case UIDeviceBatteryStateCharging:  state = @"charging";  break;
            case UIDeviceBatteryStateFull:      state = @"full";      break;
            case UIDeviceBatteryStateUnplugged: state = @"unplugged"; break;
            default:                            state = @"unknown";   break;
        }
        if (lvl >= 0) out[@"battery_level"] = @(lvl);     // 0.0–1.0
        out[@"battery_state"] = state;
    });
}

// One device-motion sample (attitude, userAcceleration, gravity, rotationRate,
// magneticField). No permission needed. ~0.4s to get a stable sample.
static void spectyn_read_motion(NSMutableDictionary *out) {
    CMMotionManager *mm = [[CMMotionManager alloc] init];
    if (!mm.deviceMotionAvailable) {
        out[@"motion"] = @"unavailable";
        return;
    }
    mm.deviceMotionUpdateInterval = 0.1;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block BOOL got = NO;
    [mm startDeviceMotionUpdatesToQueue:[[NSOperationQueue alloc] init]
                           withHandler:^(CMDeviceMotion *dm, NSError *err) {
        if (got || dm == nil) return;
        got = YES;
        out[@"attitude"] = @{ @"pitch": @(dm.attitude.pitch),
                              @"roll":  @(dm.attitude.roll),
                              @"yaw":   @(dm.attitude.yaw) };
        out[@"user_accel"] = @{ @"x": @(dm.userAcceleration.x),
                                @"y": @(dm.userAcceleration.y),
                                @"z": @(dm.userAcceleration.z) };
        out[@"gravity"] = @{ @"x": @(dm.gravity.x),
                             @"y": @(dm.gravity.y),
                             @"z": @(dm.gravity.z) };
        out[@"rotation_rate"] = @{ @"x": @(dm.rotationRate.x),
                                   @"y": @(dm.rotationRate.y),
                                   @"z": @(dm.rotationRate.z) };
        if (dm.magneticField.accuracy != CMMagneticFieldCalibrationAccuracyUncalibrated) {
            out[@"magnetic_field"] = @{ @"x": @(dm.magneticField.field.x),
                                        @"y": @(dm.magneticField.field.y),
                                        @"z": @(dm.magneticField.field.z) };
        }
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
    [mm stopDeviceMotionUpdates];
}

// Pedometer steps since midnight (best-effort; needs Motion permission).
static void spectyn_read_pedometer(NSMutableDictionary *out) {
    if (![CMPedometer isStepCountingAvailable]) return;
    CMPedometer *ped = [[CMPedometer alloc] init];
    NSCalendar *cal = [NSCalendar currentCalendar];
    NSDate *midnight = [cal startOfDayForDate:[NSDate date]];
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [ped queryPedometerDataFromDate:midnight toDate:[NSDate date]
                        withHandler:^(CMPedometerData *d, NSError *err) {
        if (d != nil && err == nil) {
            out[@"steps_today"] = d.numberOfSteps ?: @0;
            if (d.distance) out[@"distance_m_today"] = d.distance;
        }
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC));
}

// Latest motion-activity classification (best-effort; needs Motion permission).
static void spectyn_read_activity(NSMutableDictionary *out) {
    if (![CMMotionActivityManager isActivityAvailable]) return;
    CMMotionActivityManager *am = [[CMMotionActivityManager alloc] init];
    NSDate *from = [NSDate dateWithTimeIntervalSinceNow:-3600]; // last hour
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [am queryActivityStartingFromDate:from toDate:[NSDate date]
                              toQueue:[[NSOperationQueue alloc] init]
                          withHandler:^(NSArray<CMMotionActivity *> *acts, NSError *err) {
        CMMotionActivity *a = acts.lastObject;
        if (a != nil) {
            NSString *kind = @"unknown";
            if (a.stationary) kind = @"stationary";
            else if (a.walking) kind = @"walking";
            else if (a.running) kind = @"running";
            else if (a.automotive) kind = @"automotive";
            else if (a.cycling) kind = @"cycling";
            out[@"activity"] = kind;
            NSString *conf = a.confidence == CMMotionActivityConfidenceHigh ? @"high"
                           : a.confidence == CMMotionActivityConfidenceMedium ? @"medium" : @"low";
            out[@"activity_confidence"] = conf;
        }
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC));
}

// Public entry: fill json_buf with a UTF-8 JSON object of all sensor readings.
void spectyn_ios_sensors(char *json_buf, long *json_len, long max_len) {
    @autoreleasepool {
        NSMutableDictionary *out = [NSMutableDictionary dictionary];
        out[@"ts_unix"] = @((long)[[NSDate date] timeIntervalSince1970]);
        out[@"device"] = [[UIDevice currentDevice] model] ?: @"iPhone";

        spectyn_read_battery(out);
        spectyn_read_motion(out);
        spectyn_read_pedometer(out);   // best-effort (Motion permission)
        spectyn_read_activity(out);    // best-effort (Motion permission)

        NSError *jerr = nil;
        NSData *data = [NSJSONSerialization dataWithJSONObject:out options:0 error:&jerr];
        if (data == nil) {
            const char *fallback = "{\"error\":\"sensor json serialization failed\"}";
            long n = (long)strlen(fallback);
            if (n > max_len) n = max_len;
            memcpy(json_buf, fallback, (size_t)n);
            *json_len = n;
            return;
        }
        long n = (long)data.length;
        if (n > max_len) n = max_len;
        memcpy(json_buf, data.bytes, (size_t)n);
        *json_len = n;
    }
}
