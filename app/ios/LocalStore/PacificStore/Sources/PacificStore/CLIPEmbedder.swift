//  CLIPEmbedder.swift
//  The on-device image/text encoder that makes notes searchable by picture.
//
//  WHY A SECOND EMBEDDER. The union store runs Apple's NLEmbedding, which is text-only: it
//  has no image counterpart, so an image vector could never be compared with it. CLIP has two
//  towers trained into ONE shared space, so a text query and an image land in the same
//  geometry and "a yellow flower in the dunes" retrieves the photo.
//
//  WHY NOT JUST MOVE THE UNION TO CLIP. CLIP's text tower has a 77-token context. Note bodies
//  are far longer than that, so re-embedding note TEXT with CLIP would truncate it and make
//  ordinary text search worse. The right split is: note text stays on the text model, images
//  get their own CLIP-dimension index, and a query is embedded by BOTH and the results fused.
//  Short queries are exactly what CLIP's text tower is good at, so nothing is lost.
//
//  MODEL. Apple's MobileCLIP-S0, both towers as Core ML. Image tower takes a 256x256 buffer;
//  text tower takes [1, 77] int32 CLIP BPE tokens; both emit 512-d. Weights are not committed:
//  see CLIPModels/README.md.
//
//  Honesty contract (house rule): this vends real vectors or THROWS. A missing model, a failed
//  decode, or an unexpected output shape is a real error — never a zero vector, which would
//  silently make every image equally (ir)relevant and look like "no matches".

import Foundation
import CoreML
import CoreGraphics
import ImageIO
import Accelerate

public enum CLIPEmbedderError: Error, LocalizedError {
    case notProvisioned(String)
    case decode(String)
    case badOutput(String)

    public var errorDescription: String? {
        switch self {
        case .notProvisioned(let what):
            return "CLIP is not provisioned (\(what)). See PacificStore/Sources/PacificStore/CLIPModels/README.md."
        case .decode(let m): return "CLIP could not read the image: \(m)"
        case .badOutput(let m): return "CLIP returned an unusable result: \(m)"
        }
    }
}

/// Embeds images and short queries into one shared 512-d space.
///
/// Thread-safe: Core ML `MLModel` prediction is safe to call concurrently, and the tokenizer
/// is immutable after construction.
public final class CLIPEmbedder: @unchecked Sendable {

    /// The shared space's width. Images are indexed at this dimension, NOT the union store's.
    public static let dimension = 512

    /// The side length the image tower expects.
    private static let inputSide = 256

    /// Subdirectory the provisioned models are copied into. NOT named "Resources": codesign
    /// treats a resource bundle containing a `Resources/` directory as a structured bundle and
    /// refuses to sign it ("bundle format unrecognized").
    private static let resourceDir = "CLIPModels"

    private let imageModel: MLModel
    private let textModel: MLModel
    private let tokenizer: CLIPTokenizer

    /// Loads both towers from the package bundle. Throws if provisioning has not been run —
    /// which is the honest outcome, since there is no meaningful vector to return without them.
    public convenience init() throws { try self.init(bundle: .module) }

    public init(bundle: Bundle) throws {
        // Already-compiled models (.mlmodelc). Compiling at provision time rather than at
        // load means no first-image stall on device, and keeps codesign happy — an
        // .mlpackage inside a resource bundle is a bundle-in-a-bundle that it rejects.
        func modelURL(_ name: String) throws -> URL {
            guard let url = bundle.url(forResource: name, withExtension: "mlmodelc", subdirectory: Self.resourceDir) else {
                throw CLIPEmbedderError.notProvisioned("\(name).mlmodelc is not in the bundle")
            }
            return url
        }
        guard let vocabURL = bundle.url(forResource: "clip_bpe_vocab", withExtension: "txt", subdirectory: Self.resourceDir) else {
            throw CLIPEmbedderError.notProvisioned("clip_bpe_vocab.txt is not in the bundle")
        }

        let config = MLModelConfiguration()
        // Let Core ML use the Neural Engine when the device has one; it falls back on its own.
        config.computeUnits = .all
        imageModel = try MLModel(contentsOf: try modelURL("mobileclip_s0_image"), configuration: config)
        textModel = try MLModel(contentsOf: try modelURL("mobileclip_s0_text"), configuration: config)
        tokenizer = CLIPTokenizer(mergesText: try String(contentsOf: vocabURL, encoding: .utf8))
    }

    // MARK: - image

    /// Embed a decoded image. The image is CENTRE-CROPPED to square before scaling, matching
    /// CLIP's own preprocessing — squashing to the square instead distorts geometry and costs
    /// real accuracy.
    public func embed(image: CGImage) throws -> [Float] {
        let buffer = try Self.pixelBuffer(from: image)
        let input = try MLDictionaryFeatureProvider(dictionary: ["image": MLFeatureValue(pixelBuffer: buffer)])
        let out = try imageModel.prediction(from: input)
        return try Self.unitVector(out, "image")
    }

    /// Embed image file bytes (what a Photos asset request hands back).
    public func embed(imageData: Data) throws -> [Float] {
        guard let src = CGImageSourceCreateWithData(imageData as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
            throw CLIPEmbedderError.decode("could not decode \(imageData.count) bytes")
        }
        return try embed(image: image)
    }

    // MARK: - text

    /// Embed a SHORT query into the image space. Anything past CLIP's 77-token window is
    /// truncated by the tokenizer — correct for queries, which is all this is for. Note bodies
    /// belong on the union store's text embedder, not here.
    public func embed(query: String) throws -> [Float] {
        let ids = tokenizer.encode(query)
        let array = try MLMultiArray(shape: [1, NSNumber(value: CLIPTokenizer.contextLength)], dataType: .int32)
        for (i, id) in ids.enumerated() { array[i] = NSNumber(value: id) }
        let input = try MLDictionaryFeatureProvider(dictionary: ["text": MLFeatureValue(multiArray: array)])
        let out = try textModel.prediction(from: input)
        return try Self.unitVector(out, "text")
    }

    // MARK: - plumbing

    /// L2-normalise, so a dot product IS cosine similarity — which is what the index scans.
    private static func unitVector(_ out: MLFeatureProvider, _ which: String) throws -> [Float] {
        guard let name = out.featureNames.first(where: { out.featureValue(for: $0)?.multiArrayValue != nil }),
              let array = out.featureValue(for: name)?.multiArrayValue else {
            throw CLIPEmbedderError.badOutput("\(which) tower produced no array")
        }
        guard array.count == dimension else {
            throw CLIPEmbedderError.badOutput("\(which) tower produced \(array.count) values, expected \(dimension)")
        }
        var v = [Float](repeating: 0, count: dimension)
        for i in 0..<dimension { v[i] = array[i].floatValue }
        var norm: Float = 0
        vDSP_svesq(v, 1, &norm, vDSP_Length(dimension))
        norm = sqrt(norm)
        guard norm > 0 else { throw CLIPEmbedderError.badOutput("\(which) tower produced a zero vector") }
        var scale = 1 / norm
        vDSP_vsmul(v, 1, &scale, &v, 1, vDSP_Length(dimension))
        return v
    }

    private static func pixelBuffer(from image: CGImage) throws -> CVPixelBuffer {
        var pb: CVPixelBuffer?
        let status = CVPixelBufferCreate(
            nil, inputSide, inputSide, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferCGImageCompatibilityKey: true,
             kCVPixelBufferCGBitmapContextCompatibilityKey: true] as CFDictionary, &pb)
        guard status == kCVReturnSuccess, let buffer = pb else {
            throw CLIPEmbedderError.decode("could not allocate a \(inputSide)x\(inputSide) buffer")
        }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let ctx = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: inputSide, height: inputSide, bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        ) else {
            throw CLIPEmbedderError.decode("could not create a drawing context")
        }
        let w = CGFloat(image.width), h = CGFloat(image.height), side = min(w, h)
        guard let square = image.cropping(to: CGRect(x: (w - side) / 2, y: (h - side) / 2,
                                                     width: side, height: side)) else {
            throw CLIPEmbedderError.decode("could not centre-crop a \(image.width)x\(image.height) image")
        }
        ctx.draw(square, in: CGRect(x: 0, y: 0, width: inputSide, height: inputSide))
        return buffer
    }
}
