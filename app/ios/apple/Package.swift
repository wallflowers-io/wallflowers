// swift-tools-version: 5.9
// PacificCore — the thin Swift package the iOS app imports. Three targets wire the
// Rust core in: the built xcframework (binary) ← the UniFFI-generated glue ←
// the idiomatic PacificCore wrapper. The app depends only on PacificCore.
//
// A local-path binaryTarget needs no code-signing/checksum (only remote ones do).
import PackageDescription

let package = Package(
  name: "PacificCore",
  platforms: [.iOS("18.0")],
  products: [
    .library(name: "PacificCore", targets: ["PacificCore"]),
  ],
  targets: [
    .binaryTarget(name: "PacificCoreRS", path: "Artifacts/PacificCoreRS.xcframework"),
    .target(name: "UniFFI", dependencies: ["PacificCoreRS"], path: "Sources/UniFFI"),
    .target(name: "PacificCore", dependencies: ["UniFFI"], path: "Sources/PacificCore"),
  ]
)
