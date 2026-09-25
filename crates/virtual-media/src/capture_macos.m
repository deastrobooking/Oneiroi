#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>

typedef void (*VIRTUALDeviceVisitor)(void *, const char *, const char *);

// Enumerating does not open a capture session or request camera permission.
// Both video and muxed devices are needed for external capture hardware.
int virtual_video_inputs(void *context, VIRTUALDeviceVisitor visit) {
    @autoreleasepool {
        @try {
            NSMutableArray *types = [NSMutableArray arrayWithObject:AVCaptureDeviceTypeBuiltInWideAngleCamera];
            if (@available(macOS 14.0, *)) {
                [types addObject:AVCaptureDeviceTypeExternal];
                [types addObject:AVCaptureDeviceTypeContinuityCamera];
            } else {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
                [types addObject:AVCaptureDeviceTypeExternalUnknown];
#pragma clang diagnostic pop
            }
            NSMutableSet *seen = [NSMutableSet set];
            for (AVMediaType mediaType in @[AVMediaTypeVideo, AVMediaTypeMuxed]) {
                AVCaptureDeviceDiscoverySession *session =
                    [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:types
                        mediaType:mediaType position:AVCaptureDevicePositionUnspecified];
                for (AVCaptureDevice *device in session.devices) {
                    if ([seen containsObject:device.uniqueID]) continue;
                    [seen addObject:device.uniqueID];
                    visit(context, device.uniqueID.UTF8String, device.localizedName.UTF8String);
                }
            }
            return 0;
        } @catch (NSException *exception) {
            return -1;
        }
    }
}
