import AppKit
import AVFoundation

final class FixturePlayer: NSObject, NSApplicationDelegate {
    private var player: AVAudioPlayer?
    private var window: NSWindow?
    private var statusTimer: Timer?

    private func writeStatus() {
        guard let player else { return }
        player.updateMeters()
        let status: [String: Any] = [
            "wall_time": Date().timeIntervalSince1970, "pid": ProcessInfo.processInfo.processIdentifier,
            "is_playing": player.isPlaying, "current_time": player.currentTime,
            "duration": player.duration, "volume": player.volume,
            "peak_db": player.peakPower(forChannel: 0),
        ]
        do {
            let data = try JSONSerialization.data(withJSONObject: status)
            try data.write(to: URL(fileURLWithPath: "/private/tmp/handy-meetings-fixtures/player-status.json"), options: .atomic)
        } catch {
            fputs("Fixture status failed: \(error)\n", stderr)
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            let url = URL(fileURLWithPath: "/private/tmp/handy-meetings-fixtures/french.wav")
            let player = try AVAudioPlayer(contentsOf: url)
            player.numberOfLoops = -1
            player.isMeteringEnabled = true
            player.prepareToPlay()
            guard player.play() else { throw NSError(domain: "FixturePlayer", code: 1) }
            self.player = player
            writeStatus()
            statusTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.writeStatus() }
            let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 400, height: 200),
                styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
            window.title = "Handy Fixture Player"
            let label = NSTextField(labelWithString: "Fixture française en boucle · 45 secondes")
            label.frame = NSRect(x: 24, y: 150, width: 360, height: 24)
            window.contentView?.addSubview(label)
            let editor = NSTextView(frame: NSRect(x: 24, y: 24, width: 352, height: 110))
            editor.string = "Dictation test target"
            editor.setAccessibilityIdentifier("handy-fixture-dictation-target")
            window.contentView?.addSubview(editor)
            window.makeKeyAndOrderFront(nil)
            self.window = window
            let started = Date().timeIntervalSince1970
            try "{\"started_at_seconds\":\(started),\"loop_seconds\":\(player.duration)}\n"
                .write(toFile: "/private/tmp/handy-meetings-fixtures/player-start.json", atomically: true, encoding: .utf8)
            let menu = NSMenu()
            let root = NSMenuItem()
            let actions = NSMenu()
            actions.addItem(withTitle: "Quit Fixture Player", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
            root.submenu = actions
            menu.addItem(root)
            NSApp.mainMenu = menu
        } catch {
            fputs("Fixture player failed: \(error)\n", stderr)
            NSApp.terminate(nil)
        }
    }
}

let app = NSApplication.shared
let delegate = FixturePlayer()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
