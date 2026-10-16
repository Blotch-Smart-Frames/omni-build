import Foundation
import UIKit

/// What the app found in its own bundle: every value OmniBuild may rewrite.
/// The keys match the expected values in packages/e2e/src/shell.js.
struct ShellReport: Encodable {
    /// The file the harness reads, in the app's Documents folder. A file
    /// rather than the log, because the simulator's data container is a
    /// folder on the Mac, which the harness can read without any tool.
    static let fileName = "omni-build-e2e.json"

    let id: String?
    let name: String?
    let version: String?
    let build: String?
    let payload: String?
    let icon: String?

    init(bundle: Bundle) {
        let info = bundle.infoDictionary ?? [:]
        id = bundle.bundleIdentifier
        // The name under the icon on the home screen.
        name = (info["CFBundleDisplayName"] ?? info["CFBundleName"]) as? String
        version = info["CFBundleShortVersionString"] as? String
        build = info["CFBundleVersion"] as? String
        payload = bundle.url(forResource: "payload", withExtension: "txt")
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        // The icon as UIKit can load it: the PNGs that Xcode writes next to
        // Assets.car, named in Info.plist. The home screen shows the copy in
        // Assets.car, which no public API reads (`UIImage(named: "AppIcon")`
        // is nil: actool stores app icons as a kind of asset UIImage doesn't
        // load), so that one can't be checked from inside the app.
        let icons = info["CFBundleIcons"] as? [String: Any]
        let primaryIcon = icons?["CFBundlePrimaryIcon"] as? [String: Any]
        icon = (primaryIcon?["CFBundleIconFiles"] as? [String])?.last
            .flatMap { UIImage(named: $0) }
            .flatMap(Self.centerColor)
    }

    func save() {
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            let documents = try FileManager.default.url(
                for: .documentDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            )
            // Atomic, so the harness, which polls for the file, never reads
            // half of it.
            try encoder.encode(self).write(
                to: documents.appendingPathComponent(Self.fileName),
                options: .atomic
            )
        } catch {
            // In the simulator's log, which the harness saves when the
            // report never arrives.
            NSLog("OmniBuildE2E: couldn't save the report: \(error)")
        }
    }

    /// The colour of the image's centre pixel as `#RRGGBB`. The fixture icons
    /// are solid colours, so this tells the harness which icon the app got
    /// without comparing images.
    static func centerColor(of image: UIImage) -> String? {
        guard let cgImage = image.cgImage,
            let sRGB = CGColorSpace(name: CGColorSpace.sRGB)
        else {
            return nil
        }
        var pixel: [UInt8] = [0, 0, 0, 0]
        let drawn = pixel.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress,
                    width: 1,
                    height: 1,
                    bitsPerComponent: 8,
                    bytesPerRow: 4,
                    space: sRGB,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                )
            else {
                return false
            }
            // Offset the image so that its centre pixel is the context's
            // only one.
            let width = CGFloat(cgImage.width)
            let height = CGFloat(cgImage.height)
            context.draw(
                cgImage,
                in: CGRect(
                    x: -(width / 2).rounded(.down),
                    y: -(height / 2).rounded(.down),
                    width: width,
                    height: height
                )
            )
            return true
        }
        guard drawn else {
            return nil
        }
        return String(format: "#%02X%02X%02X", pixel[0], pixel[1], pixel[2])
    }
}
