// swift-tools-version: 5.9
import PackageDescription

// ArcResolverKit — the Swift face of the on-device arc-resolver (a bi-temporal graph over
// lodedb-graph: deterministic object projection, LIFE reads and on-device embeddings, with no
// language model). The native crate ships as a prebuilt static XCFramework
// (arm64 slices for macOS, iOS device, iOS simulator + the C header as a Clang module),
// built by scripts/package_xcframework.sh with --no-default-features so the platform injects
// its embedder via the C callback. Run that script once before `swift build`.
let package = Package(
    name: "ArcResolverKit",
    platforms: [
        .macOS(.v13),
        .iOS("18.0")
    ],
    products: [
        .library(name: "ArcResolverKit", targets: ["ArcResolverKit"])
    ],
    targets: [
        .binaryTarget(name: "ArcResolverFFI", path: "Artifacts/ArcResolverFFI.xcframework"),
        .target(
            name: "ArcResolverKit",
            dependencies: ["ArcResolverFFI"],
            linkerSettings: [
                // lodedb-graph (faer/gemm) calls BLAS via Accelerate on Apple.
                .linkedFramework("Accelerate")
            ]
        )
    ]
)
