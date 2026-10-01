// Independent QR check: decode a PNG with CIDetector — Apple's own detector, the
// same one an iPhone camera uses, and the same framework `ContactExport.swift`
// generates with. `qr.js` decoding its own matrix proves internal consistency;
// only a second implementation proves the code is really a QR code.
//
//   swift qr-verify.swift <png> [expected]
import Foundation
import CoreImage

let args = CommandLine.arguments
guard args.count >= 2, let img = CIImage(contentsOf: URL(fileURLWithPath: args[1])) else {
    print("usage: swift qr-verify.swift <png> [expected]"); exit(2)
}
let d = CIDetector(ofType: CIDetectorTypeQRCode, context: CIContext(),
                   options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])!
let found = d.features(in: img).compactMap { ($0 as? CIQRCodeFeature)?.messageString }
guard let got = found.first else { print("NO QR FOUND"); exit(1) }
print("decoded: \(got)")
if args.count >= 3 {
    let want = args[2]
    print(got == want ? "MATCH" : "MISMATCH\n  want: \(want)\n  got:  \(got)")
    exit(got == want ? 0 : 1)
}
