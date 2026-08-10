#ifndef COWBOY_KEYBOARD_GEOMETRY_H
#define COWBOY_KEYBOARD_GEOMETRY_H

#include <math.h>
#include <stdbool.h>

typedef struct {
    double parentWidth;
    double parentHeight;
    double keyboardMinY;
    double keyboardMaxY;
    double keyboardWidth;
    double keyboardHeight;
} CowboyKeyboardOverlapInput;

typedef struct {
    double overlap;
    double frameDepth;
    bool frameReachesBottom;
    bool fullWidthDockCandidate;
} CowboyKeyboardOverlapResult;

// UIKeyboardFrameEnd can briefly retain the previous interface orientation.
// CGRect conversion then swaps the keyboard's long and short axes, so a
// vertical intersection is not a trustworthy keyboard depth by itself. The
// short edge is orientation-invariant and bounds how much of the current
// viewport this notification may remove until keyboardLayoutGuide settles.
static inline CowboyKeyboardOverlapResult cowboyKeyboardOverlapForNotification(
    CowboyKeyboardOverlapInput input
) {
    const double width = fabs(input.keyboardWidth);
    const double height = fabs(input.keyboardHeight);
    const double frameDepth = fmin(width, height);
    const double frameLongEdge = fmax(width, height);
    const bool frameReachesBottom =
        input.keyboardMaxY >= input.parentHeight - 2.0;
    const bool fullWidthDockCandidate =
        frameDepth >= 80.0 &&
        frameLongEdge >= input.parentWidth * 0.8;

    double overlap = 0.0;
    if (frameReachesBottom) {
        const double bottomIntersection =
            fmax(0.0, input.parentHeight - input.keyboardMinY);
        overlap = fmin(bottomIntersection, frameDepth);
    } else if (fullWidthDockCandidate) {
        overlap = frameDepth;
    }
    overlap = fmin(fmax(0.0, overlap), fmax(0.0, input.parentHeight));

    return (CowboyKeyboardOverlapResult) {
        .overlap = overlap,
        .frameDepth = frameDepth,
        .frameReachesBottom = frameReachesBottom,
        .fullWidthDockCandidate = fullWidthDockCandidate,
    };
}

#endif
