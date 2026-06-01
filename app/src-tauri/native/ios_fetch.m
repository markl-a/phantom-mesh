// ios_fetch.m
//
// Native iOS URLSession-based HTTP fetch, compiled by cargo's build.rs
// via the `cc` crate so the symbol is in the same dylib the Rust code
// produces (which avoids the "Swift-compiled-by-xcodebuild-but-Rust-
// links-first" link error we hit on the swift_cluster_fetch attempt).
//
// Exposed to Rust via extern "C" — see app/src-tauri/src/lib.rs.
//
// Why this exists:
//   tauri-plugin-http's reqwest backend silently times out fetching
//   Tailscale magic hostnames + private IPs from physical iOS devices.
//   reqwest uses raw sockets that don't go through iOS's standard URL
//   loading stack, so the sandbox / NetworkExtension / VPN layer treats
//   them differently. NSURLSession is the Apple-blessed path; works
//   reliably for both LAN + Tailnet hosts.

#import <Foundation/Foundation.h>

void phantom_ios_fetch(
    const char *url_cstr,
    const char *method_cstr,
    const unsigned char *body_bytes,
    long body_len,
    const char *auth_header_cstr,
    unsigned char *result_buf,
    long *result_buf_len,
    long *status_out,
    long max_result_len
) {
    @autoreleasepool {
        NSString *urlStr = [NSString stringWithUTF8String:url_cstr];
        NSString *methodStr = [NSString stringWithUTF8String:method_cstr];
        NSString *authHeader = [NSString stringWithUTF8String:auth_header_cstr];

        NSLog(@"[PhantomFetch] entry method=%@ url=%@ body_len=%ld auth_len=%lu",
              methodStr, urlStr, body_len, (unsigned long)authHeader.length);

        NSURL *url = [NSURL URLWithString:urlStr];
        if (url == nil) {
            *status_out = -1;
            const char *msg = "invalid url";
            long mlen = (long)strlen(msg);
            if (mlen > max_result_len) mlen = max_result_len;
            memcpy(result_buf, msg, (size_t)mlen);
            *result_buf_len = mlen;
            return;
        }

        NSMutableURLRequest *req = [NSMutableURLRequest requestWithURL:url];
        req.HTTPMethod = methodStr;
        req.timeoutInterval = 30;

        NSString *upper = [methodStr uppercaseString];
        if ([upper isEqualToString:@"POST"] || [upper isEqualToString:@"PUT"]) {
            if (body_bytes != NULL && body_len > 0) {
                req.HTTPBody = [NSData dataWithBytes:body_bytes length:(NSUInteger)body_len];
                [req setValue:@"application/json" forHTTPHeaderField:@"Content-Type"];
            }
        }
        if (authHeader.length > 0) {
            [req setValue:authHeader forHTTPHeaderField:@"X-Cluster-Auth"];
        }

        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        __block NSData *respData = nil;
        __block NSInteger respStatus = 0;
        __block NSString *errMsg = nil;

        NSURLSessionDataTask *task = [[NSURLSession sharedSession]
            dataTaskWithRequest:req
                completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
                    if (error != nil) {
                        errMsg = error.localizedDescription;
                    } else {
                        if ([response isKindOfClass:[NSHTTPURLResponse class]]) {
                            respStatus = ((NSHTTPURLResponse *)response).statusCode;
                        }
                        respData = data;
                    }
                    dispatch_semaphore_signal(sem);
                }];
        [task resume];
        long waitResult = dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 35 * NSEC_PER_SEC));

        NSLog(@"[PhantomFetch] complete waitResult=%ld status=%ld dataLen=%lu err=%@",
              waitResult, (long)respStatus, (unsigned long)(respData ? respData.length : 0), errMsg);

        *status_out = (long)respStatus;

        if (respData != nil) {
            long copyLen = (long)respData.length;
            if (copyLen > max_result_len) copyLen = max_result_len;
            memcpy(result_buf, respData.bytes, (size_t)copyLen);
            *result_buf_len = copyLen;
        } else if (errMsg != nil) {
            const char *cstr = [errMsg UTF8String];
            long mlen = (long)strlen(cstr);
            if (mlen > max_result_len) mlen = max_result_len;
            memcpy(result_buf, cstr, (size_t)mlen);
            *result_buf_len = mlen;
            if (*status_out == 0) *status_out = -1;
        } else {
            *result_buf_len = 0;
        }
    }
}
