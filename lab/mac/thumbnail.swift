// Ask QuickLook for a thumbnail the way Finder does (QLThumbnailGenerator), for lab/mac/check.sh: prints
// "thumbnail <width>x<height>", or why there is none.
//
//   xcrun swift lab/mac/thumbnail.swift <file>

import Foundation
import QuickLookThumbnailing

let url = URL(fileURLWithPath: CommandLine.arguments[1])
let request = QLThumbnailGenerator.Request(fileAt: url, size: CGSize(width: 256, height: 256), scale: 2,
                                           representationTypes: .thumbnail)
let done = DispatchSemaphore(value: 0)
QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { thumbnail, error in
    if let thumbnail {
        print("thumbnail \(thumbnail.cgImage.width)x\(thumbnail.cgImage.height)")
    } else {
        print("no thumbnail: \(error.map { String(describing: $0) } ?? "none given")")
    }
    done.signal()
}
_ = done.wait(timeout: .now() + 60)
