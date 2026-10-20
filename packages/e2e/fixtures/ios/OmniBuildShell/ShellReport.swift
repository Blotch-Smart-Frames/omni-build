import Foundation
import ShellKit
import UIKit

/// What the app found in its own bundle: every value OmniBuild may rewrite.
/// The keys match the expected values in packages/e2e/src/shell.ts.
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
    /// Why the app asks for the camera, as iOS shows it.
    let cameraUsage: String?
    /// The URL schemes whose links open the app, which iOS reads from its
    /// Info.plist's URL types.
    let urlSchemes: String?
    /// The embedded framework's name, which the app only knows if the
    /// framework loaded.
    let framework: String
    /// The app extension's bundle ID, which iOS only installs the app with
    /// if it's under the app's.
    let extensionId: String?


    init(bundle: Bundle) {
        let info = bundle.infoDictionary ?? [:]
        // What iOS shows: the strings in the language it runs in
        // (InfoPlist.strings), where the shell has them, over Info.plist's.
        let shown = (bundle.localizedInfoDictionary ?? [:]).merging(info) { localized, _ in
            localized
        }
        id = bundle.bundleIdentifier
        // The name under the icon on the home screen.
        name = (shown["CFBundleDisplayName"] ?? shown["CFBundleName"]) as? String
        cameraUsage = shown["NSCameraUsageDescription"] as? String
        urlSchemes = (info["CFBundleURLTypes"] as? [[String: Any]])?
            .flatMap { $0["CFBundleURLSchemes"] as? [String] ?? [] }
            .joined(separator: ", ")
        framework = ShellKit.name
        extensionId = bundle.builtInPlugInsURL
            .flatMap {
                try? FileManager.default.contentsOfDirectory(
                    at: $0, includingPropertiesForKeys: nil)
            }?
            .first { $0.pathExtension == "appex" }
            .flatMap { Bundle(url: $0)?.bundleIdentifier }
        version = info["CFBundleShortVersionString"] as? String
        build = info["CFBundleVersion"] as? String
        payload = bundle.url(forResource: "payload", withExtension: "txt")
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        // The icon is in two places, which Info.plist names: Assets.car,
        // which the home screen draws it from, and the PNGs that Xcode
        // writes beside it for earlier versions of iOS. Both should be the
        // app's, so the report gives one colour when they agree, and both
        // when they don't.
        let icons = info["CFBundleIcons"] as? [String: Any]
        let primaryIcon = icons?["CFBundlePrimaryIcon"] as? [String: Any]
        let catalogIcon = (primaryIcon?["CFBundleIconName"] as? String)
            .flatMap { Self.catalogIcon(named: $0, in: bundle) }
            .flatMap(Self.centerColor)
        let fileIcon = (primaryIcon?["CFBundleIconFiles"] as? [String])?.last
            .flatMap { UIImage(named: $0) }
            .flatMap(Self.centerColor)
        icon =
            catalogIcon == fileIcon
            ? catalogIcon
            : "Assets.car: \(catalogIcon ?? "none"), PNGs: \(fileIcon ?? "none")"
    }

    /// The iPhone icon named `name` in the bundle's Assets.car, as CoreUI
    /// reads it for the home screen. No public API reads it:
    /// `UIImage(named: "AppIcon")` is nil, because actool stores an app
    /// icon as a kind of asset UIImage doesn't load. So this calls CoreUI's
    /// CUICatalog by name, which is fine in an app that's only ever built
    /// for these tests. A method CoreUI no longer has gives nil, and so a
    /// failed test, rather than a crash.
    static func catalogIcon(named name: String, in bundle: Bundle) -> UIImage? {
        guard let url = bundle.url(forResource: "Assets", withExtension: "car"),
            let catalogClass = NSClassFromString("CUICatalog") as? NSObject.Type
        else {
            return nil
        }
        // `[[CUICatalog alloc] initWithURL:error:]`. init takes over the
        // reference alloc returns, so Swift mustn't release it too.
        let allocated =
            catalogClass.perform(NSSelectorFromString("alloc")).takeUnretainedValue() as! NSObject
        let initialize = NSSelectorFromString("initWithURL:error:")
        typealias Initialize = @convention(c) (
            NSObject, Selector, NSURL, UnsafeMutablePointer<NSError?>?
        ) -> Unmanaged<NSObject>?
        guard allocated.responds(to: initialize),
            let catalog = unsafeBitCast(
                allocated.method(for: initialize), to: Initialize.self
            )(allocated, initialize, url as NSURL, nil)?.takeRetainedValue()
        else {
            return nil
        }
        // An iPhone's icon (idiom 1), as large as it comes: the shell's
        // single-size icon is 1024×1024, at 1x. The other attributes are
        // left to match any.
        let lookup = NSSelectorFromString(
            "iconImageWithName:scaleFactor:deviceIdiom:deviceSubtype:displayGamut:layoutDirection:sizeClassHorizontal:sizeClassVertical:desiredSize:"
        )
        typealias Lookup = @convention(c) (
            NSObject, Selector, NSString, CGFloat, Int, Int, Int, Int, Int, Int, CGSize
        ) -> Unmanaged<NSObject>?
        let image = NSSelectorFromString("image")
        typealias Image = @convention(c) (NSObject, Selector) -> Unmanaged<CGImage>?
        guard catalog.responds(to: lookup),
            let named = unsafeBitCast(catalog.method(for: lookup), to: Lookup.self)(
                catalog, lookup, name as NSString, 1, 1, 0, 0, 0, 0, 0,
                CGSize(width: 1024, height: 1024)
            )?.takeUnretainedValue(),
            named.responds(to: image),
            let cgImage = unsafeBitCast(named.method(for: image), to: Image.self)(named, image)?
                .takeUnretainedValue()
        else {
            return nil
        }
        return UIImage(cgImage: cgImage)
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
