import AppKit
import ApplicationServices
import Carbon
import CoreGraphics
import Foundation
import Vision

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

let arguments = CommandLine.arguments
if arguments.count < 3 {
    fail("Usage: drive-window <pid> doctor|windows|ocr|ocr-boxes|pixel|sample|pixel-range|type|paste|paste-run|key|click|hover|drag|resize|position|activate|active|english [arguments]")
}
guard let pid = Int32(arguments[1]) else { fail("Invalid application PID") }
let command = arguments[2]

if command == "doctor" {
    guard AXIsProcessTrusted() else { fail("Enable Accessibility for the terminal running this driver in System Settings") }
    guard CGPreflightScreenCaptureAccess() else { fail("Enable Screen Recording for the terminal running this driver in System Settings") }
    print("Accessibility and screen capture are available")
    exit(0)
}
if command == "ocr" {
    guard arguments.count == 4 || arguments.count == 8 else { fail("ocr requires an image path and optional normalized x y width height") }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["en-US", "zh-Hans"]
    request.usesLanguageCorrection = false
    if arguments.count == 8 {
        guard let x = Double(arguments[4]), let y = Double(arguments[5]),
              let width = Double(arguments[6]), let height = Double(arguments[7]),
              [x, y, width, height].allSatisfy({ $0.isFinite }),
              x >= 0, y >= 0, width > 0, height > 0,
              x + width <= 1, y + height <= 1 else { fail("OCR region must fit within the image") }
        // Driver coordinates start at the top left; Vision starts at the bottom left.
        request.regionOfInterest = CGRect(x: x, y: max(0, 1 - y - height), width: width, height: height)
    }
    do {
        try VNImageRequestHandler(url: URL(fileURLWithPath: arguments[3])).perform([request])
        for observation in request.results ?? [] {
            if let text = observation.topCandidates(1).first?.string { print(text) }
        }
    } catch { fail("OCR failed: \(error)") }
    exit(0)
}
if command == "ocr-boxes" {
    guard [4, 5, 9].contains(arguments.count) else { fail("ocr-boxes requires an image path, optional exact substring, and optional normalized x y width height") }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["en-US", "zh-Hans"]
    request.usesLanguageCorrection = false
    if arguments.count == 9 {
        guard let x = Double(arguments[5]), let y = Double(arguments[6]),
              let width = Double(arguments[7]), let height = Double(arguments[8]),
              [x, y, width, height].allSatisfy({ $0.isFinite }),
              x >= 0, y >= 0, width > 0, height > 0,
              x + width <= 1, y + height <= 1 else { fail("OCR region must fit within the image") }
        request.regionOfInterest = CGRect(x: x, y: max(0, 1 - y - height), width: width, height: height)
    }
    do {
        try VNImageRequestHandler(url: URL(fileURLWithPath: arguments[3])).perform([request])
        for observation in request.results ?? [] {
            guard let candidate = observation.topCandidates(1).first else { continue }
            var text = candidate.string
            var box = observation.boundingBox
            if arguments.count >= 5 && !arguments[4].isEmpty {
                guard let range = text.range(of: arguments[4], options: .caseInsensitive),
                      let substringBox = try candidate.boundingBox(for: range) else { continue }
                text = String(text[range])
                box = substringBox.boundingBox
            }
            if arguments.count == 9 {
                let region = request.regionOfInterest
                box = CGRect(x: region.minX + box.minX * region.width,
                             y: region.minY + box.minY * region.height,
                             width: box.width * region.width, height: box.height * region.height)
            }
            print("\(text)\t\(box.origin.x)\t\(box.origin.y)\t\(box.size.width)\t\(box.size.height)")
        }
    } catch { fail("OCR failed: \(error)") }
    exit(0)
}
if command == "pixel" {
    guard arguments.count == 6 else { fail("pixel requires an image path and normalized x y") }
    guard let nx = Double(arguments[4]), let ny = Double(arguments[5]),
          nx.isFinite, ny.isFinite, nx >= 0, nx <= 1, ny >= 0, ny <= 1 else {
        fail("Pixel coordinates must be normalized within the image")
    }
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: arguments[3])),
          let bitmap = NSBitmapImageRep(data: data),
          bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else { fail("Could not decode image") }
    let x = min(bitmap.pixelsWide - 1, Int(nx * Double(bitmap.pixelsWide)))
    let y = min(bitmap.pixelsHigh - 1, Int(ny * Double(bitmap.pixelsHigh)))
    guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else {
        fail("Image has no readable pixel")
    }
    print([color.redComponent, color.greenComponent, color.blueComponent, color.alphaComponent]
        .map { String(Int(($0 * 255).rounded())) }.joined(separator: " "))
    exit(0)
}
if command == "sample" {
    guard arguments.count == 7 else { fail("sample requires an image path, normalized x y, and normalized half-size") }
    guard let nx = Double(arguments[4]), let ny = Double(arguments[5]),
          let half = Double(arguments[6]), [nx, ny, half].allSatisfy({ $0.isFinite }),
          nx >= 0, nx <= 1, ny >= 0, ny <= 1, half > 0, half <= 1 else {
        fail("Sample coordinates must be normalized within the image")
    }
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: arguments[3])),
          let bitmap = NSBitmapImageRep(data: data),
          bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else { fail("Could not decode image") }
    let centerX = min(bitmap.pixelsWide - 1, max(0, Int(nx * Double(bitmap.pixelsWide))))
    let centerY = min(bitmap.pixelsHigh - 1, max(0, Int(ny * Double(bitmap.pixelsHigh))))
    let radius = max(1, Int(half * Double(min(bitmap.pixelsWide, bitmap.pixelsHigh))))
    // The icon stroke is antialiased over a uniform panel, so the most
    // saturated pixel in a small box identifies its tint reliably.
    var best = (spread: -1.0, red: 0.0, green: 0.0, blue: 0.0)
    for y in max(0, centerY - radius)...min(bitmap.pixelsHigh - 1, centerY + radius) {
        for x in max(0, centerX - radius)...min(bitmap.pixelsWide - 1, centerX + radius) {
            // Both driver coordinates and NSBitmapImageRep's pixel access start
            // at the top left, so no vertical flip is needed.
            guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
            let red = color.redComponent * 255, green = color.greenComponent * 255, blue = color.blueComponent * 255
            let spread = max(red, green, blue) - min(red, green, blue)
            if spread > best.spread { best = (spread, red, green, blue) }
        }
    }
    guard best.spread >= 0 else { fail("Image has no readable pixels") }
    print(String(format: "%02x%02x%02x", Int(best.red.rounded()), Int(best.green.rounded()), Int(best.blue.rounded())))
    exit(0)
}
if command == "pixel-range" {
    guard arguments.count == 7 else { fail("pixel-range requires an image path, normalized x y, and normalized half-size") }
    guard let nx = Double(arguments[4]), let ny = Double(arguments[5]),
          let half = Double(arguments[6]), [nx, ny, half].allSatisfy({ $0.isFinite }),
          nx >= 0, nx <= 1, ny >= 0, ny <= 1, half > 0, half <= 1 else {
        fail("Pixel-range coordinates must be normalized within the image")
    }
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: arguments[3])),
          let bitmap = NSBitmapImageRep(data: data),
          bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else { fail("Could not decode image") }
    let centerX = min(bitmap.pixelsWide - 1, max(0, Int(nx * Double(bitmap.pixelsWide))))
    let centerY = min(bitmap.pixelsHigh - 1, max(0, Int(ny * Double(bitmap.pixelsHigh))))
    let radius = max(1, Int(half * Double(min(bitmap.pixelsWide, bitmap.pixelsHigh))))
    var minimum = 255.0, maximum = 0.0
    for y in max(0, centerY - radius)...min(bitmap.pixelsHigh - 1, centerY + radius) {
        for x in max(0, centerX - radius)...min(bitmap.pixelsWide - 1, centerX + radius) {
            guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
            let luminance = (color.redComponent * 0.299 + color.greenComponent * 0.587
                             + color.blueComponent * 0.114) * 255
            minimum = min(minimum, luminance)
            maximum = max(maximum, luminance)
        }
    }
    print("\(Int(minimum.rounded())) \(Int(maximum.rounded()))")
    exit(0)
}
if command == "pixel-bounds" {
    guard arguments.count == 11 else { fail("pixel-bounds requires an image path, normalized x y width height, a green or ink mode, and normalized background x y") }
    guard let nx = Double(arguments[4]), let ny = Double(arguments[5]),
          let nwidth = Double(arguments[6]), let nheight = Double(arguments[7]),
          let backgroundX = Double(arguments[9]), let backgroundY = Double(arguments[10]),
          [nx, ny, nwidth, nheight, backgroundX, backgroundY].allSatisfy({ $0.isFinite }),
          nx >= 0, ny >= 0, nwidth > 0, nheight > 0, nx + nwidth <= 1, ny + nheight <= 1,
          backgroundX >= 0, backgroundX <= 1, backgroundY >= 0, backgroundY <= 1 else {
        fail("Pixel-bounds coordinates must be normalized within the image")
    }
    let mode = arguments[8]
    guard mode == "green" || mode == "ink" else { fail("Pixel-bounds mode must be green or ink") }
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: arguments[3])),
          let bitmap = NSBitmapImageRep(data: data),
          bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else { fail("Could not decode image") }
    let left = Int(nx * Double(bitmap.pixelsWide))
    let top = Int(ny * Double(bitmap.pixelsHigh))
    let right = min(bitmap.pixelsWide, Int((nx + nwidth) * Double(bitmap.pixelsWide)))
    let bottom = min(bitmap.pixelsHigh, Int((ny + nheight) * Double(bitmap.pixelsHigh)))
    let sampleX = min(bitmap.pixelsWide - 1, max(0, Int(backgroundX * Double(bitmap.pixelsWide))))
    let sampleY = min(bitmap.pixelsHigh - 1, max(0, Int(backgroundY * Double(bitmap.pixelsHigh))))
    guard let background = bitmap.colorAt(x: sampleX, y: sampleY)?.usingColorSpace(.sRGB) else {
        fail("Could not sample pixel-bounds background")
    }
    var minimumX = bitmap.pixelsWide, maximumX = -1, minimumY = bitmap.pixelsHigh, maximumY = -1
    for y in top..<bottom {
        for x in left..<right {
            guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
            let red = color.redComponent * 255, green = color.greenComponent * 255, blue = color.blueComponent * 255
            let matches: Bool
            if mode == "green" {
                matches = green > red + 15 && green > blue + 10
            } else {
                let difference = max(abs(red - background.redComponent * 255),
                                     max(abs(green - background.greenComponent * 255),
                                         abs(blue - background.blueComponent * 255)))
                matches = difference >= 40
            }
            if matches {
                minimumX = min(minimumX, x)
                maximumX = max(maximumX, x)
                minimumY = min(minimumY, y)
                maximumY = max(maximumY, y)
            }
        }
    }
    guard maximumX >= 0, maximumY >= 0 else { fail("No pixels matched \(mode) in the requested region") }
    print("\(minimumX) \(minimumY) \(maximumX + 1) \(maximumY + 1)")
    exit(0)
}
guard let application = NSRunningApplication(processIdentifier: pid) else {
    fail("Application is not running")
}

func firstWindow() -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success,
          let windows = value as? [AXUIElement], let first = windows.first else {
        fail("Cannot inspect application window")
    }
    return first
}

func activateTarget() throws {
    application.unhide()
    application.activate(options: [])
    let deadline = Date().addingTimeInterval(3)
    while NSWorkspace.shared.frontmostApplication?.processIdentifier != pid {
        if Date() > deadline { throw NSError(domain: "WindowDriver", code: 1, userInfo: [NSLocalizedDescriptionKey: "Target did not become frontmost"]) }
        Thread.sleep(forTimeInterval: 0.05)
    }
    Thread.sleep(forTimeInterval: 0.15)
}

func mouseTarget(at point: CGPoint) throws -> Int {
    guard point.x.isFinite, point.y.isFinite,
          NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        throw NSError(domain: "WindowDriver", code: 5, userInfo: [NSLocalizedDescriptionKey: "Target lost focus; no mouse event sent"])
    }
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let target = windows.first { entry in
        guard (entry[kCGWindowLayer as String] as? Int) == 0,
              let bounds = entry[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return false }
        return rect.contains(point)
    }
    guard (target?[kCGWindowOwnerPID as String] as? Int32) == pid,
          let number = target?[kCGWindowNumber as String] as? Int else {
        throw NSError(domain: "WindowDriver", code: 6, userInfo: [NSLocalizedDescriptionKey: "Mouse point is not on the target application's frontmost window"])
    }
    return number
}

func mouseEvent(_ type: CGEventType, at point: CGPoint) -> CGEvent {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: (type == .rightMouseDown || type == .rightMouseUp) ? .right : .left) else { fail("Cannot create mouse event") }
    event.flags = []
    event.setIntegerValueField(.mouseEventClickState, value: 1)
    return event
}

func selectEnglish() throws {
    let query = [kTISPropertyInputSourceID as String: "com.apple.keylayout.ABC"] as CFDictionary
    let sources = TISCreateInputSourceList(query, false).takeRetainedValue() as! [TISInputSource]
    guard let source = sources.first, TISSelectInputSource(source) == noErr else {
        throw NSError(domain: "WindowDriver", code: 2, userInfo: [NSLocalizedDescriptionKey: "Enable the ABC input source in System Settings"])
    }
}

func press(_ code: UInt16, modifiers: [String] = []) throws {
    let names = ["command": "command down", "control": "control down", "shift": "shift down", "option": "option down"]
    let mapped = try modifiers.map { modifier -> String in
        guard let name = names[modifier] else {
            throw NSError(domain: "WindowDriver", code: 3, userInfo: [NSLocalizedDescriptionKey: "Unknown modifier: \(modifier)"])
        }
        return name
    }
    if code == 13 && Set(modifiers) == Set(["command", "shift"]) {
        // System Events drops Cmd-Shift-W on this host. Use a native event for
        // that shortcut while retaining text-aware dispatch for other keys.
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
            throw NSError(domain: "WindowDriver", code: 4, userInfo: [NSLocalizedDescriptionKey: "Target lost focus; no key sent"])
        }
        let flags: [String: CGEventFlags] = ["command": .maskCommand, "control": .maskControl,
                                            "shift": .maskShift, "option": .maskAlternate]
        let combined = modifiers.reduce(CGEventFlags()) { $0.union(flags[$1]!) }
        for down in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) else {
                fail("Cannot create command key event")
            }
            event.flags = combined
            event.postToPid(pid)
            Thread.sleep(forTimeInterval: 0.05)
        }
        Thread.sleep(forTimeInterval: 0.15)
        return
    }
    let suffix = mapped.isEmpty ? "" : " using {\(mapped.joined(separator: ", "))}"
    // System Events reliably supplies characters for Return/Escape to AppKit.
    let script = """
    tell application "System Events"
        set foregroundProcess to first application process whose frontmost is true
        set foregroundPID to unix id of foregroundProcess
        if foregroundPID is not \(pid) then error "Target lost focus; no key sent (expected PID \(pid), got " & foregroundPID & ", key \(code))"
        key code \(code)\(suffix)
    end tell
    """
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
    process.arguments = ["-e", script]
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw NSError(domain: "WindowDriver", code: 4, userInfo: [NSLocalizedDescriptionKey: "Key delivery failed"])
    }
    Thread.sleep(forTimeInterval: 0.15)
}

do {
    switch command {
    case "english":
        try activateTarget()
        try selectEnglish()
        print("Selected ABC input source")
    case "windows":
        let entries = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        let windows = entries.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }
        let data = try JSONSerialization.data(withJSONObject: windows, options: [.prettyPrinted, .sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    case "type":
        guard arguments.count == 4 else { fail("type requires one text argument") }
        try activateTarget()
        try selectEnglish()
        let text = Array(arguments[3].utf16)
        guard let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true) else { fail("Cannot create text event") }
        event.flags = []
        text.withUnsafeBufferPointer { buffer in
            event.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: buffer.baseAddress)
        }
        event.postToPid(pid)
        event.type = .keyUp
        event.postToPid(pid)
    case "paste", "paste-run":
        guard arguments.count == 4 else { fail("paste requires one text argument") }
        try activateTarget()
        try selectEnglish()
        let pasteboard = NSPasteboard.general
        let savedItems = (pasteboard.pasteboardItems ?? []).map { item in
            item.types.compactMap { type -> (NSPasteboard.PasteboardType, Data)? in
                guard let data = item.data(forType: type) else { return nil }
                return (type, data)
            }
        }
        pasteboard.clearContents()
        pasteboard.setString(arguments[3], forType: .string)
        let temporaryChange = pasteboard.changeCount
        defer {
            // Do not replace a newer clipboard value from the user or another app.
            if pasteboard.changeCount == temporaryChange {
                pasteboard.clearContents()
                let restored = savedItems.map { values in
                    let item = NSPasteboardItem()
                    for (type, data) in values { item.setData(data, forType: type) }
                    return item
                }
                pasteboard.writeObjects(restored)
            }
        }
        try press(9, modifiers: ["command"])
        Thread.sleep(forTimeInterval: 0.35)
        if command == "paste-run" { try press(36) }
    case "copy-read":
        try activateTarget()
        let pasteboard = NSPasteboard.general
        let savedItems = (pasteboard.pasteboardItems ?? []).map { item in
            item.types.compactMap { type -> (NSPasteboard.PasteboardType, Data)? in
                guard let data = item.data(forType: type) else { return nil }
                return (type, data)
            }
        }
        let before = pasteboard.changeCount
        try press(8, modifiers: ["command"])
        Thread.sleep(forTimeInterval: 0.35)
        guard pasteboard.changeCount != before,
              let text = pasteboard.string(forType: .string) else { fail("Target did not copy text") }
        let copiedChange = pasteboard.changeCount
        print(text)
        if pasteboard.changeCount == copiedChange {
            pasteboard.clearContents()
            let restored = savedItems.map { values in
                let item = NSPasteboardItem()
                for (type, data) in values { item.setData(data, forType: type) }
                return item
            }
            pasteboard.writeObjects(restored)
        }
    case "key":
        guard arguments.count >= 4, let keyCode = UInt16(arguments[3]) else { fail("key requires a macOS virtual key code") }
        try activateTarget()
        try selectEnglish()
        try press(keyCode, modifiers: Array(arguments.dropFirst(4)))
    case "menu-titles":
        let app = AXUIElementCreateApplication(pid)
        var menu: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXMenuBarAttribute as CFString, &menu) == .success,
              let menu else { fail("Cannot inspect target menu bar") }
        var children: CFTypeRef?
        guard AXUIElementCopyAttributeValue(menu as! AXUIElement, kAXChildrenAttribute as CFString, &children) == .success,
              let items = children as? [AXUIElement] else { fail("Cannot inspect menu titles") }
        for item in items {
            var title: CFTypeRef?
            if AXUIElementCopyAttributeValue(item, kAXTitleAttribute as CFString, &title) == .success,
               let text = title as? String { print(text) }
        }
    case "repeat-key":
        guard arguments.count == 5, let code = UInt16(arguments[3]),
              let count = Int(arguments[4]), (2...100).contains(count) else { fail("repeat-key requires key code and count 2...100") }
        try activateTarget()
        try selectEnglish()
        guard let release = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { fail("Cannot create key release") }
        release.flags = []
        defer { release.postToPid(pid) }
        for index in 0..<count {
            guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
                throw NSError(domain: "WindowDriver", code: 4, userInfo: [NSLocalizedDescriptionKey: "Target lost focus; no repeated key sent"])
            }
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true) else {
                throw NSError(domain: "WindowDriver", code: 4, userInfo: [NSLocalizedDescriptionKey: "Cannot create repeated key"])
            }
            event.flags = []
            event.setIntegerValueField(.keyboardEventAutorepeat, value: index == 0 ? 0 : 1)
            event.post(tap: .cghidEventTap)
            Thread.sleep(forTimeInterval: index == 0 ? 0.5 : 0.04)
        }
    case "hover":
        guard arguments.count == 5, let x = Double(arguments[3]), let y = Double(arguments[4]) else { fail("hover requires screen x and y") }
        try activateTarget()
        let point = CGPoint(x: x, y: y)
        let target = try mouseTarget(at: point)
        guard CGWarpMouseCursorPosition(point) == .success else { fail("Cannot move the pointer") }
        guard try mouseTarget(at: point) == target else { fail("Target window changed during hover") }
        mouseEvent(.mouseMoved, at: point).post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.1)
    case "click", "right-click":
        guard arguments.count == 5, let x = Double(arguments[3]), let y = Double(arguments[4]) else { fail("click requires screen x and y") }
        try activateTarget()
        let point = CGPoint(x: x, y: y)
        let target = try mouseTarget(at: point)
        let down: CGEventType = command == "right-click" ? .rightMouseDown : .leftMouseDown
        let up: CGEventType = command == "right-click" ? .rightMouseUp : .leftMouseUp
        var pressed = false
        defer {
            if pressed { mouseEvent(up, at: point).postToPid(pid) }
        }
        // AppKit needs the global mouse location and a click count to resolve GPUI hit targets.
        for type: CGEventType in [.mouseMoved, down, up] {
            guard try mouseTarget(at: point) == target else { throw NSError(domain: "WindowDriver", code: 7, userInfo: [NSLocalizedDescriptionKey: "Target window changed during click"]) }
            mouseEvent(type, at: point).post(tap: .cghidEventTap)
            if type == down { pressed = true }
            if type == up { pressed = false }
            Thread.sleep(forTimeInterval: 0.05)
        }
    case "drag", "select-read":
        guard (7...8).contains(arguments.count),
              let x1 = Double(arguments[3]), let y1 = Double(arguments[4]),
              let x2 = Double(arguments[5]), let y2 = Double(arguments[6]),
              arguments.count == 7 || arguments[7] == "shift" else { fail("drag requires start x y and end x y, with optional shift") }
        try activateTarget()
        let captureCopy = command == "select-read"
        let pasteboard = NSPasteboard.general
        let savedItems = captureCopy ? (pasteboard.pasteboardItems ?? []).map { item in
            item.types.compactMap { type -> (NSPasteboard.PasteboardType, Data)? in
                guard let data = item.data(forType: type) else { return nil }
                return (type, data)
            }
        } : []
        var temporaryChange: Int?
        if captureCopy {
            pasteboard.clearContents()
            pasteboard.setString("MONOTERM_SELECTION_SENTINEL", forType: .string)
            temporaryChange = pasteboard.changeCount
        }
        defer {
            if let temporaryChange, pasteboard.changeCount == temporaryChange {
                pasteboard.clearContents()
                let restored = savedItems.map { values in
                    let item = NSPasteboardItem()
                    for (type, data) in values { item.setData(data, forType: type) }
                    return item
                }
                pasteboard.writeObjects(restored)
            }
        }
        let dragFlags: CGEventFlags = arguments.count == 8 ? .maskShift : []
        func dragEvent(_ type: CGEventType, at point: CGPoint) -> CGEvent {
            let event = mouseEvent(type, at: point)
            event.flags = dragFlags
            return event
        }
        let points = (0...20).map { step -> CGPoint in
            let fraction = Double(step) / 20
            return CGPoint(x: x1 + (x2 - x1) * fraction, y: y1 + (y2 - y1) * fraction)
        }
        let target = try mouseTarget(at: points[0])
        // Refuse a route that leaves this window, including any overlaid window.
        for point in points {
            guard try mouseTarget(at: point) == target else { throw NSError(domain: "WindowDriver", code: 7, userInfo: [NSLocalizedDescriptionKey: "Drag must stay within one target window"]) }
        }
        var pressed = false
        var current = points[0]
        defer {
            // Release only this PID if a focus/geometry guard interrupts the drag.
            if pressed { dragEvent(.leftMouseUp, at: current).postToPid(pid) }
        }
        for type: CGEventType in [.mouseMoved, .leftMouseDown] {
            guard try mouseTarget(at: current) == target else { throw NSError(domain: "WindowDriver", code: 7, userInfo: [NSLocalizedDescriptionKey: "Target window changed during drag"]) }
            dragEvent(type, at: current).post(tap: .cghidEventTap)
            if type == .leftMouseDown { pressed = true }
            Thread.sleep(forTimeInterval: 0.05)
        }
        for point in points.dropFirst() {
            guard try mouseTarget(at: point) == target else { throw NSError(domain: "WindowDriver", code: 7, userInfo: [NSLocalizedDescriptionKey: "Target window changed during drag"]) }
            current = point
            dragEvent(.leftMouseDragged, at: current).post(tap: .cghidEventTap)
            Thread.sleep(forTimeInterval: 0.025)
        }
        guard try mouseTarget(at: current) == target else { throw NSError(domain: "WindowDriver", code: 7, userInfo: [NSLocalizedDescriptionKey: "Target window changed during drag"]) }
        if captureCopy, pasteboard.changeCount != temporaryChange {
            throw NSError(domain: "WindowDriver", code: 8, userInfo: [NSLocalizedDescriptionKey: "Clipboard changed before selection completed"])
        }
        dragEvent(.leftMouseUp, at: current).post(tap: .cghidEventTap)
        pressed = false
        if captureCopy {
            Thread.sleep(forTimeInterval: 0.15)
            temporaryChange = pasteboard.changeCount
            print(pasteboard.string(forType: .string) ?? "")
        }
    case "resize":
        guard arguments.count == 5, let width = Double(arguments[3]), let height = Double(arguments[4]) else { fail("resize requires width and height") }
        var size = CGSize(width: width, height: height)
        guard let value = AXValueCreate(.cgSize, &size) else { fail("Cannot create size value") }
        let result = AXUIElementSetAttributeValue(firstWindow(), kAXSizeAttribute as CFString, value)
        if result != .success { fail("Resize failed: \(result.rawValue)") }
    case "resize-titled", "position-titled":
        guard arguments.count == 6, let first = Double(arguments[3]), let second = Double(arguments[4]) else { fail("Titled geometry requires two coordinates and an exact title") }
        let app = AXUIElementCreateApplication(pid)
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success,
              let windows = value as? [AXUIElement] else { fail("Cannot inspect application windows") }
        let matches = windows.filter { window in
            var title: CFTypeRef?
            return AXUIElementCopyAttributeValue(window, kAXTitleAttribute as CFString, &title) == .success
                && (title as? String) == arguments[5]
        }
        guard matches.count == 1 else { fail("Expected exactly one owned window with the supplied title") }
        let result: AXError
        if command == "resize-titled" {
            var size = CGSize(width: first, height: second)
            guard let value = AXValueCreate(.cgSize, &size) else { fail("Cannot create size value") }
            result = AXUIElementSetAttributeValue(matches[0], kAXSizeAttribute as CFString, value)
        } else {
            var point = CGPoint(x: first, y: second)
            guard let value = AXValueCreate(.cgPoint, &point) else { fail("Cannot create position value") }
            result = AXUIElementSetAttributeValue(matches[0], kAXPositionAttribute as CFString, value)
        }
        if result != .success { fail("Window geometry failed: \(result.rawValue)") }
    case "position":
        guard arguments.count == 5, let x = Double(arguments[3]), let y = Double(arguments[4]) else { fail("position requires screen x and y") }
        var point = CGPoint(x: x, y: y)
        guard let value = AXValueCreate(.cgPoint, &point) else { fail("Cannot create position value") }
        let result = AXUIElementSetAttributeValue(firstWindow(), kAXPositionAttribute as CFString, value)
        if result != .success { fail("Position failed: \(result.rawValue)") }
    case "active":
        print(NSWorkspace.shared.frontmostApplication?.processIdentifier == pid ? "true" : "false")
    case "activate":
        try activateTarget()
    default:
        fail("Unknown command")
    }
} catch {
    fail(error.localizedDescription)
}
