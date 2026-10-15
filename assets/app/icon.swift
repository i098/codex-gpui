// Writes the original app icon ("Icon" in ChatGPT.app's Assets.car) as an .iconset, read
// with CoreUI's CUICatalog (private; assetutil only lists renditions).
// Usage (Mac): swiftc -O icon.swift -o icon && ./icon <Assets.car> <AppIcon.iconset>
//              && iconutil -c icns AppIcon.iconset -o AppIcon.icns
import AppKit

let args = CommandLine.arguments
dlopen("/System/Library/PrivateFrameworks/CoreUI.framework/CoreUI", RTLD_NOW)
let initSel = NSSelectorFromString("initWithURL:error:")
typealias InitURL = @convention(c) (AnyObject, Selector, NSURL, UnsafeMutableRawPointer?) -> AnyObject?
let blank = (NSClassFromString("CUICatalog") as! NSObject.Type).init()
let catalog = unsafeBitCast(blank.method(for: initSel), to: InitURL.self)(
    blank, initSel, URL(fileURLWithPath: args[1]) as NSURL, nil)!
let renditions = catalog.perform(NSSelectorFromString("imagesWithName:"), with: "Icon")
    .takeUnretainedValue() as! [NSObject]
// Pixel width -> image; the catalog has 32...1024 px renditions (16 px is scaled from 32).
var byWidth: [Int: CGImage] = [:]
for r in renditions where r.responds(to: NSSelectorFromString("image")) {
    let image = r.perform(NSSelectorFromString("image")).takeUnretainedValue() as! CGImage
    byWidth[image.width] = image
}
func scaled(_ image: CGImage, _ px: Int) -> CGImage {
    let ctx = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: px, height: px))
    return ctx.makeImage()!
}
try FileManager.default.createDirectory(atPath: args[2], withIntermediateDirectories: true)
for pt in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let px = pt * scale
        let image = byWidth[px] ?? scaled(byWidth[px * 2]!, px)
        let name = "icon_\(pt)x\(pt)\(scale == 2 ? "@2x" : "").png"
        try NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])!
            .write(to: URL(fileURLWithPath: args[2]).appendingPathComponent(name))
    }
}
