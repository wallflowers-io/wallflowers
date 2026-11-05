// swift-tools-version:5.10
import PackageDescription

// VectorService — the embedding seam for LocalStore/Vector. It turns text into
// vectors behind ONE public facade. The only backend is Apple NLEmbedding,
// on device: no network, and no content ever leaves the device. (The remote
// Voyage backend was deleted with the Anthropic-route cut, 26 Sep 2026.)
//
// Callers depend only on `VectorService`; the backend is chosen once, at
// construction, and is invisible everywhere downstream. `embed` is fallible and
// THROWS on any failure — it never returns a zero/placeholder vector.
//
// No external dependencies (Foundation + NaturalLanguage only), so it builds and
// `swift test`s on a host Mac independently of the rest of the app.
let package = Package(
    name: "VectorService",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "VectorService", targets: ["VectorService"])],
    targets: [
        .target(name: "VectorService", path: "Sources/VectorService"),
        .testTarget(name: "VectorServiceTests", dependencies: ["VectorService"], path: "Tests/VectorServiceTests")
    ]
)
