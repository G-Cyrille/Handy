import AppKit

// Keep every clipboard item/type in memory while an isolated paste test runs.
// No clipboard data is written to disk or printed.
let board = NSPasteboard.general
let saved = (board.pasteboardItems ?? []).map { item in
    item.types.compactMap { type in item.data(forType: type).map { (type, $0) } }
}
let restored = saved.map { representations in
    let item = NSPasteboardItem()
    for (type, data) in representations { item.setData(data, forType: type) }
    return item
}
board.clearContents()
board.setString("Handy meetings clipboard test", forType: .string)
let child = Process()
child.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
child.arguments = Array(CommandLine.arguments.dropFirst(2))
var status: Int32 = 1
do {
    try child.run()
    child.waitUntilExit()
    status = child.terminationStatus
} catch {
    fputs("Clipboard test runner failed\n", stderr)
}
board.clearContents()
if !restored.isEmpty { board.writeObjects(restored) }
exit(status)
