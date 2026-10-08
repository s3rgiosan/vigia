// Renders a page in WKWebView, the engine Tauri uses on macOS, and saves a PNG snapshot.
// System colors such as -apple-system-label resolve here exactly as they do in the app, which a
// Chromium preview cannot show.
//
// Usage: swift scripts/webkit-snapshot.swift <url> <width> <height> <out.png> <light|dark> [script]
// The optional script runs after load, for example to open a sheet before the snapshot.

import AppKit
import WebKit

let args = CommandLine.arguments
guard args.count >= 6 else {
    FileHandle.standardError.write("usage: webkit-snapshot <url> <width> <height> <out.png> <light|dark> [script]\n".data(using: .utf8)!)
    exit(2)
}
let url = URL(string: args[1])!
let width = Double(args[2])!
let height = Double(args[3])!
let out = URL(fileURLWithPath: args[4])
let dark = args[5] == "dark"
let script = args.count > 6 ? args[6] : nil

final class Snapper: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    let window: NSWindow

    init(width: Double, height: Double, dark: Bool) {
        let frame = NSRect(x: 0, y: 0, width: width, height: height)
        window = NSWindow(contentRect: frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        webView = WKWebView(frame: frame)
        window.contentView = webView
        super.init()
        webView.navigationDelegate = self
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // Give React time to render and fetch its mocked data.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
            if let script {
                webView.evaluateJavaScript(script) { result, error in
                    if let error { FileHandle.standardError.write("script error: \(error)\n".data(using: .utf8)!) }
                    if let result { print("script result: \(result)") }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { self.snap() }
                }
            } else {
                self.snap()
            }
        }
    }

    func snap() {
        webView.takeSnapshot(with: nil) { image, error in
            guard let image, let tiff = image.tiffRepresentation,
                  let rep = NSBitmapImageRep(data: tiff),
                  let png = rep.representation(using: .png, properties: [:]) else {
                FileHandle.standardError.write("snapshot failed: \(String(describing: error))\n".data(using: .utf8)!)
                exit(1)
            }
            try! png.write(to: out)
            exit(0)
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let snapper = Snapper(width: width, height: height, dark: dark)
snapper.webView.load(URLRequest(url: url))
app.run()
