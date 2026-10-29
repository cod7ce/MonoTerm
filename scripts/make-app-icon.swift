#!/usr/bin/env swift
// Build the macOS icon from the supplied artwork using native image tooling.
import AppKit
import CoreImage
import ImageIO
import UniformTypeIdentifiers

func fail(_ message: String) -> Never {
    fputs("\(message)\n", stderr)
    exit(1)
}

let arguments = CommandLine.arguments
if arguments.count != 3 {
    fail("Usage: make-app-icon.swift <source.png> <output-directory>")
}
let sourceURL = URL(fileURLWithPath: arguments[1])
let outputURL = URL(fileURLWithPath: arguments[2], isDirectory: true)
let fileManager = FileManager.default
try fileManager.createDirectory(at: outputURL, withIntermediateDirectories: true)
guard let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
      let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
    fail("Cannot decode source artwork")
}
guard (image.width, image.height) == (1296, 1158) || (image.width, image.height) == (1264, 1264) else {
    fail("Unsupported artwork dimensions \(image.width) x \(image.height)")
}

func context(_ width: Int, _ height: Int) -> CGContext {
    guard let result = CGContext(data: nil, width: width, height: height,
                                bitsPerComponent: 8, bytesPerRow: width * 4,
                                space: colorSpace,
                                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        fail("Cannot allocate image context")
    }
    result.interpolationQuality = .high
    return result
}

func png(_ image: CGImage, _ url: URL) {
    guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
        fail("Cannot write \(url.path)")
    }
    CGImageDestinationAddImage(destination, image, nil)
    if !CGImageDestinationFinalize(destination) { fail("Cannot finish \(url.path)") }
}

// Trace just inside the object's edge to exclude the photographic background.
// Coordinates follow the original image's top-left origin.
let contour = CGMutablePath()
let sourceBounds: CGRect
if image.width == 1296 {
    sourceBounds = CGRect(x: 88, y: 8, width: 1120, height: 1128)
    contour.move(to: CGPoint(x: 335, y: 12))
    contour.addCurve(to: CGPoint(x: 956, y: 12), control1: CGPoint(x: 510, y: 8), control2: CGPoint(x: 800, y: 8))
    contour.addCurve(to: CGPoint(x: 1200, y: 269), control1: CGPoint(x: 1138, y: 13), control2: CGPoint(x: 1202, y: 88))
    contour.addCurve(to: CGPoint(x: 1195, y: 939), control1: CGPoint(x: 1207, y: 480), control2: CGPoint(x: 1207, y: 780))
    contour.addCurve(to: CGPoint(x: 1050, y: 1115), control1: CGPoint(x: 1188, y: 1026), control2: CGPoint(x: 1137, y: 1093))
    contour.addCurve(to: CGPoint(x: 954, y: 1127), control1: CGPoint(x: 1020, y: 1124), control2: CGPoint(x: 988, y: 1127))
    contour.addCurve(to: CGPoint(x: 320, y: 1126), control1: CGPoint(x: 767, y: 1132), control2: CGPoint(x: 493, y: 1132))
    contour.addCurve(to: CGPoint(x: 247, y: 1116), control1: CGPoint(x: 290, y: 1125), control2: CGPoint(x: 268, y: 1122))
    contour.addCurve(to: CGPoint(x: 99, y: 928), control1: CGPoint(x: 159, y: 1090), control2: CGPoint(x: 105, y: 1027))
    contour.addCurve(to: CGPoint(x: 95, y: 263), control1: CGPoint(x: 88, y: 751), control2: CGPoint(x: 89, y: 453))
    contour.addCurve(to: CGPoint(x: 335, y: 12), control1: CGPoint(x: 99, y: 95), control2: CGPoint(x: 163, y: 16))
    contour.closeSubpath()
} else {
    // The square artwork paints its rounded tile on black. Stay 3px inside the
    // measured tile edge (x 47...1216, y 34...1207) so the black surround stays out.
    sourceBounds = CGRect(x: 50, y: 37, width: 1164, height: 1168)
    contour.addRoundedRect(in: sourceBounds, cornerWidth: 282, cornerHeight: 282,
                           transform: .identity)
}

// A centered 824px optical body leaves room for the rounded silhouette and shadow
// on the 1024px macOS canvas. This is an artwork choice, not a format requirement.
let master = context(1024, 1024)
let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let scale = body.width / sourceBounds.height
let bodyWidth = sourceBounds.width * scale
let originX = (1024 - bodyWidth) / 2
master.saveGState()
master.translateBy(x: 0, y: 1024)
master.scaleBy(x: 1, y: -1)
master.translateBy(x: originX - sourceBounds.minX * scale, y: body.minY - sourceBounds.minY * scale)
master.scaleBy(x: scale, y: scale)
master.addPath(contour)
master.clip()
master.translateBy(x: 0, y: CGFloat(image.height))
master.scaleBy(x: 1, y: -1)
master.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
master.restoreGState()
guard let cutout = master.makeImage() else { fail("Cannot render icon cutout") }

let ciContext = CIContext(options: [.workingColorSpace: colorSpace, .outputColorSpace: colorSpace])
let silhouette = CIImage(cgImage: cutout)
let shadow = silhouette.applyingFilter("CIColorMatrix", parameters: [
    "inputRVector": CIVector(x: 0, y: 0, z: 0, w: 0),
    "inputGVector": CIVector(x: 0, y: 0, z: 0, w: 0),
    "inputBVector": CIVector(x: 0, y: 0, z: 0, w: 0),
    "inputAVector": CIVector(x: 0, y: 0, z: 0, w: 0.24),
]).applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: 8])
    .transformed(by: CGAffineTransform(translationX: 0, y: -6))
guard let icon = ciContext.createCGImage(silhouette.composited(over: shadow),
                                         from: CGRect(x: 0, y: 0, width: 1024, height: 1024),
                                         format: .RGBA8, colorSpace: colorSpace) else {
    fail("Cannot render icon shadow")
}
png(icon, outputURL.appendingPathComponent("AppIcon.png"))

let iconset = outputURL.appendingPathComponent("AppIcon.iconset", isDirectory: true)
try fileManager.createDirectory(at: iconset, withIntermediateDirectories: true)
var sizes: [Int: CGImage] = [:]
for points in [16, 32, 128, 256, 512] {
    for density in [1, 2] {
        let pixels = points * density
        let resized: CGImage
        if let cached = sizes[pixels] {
            resized = cached
        } else {
            let filter = CIFilter(name: "CILanczosScaleTransform")!
            filter.setValue(CIImage(cgImage: icon), forKey: kCIInputImageKey)
            filter.setValue(Double(pixels) / 1024, forKey: kCIInputScaleKey)
            filter.setValue(1.0, forKey: kCIInputAspectRatioKey)
            guard let output = filter.outputImage,
                  let result = ciContext.createCGImage(output,
                    from: CGRect(x: 0, y: 0, width: pixels, height: pixels),
                    format: .RGBA8, colorSpace: colorSpace) else { fail("Cannot resize icon") }
            resized = result
            sizes[pixels] = resized
        }
        let suffix = density == 2 ? "@2x" : ""
        png(resized, iconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}

let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", "-o", outputURL.appendingPathComponent("AppIcon.icns").path, iconset.path]
try iconutil.run()
iconutil.waitUntilExit()
if iconutil.terminationStatus != 0 { fail("iconutil failed") }

// A local review sheet shows the real alpha edges on both desktop appearances.
let preview = context(1400, 820)
for (index, background) in [NSColor(srgbRed: 0.94, green: 0.94, blue: 0.96, alpha: 1),
                             NSColor(srgbRed: 0.12, green: 0.12, blue: 0.14, alpha: 1)].enumerated() {
    let x = CGFloat(index * 700)
    preview.setFillColor(background.cgColor)
    preview.fill(CGRect(x: x, y: 0, width: 700, height: 820))
    preview.draw(icon, in: CGRect(x: x + 94, y: 240, width: 512, height: 512))
    var cursor = x + 80
    for pixels in [128, 64, 32, 16] {
        preview.draw(sizes[pixels]!, in: CGRect(x: cursor, y: 90, width: CGFloat(pixels), height: CGFloat(pixels)))
        cursor += CGFloat(pixels + 42)
    }
}
png(preview.makeImage()!, outputURL.appendingPathComponent("AppIcon-preview.png"))
print("Generated sRGB master, preview and 10 standard/Retina representations in \(outputURL.path)")
