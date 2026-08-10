#include <assert.h>
#include <math.h>
#include <stdio.h>

#include "CowboyKeyboardGeometry.h"

static void assertOverlap(
    CowboyKeyboardOverlapInput input,
    double expected
) {
    CowboyKeyboardOverlapResult result =
        cowboyKeyboardOverlapForNotification(input);
    assert(fabs(result.overlap - expected) < 0.001);
}

int main(void) {
    // Ordinary portrait keyboard attached to the current bottom edge.
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 973,
        .keyboardMaxY = 1376,
        .keyboardWidth = 1032,
        .keyboardHeight = 403,
    }, 403);

    // First presentation after rotation: the new full-width keyboard still
    // carries the previous orientation's bottom coordinate.
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 629,
        .keyboardMaxY = 1032,
        .keyboardWidth = 1032,
        .keyboardHeight = 403,
    }, 403);

    // Regression: conversion of an old-orientation rect swaps its axes and
    // overshoots the current bottom. The old code used 1376 - 480 = 896 and
    // shrank the WebView to 480pt; the real keyboard depth remains 403pt.
    CowboyKeyboardOverlapResult rotated =
        cowboyKeyboardOverlapForNotification((CowboyKeyboardOverlapInput) {
            .parentWidth = 1032,
            .parentHeight = 1376,
            .keyboardMinY = 480,
            .keyboardMaxY = 1512,
            .keyboardWidth = 403,
            .keyboardHeight = 1032,
        });
    assert(fabs(rotated.frameDepth - 403) < 0.001);
    assert(rotated.frameReachesBottom);
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 480,
        .keyboardMaxY = 1512,
        .keyboardWidth = 403,
        .keyboardHeight = 1032,
    }, 403);

    // A hidden keyboard remains below the viewport and removes no height.
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 1376,
        .keyboardMaxY = 1779,
        .keyboardWidth = 1032,
        .keyboardHeight = 403,
    }, 0);

    // Floating keyboards overlay the document; docked split keyboards resize.
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 500,
        .keyboardMaxY = 800,
        .keyboardWidth = 500,
        .keyboardHeight = 300,
    }, 0);
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 1032,
        .parentHeight = 1376,
        .keyboardMinY = 1000,
        .keyboardMaxY = 1376,
        .keyboardWidth = 720,
        .keyboardHeight = 376,
    }, 376);

    // Landscape phone keyboards may legitimately occupy more than half the
    // viewport; the short-edge bound preserves their full depth.
    assertOverlap((CowboyKeyboardOverlapInput) {
        .parentWidth = 844,
        .parentHeight = 390,
        .keyboardMinY = 130,
        .keyboardMaxY = 390,
        .keyboardWidth = 844,
        .keyboardHeight = 260,
    }, 260);

    puts("keyboard geometry tests passed");
    return 0;
}
