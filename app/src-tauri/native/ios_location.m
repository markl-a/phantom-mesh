// ios_location.m
//
// Native iOS CoreLocation one-shot GPS read, compiled by build.rs via the
// `cc` crate (same path as native/ios_fetch.m) so the `phantom_ios_location`
// symbol lives in our dylib. Exposed to Rust via extern "C" — see
// app/src-tauri/src/lib.rs (swift_get_location command).
//
// Why native: WKWebView's navigator.geolocation is unreliable in the Tauri
// webview (it piggybacks on the host app's CLLocationManager authorization,
// which Tauri never requests). CLLocationManager is the Apple-blessed path.
//
// Pattern mirrors ios_fetch.m: a DispatchSemaphore makes the async delegate
// callback synchronous. The manager is created + started on the main queue
// (CLLocationManager needs a live run loop for its delegate); the C caller
// blocks on a background (tokio) thread until the delegate signals.

#import <Foundation/Foundation.h>
#import <CoreLocation/CoreLocation.h>

@interface PhantomLocDelegate : NSObject <CLLocationManagerDelegate>
@property (nonatomic, strong) CLLocationManager *mgr;
@property (nonatomic, strong) dispatch_semaphore_t sem;
@property (nonatomic, assign) double lat;
@property (nonatomic, assign) double lon;
@property (nonatomic, assign) double acc;
@property (nonatomic, assign) BOOL done;
@property (nonatomic, strong) NSString *err;
@end

@implementation PhantomLocDelegate

- (void)finishWithError:(NSString *)msg {
    if (self.done) return;
    self.err = msg;
    self.done = YES;
    dispatch_semaphore_signal(self.sem);
}

- (void)locationManager:(CLLocationManager *)manager
     didUpdateLocations:(NSArray<CLLocation *> *)locations {
    if (self.done) return;
    CLLocation *loc = locations.lastObject;
    if (loc != nil) {
        self.lat = loc.coordinate.latitude;
        self.lon = loc.coordinate.longitude;
        self.acc = loc.horizontalAccuracy;
        self.done = YES;
        [manager stopUpdatingLocation];
        dispatch_semaphore_signal(self.sem);
    }
}

- (void)locationManager:(CLLocationManager *)manager
       didFailWithError:(NSError *)error {
    NSLog(@"[PhantomLoc] didFailWithError: %@", error.localizedDescription);
    [self finishWithError:error.localizedDescription];
}

- (void)locationManagerDidChangeAuthorization:(CLLocationManager *)manager {
    CLAuthorizationStatus st = manager.authorizationStatus;
    NSLog(@"[PhantomLoc] auth changed: %d", (int)st);
    if (st == kCLAuthorizationStatusAuthorizedWhenInUse ||
        st == kCLAuthorizationStatusAuthorizedAlways) {
        [manager startUpdatingLocation];
    } else if (st == kCLAuthorizationStatusDenied ||
               st == kCLAuthorizationStatusRestricted) {
        [self finishWithError:@"location permission denied"];
    }
    // kCLAuthorizationStatusNotDetermined: still waiting for the prompt.
}

@end

void phantom_ios_location(
    double *lat_out,
    double *lon_out,
    double *acc_out,
    char *err_buf,
    long *err_len,
    long max_err
) {
    @autoreleasepool {
        PhantomLocDelegate *del = [[PhantomLocDelegate alloc] init];
        del.sem = dispatch_semaphore_create(0);
        del.done = NO;
        del.acc = -1;

        dispatch_async(dispatch_get_main_queue(), ^{
            del.mgr = [[CLLocationManager alloc] init];
            del.mgr.delegate = del;
            del.mgr.desiredAccuracy = kCLLocationAccuracyHundredMeters;
            CLAuthorizationStatus st = del.mgr.authorizationStatus;
            NSLog(@"[PhantomLoc] initial auth: %d", (int)st);
            if (st == kCLAuthorizationStatusNotDetermined) {
                [del.mgr requestWhenInUseAuthorization];
            } else if (st == kCLAuthorizationStatusAuthorizedWhenInUse ||
                       st == kCLAuthorizationStatusAuthorizedAlways) {
                [del.mgr startUpdatingLocation];
            } else {
                [del finishWithError:@"location permission denied"];
            }
        });

        // Wait up to 25s (covers the first-run permission prompt + GPS fix).
        long wait = dispatch_semaphore_wait(
            del.sem, dispatch_time(DISPATCH_TIME_NOW, 25 * NSEC_PER_SEC));

        if (wait != 0) {
            const char *msg = "location timeout";
            long mlen = (long)strlen(msg);
            if (mlen > max_err) mlen = max_err;
            memcpy(err_buf, msg, (size_t)mlen);
            *err_len = mlen;
            *lat_out = 0; *lon_out = 0; *acc_out = -1;
            return;
        }

        if (del.err != nil) {
            const char *cstr = [del.err UTF8String];
            long mlen = (long)strlen(cstr);
            if (mlen > max_err) mlen = max_err;
            memcpy(err_buf, cstr, (size_t)mlen);
            *err_len = mlen;
            *lat_out = 0; *lon_out = 0; *acc_out = -1;
        } else {
            *lat_out = del.lat;
            *lon_out = del.lon;
            *acc_out = del.acc;
            *err_len = 0;
        }
    }
}
