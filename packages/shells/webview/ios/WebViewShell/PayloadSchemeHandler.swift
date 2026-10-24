import Foundation
import UniformTypeIdentifiers
import WebKit

/// Serves the payload, the files OmniBuild puts at the top of the app's
/// bundle, at `app://localhost/`. A page loaded from a file:// URL has no
/// origin of its own, so WebKit refuses it fetch, ES modules and storage.
/// From a scheme of the app's own it has one, `app://localhost`, and works
/// as it does on the web.
final class PayloadSchemeHandler: NSObject, WKURLSchemeHandler {
    /// A scheme no web page or system app uses. WebKit keeps http, https,
    /// file and its own for itself.
    static let scheme = "app"
    static let startURL = URL(string: "\(scheme)://localhost/index.html")!

    /// The web's own types for the files a web app is made of. WebKit
    /// refuses a module script that isn't served as JavaScript, and
    /// UTType's guesses for some of these differ.
    private static let webTypes: [String: String] = [
        "css": "text/css",
        "htm": "text/html",
        "html": "text/html",
        "js": "text/javascript",
        "json": "application/json",
        "map": "application/json",
        "mjs": "text/javascript",
        "svg": "image/svg+xml",
        "txt": "text/plain",
        "wasm": "application/wasm",
        "webmanifest": "application/manifest+json",
        "woff": "font/woff",
        "woff2": "font/woff2",
        "xml": "application/xml",
    ]

    /// The bundle's folder, where the payload is.
    private let root: URL

    init(root: URL) {
        self.root = root.standardizedFileURL
    }

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        guard let url = task.request.url else {
            task.didFailWithError(URLError(.badURL))
            return
        }
        // A folder's index.html, as a web server serves it.
        var path = url.path
        if path.isEmpty || path.hasSuffix("/") {
            path += "index.html"
        }
        let file = root.appendingPathComponent(path).standardizedFileURL
        // Nothing outside the bundle, which `..` in the path could reach.
        guard file.path.hasPrefix(root.path + "/"), let data = try? Data(contentsOf: file) else {
            respond(to: task, url: url, status: 404, type: "text/plain", data: Data())
            return
        }
        respond(to: task, url: url, status: 200, type: Self.mimeType(of: file), data: data)
    }

    /// Nothing to stop: each file is answered at once, in one piece.
    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {}

    private func respond(to task: any WKURLSchemeTask, url: URL, status: Int, type: String, data: Data) {
        let text = type.hasPrefix("text/") || type.hasSuffix("json") || type.hasSuffix("xml")
        let response = HTTPURLResponse(
            url: url,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: [
                "Content-Type": text ? "\(type); charset=utf-8" : type,
                "Content-Length": String(data.count),
                "Cache-Control": "no-cache",
            ]
        )!
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    private static func mimeType(of file: URL) -> String {
        let pathExtension = file.pathExtension.lowercased()
        return webTypes[pathExtension]
            ?? UTType(filenameExtension: pathExtension)?.preferredMIMEType
            ?? "application/octet-stream"
    }
}
