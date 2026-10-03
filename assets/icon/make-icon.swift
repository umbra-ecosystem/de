// Placeholder app icon for Deus: a dark rounded tile with a "D".
// Regenerate with: swift make-icon.swift && ./finish.sh (see below).
// Replace freely with the real product mark; cargo-bundle reads Deus.icns.
import AppKit

let size = 1024
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()

let rect = NSRect(x: 0, y: 0, width: size, height: size)
let tile = NSBezierPath(roundedRect: rect, xRadius: CGFloat(size) * 0.225, yRadius: CGFloat(size) * 0.225)
NSColor(srgbRed: 0.086, green: 0.090, blue: 0.106, alpha: 1.0).setFill()
tile.fill()

let glyph = "D" as NSString
let font = NSFont.systemFont(ofSize: CGFloat(size) * 0.60, weight: .bold)
let attrs: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: NSColor(srgbRed: 0.93, green: 0.93, blue: 0.94, alpha: 1.0),
]
let ts = glyph.size(withAttributes: attrs)
glyph.draw(
    at: NSPoint(x: (CGFloat(size) - ts.width) / 2, y: (CGFloat(size) - ts.height) / 2 - CGFloat(size) * 0.02),
    withAttributes: attrs
)

image.unlockFocus()

let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: "Deus-1024.png"))
print("wrote Deus-1024.png")
