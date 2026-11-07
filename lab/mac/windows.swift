import CoreGraphics
import Foundation
// The windows of a running Immich Mount: where they are, how big, on screen or not. Bounds and layers only, no names
// or images, so it needs no Screen Recording permission.
let all = (CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]]) ?? []
for window in all where (window[kCGWindowOwnerName as String] as? String) == "Immich Mount" {
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    print("pid", window[kCGWindowOwnerPID as String] ?? "?", "layer", window[kCGWindowLayer as String] ?? "?",
          "onscreen", window[kCGWindowIsOnscreen as String] ?? false, "alpha", window[kCGWindowAlpha as String] ?? "?",
          "x", bounds["X"] ?? "?", "y", bounds["Y"] ?? "?", "w", bounds["Width"] ?? "?", "h", bounds["Height"] ?? "?")
}
