import SwiftUI
import AppKit

extension Notification.Name {
    static let convertSsfFile = Notification.Name("ConvertSsfFile")
}

class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Hide terminal window if launched from one
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)

        // CLI hand-off: any --file <path> or trailing .ssf positional argument
        // is converted automatically on launch — used for smoke-testing the
        // post-conversion popup without dragging a file in manually.
        let args = CommandLine.arguments.dropFirst()
        var pending: String? = nil
        var iter = args.makeIterator()
        while let a = iter.next() {
            if a == "--file" || a == "-f" {
                pending = iter.next()
            } else if a.hasSuffix(".ssf") {
                pending = a
            }
        }
        if let path = pending {
            let url = URL(fileURLWithPath: path)
            // Post after the window appears so ContentView's observer is wired.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                NotificationCenter.default.post(name: .convertSsfFile, object: url)
            }
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        if let url = urls.first {
            NotificationCenter.default.post(name: .convertSsfFile, object: url)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

@main
struct SSF2ConverterApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
        .windowResizability(.contentSize)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
    }
}
