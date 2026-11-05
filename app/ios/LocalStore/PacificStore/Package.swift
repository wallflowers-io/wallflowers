// swift-tools-version: 5.9

import PackageDescription

// PacificStore — the LOCAL-STORE seam layer for Pacific.
//
// This package holds the frozen contracts that every Swift workstream builds against: the union-store
// config (§2), the row metadata schema (§3), the `LocalStore` read/write facade
// (§4), and the `LifeSource` projection the LIFE screen reads. It intentionally
// ships ONLY the contracts plus compile-only in-memory stubs — the real
// LodeDB-backed store (WS-E) and the ONNX/BGE embedder (WS-D) land as additional
// files under Sources/PacificStore/ with no manifest churn (SwiftPM auto-includes
// Sources/). Consumers therefore compile against a stable base today and swap the
// stub for the real store when it arrives.
//
// The dependency on the sibling LodeDBCore package (the on-device vector/graph
// engine) is declared here so those later files can `import LodeDBCore` without
// touching this manifest. Only `UnionConfig` uses it today — to bridge the §2
// contract onto LodeDBCore's own `LodeStoreOptions` / `EmbeddingPreset` types so
// the two cannot drift.
//
// LodeDBCore lives OUTSIDE this repo, as a sibling checkout:
//   this manifest: <workspace>/pacific/LocalStore/PacificStore/
//   LodeDBCore:    <workspace>/lode-lab/lodedb/swift/LodeDBCore/
// so the relative path climbs four levels (PacificStore -> LocalStore -> app ->
// pacific -> <workspace>) before descending into lode-lab. (LodeDBCore's
// binary target is arm64-only; it builds green on arm64 Macs.)
let package = Package(
    name: "PacificStore",
    platforms: [
        .iOS("18.0"),
        .macOS(.v13)
    ],
    products: [
        .library(name: "PacificStore", targets: ["PacificStore"]),
        // The benchmark runner (lode-lab/bench/): drives the REAL engines over
        // standard ER datasets — never a re-implementation that could drift.
        .executable(name: "pacific-bench", targets: ["PacificBench"])
    ],
    dependencies: [
        .package(name: "LodeDBCore", path: "../../target/lodedb/swift/LodeDBCore")
    ],
    targets: [
        .target(
            name: "PacificStore",
            dependencies: [
                .product(name: "LodeDBCore", package: "LodeDBCore")
            ],
            // The CLIP towers + BPE vocabulary that make images searchable by text. Copied
            // verbatim (an .mlpackage is a directory Core ML compiles at load; `process`
            // would flatten it). Not committed (CLIPModels/README.md), so a fresh clone builds
            // and CLIPEmbedder throws until the models are in place.
            resources: [.copy("CLIPModels")],
            linkerSettings: [
                // Transitively links the static native core, which calls Accelerate
                // (BLAS) — mirror LodeDBCore's own manifest so the frameworks resolve.
                .linkedFramework("Accelerate")
            ]
        ),
        .executableTarget(name: "PacificBench", dependencies: ["PacificStore"]),
        .testTarget(name: "PacificStoreTests", dependencies: ["PacificStore"])
    ]
)
