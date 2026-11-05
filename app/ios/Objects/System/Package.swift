// swift-tools-version:5.10
import PackageDescription

// System — an interactable external system, API or software package
// (Onshape, Gmail, a watched file …). A System is a NORMAL Pacific
// object: stored in SQLite, its membership an MLS group, edits synced over MLS.
// It is COMPOSED OF Fields (the Field object): each declared field deep-links
// into the external source. Its user-authored definition is stored WITH the
// object (not as a Dictionary schema); it may carry a MatchingTag for optional
// cross-space interop. A System exposes operations the right rail and a
// Project's CLA invoke (read / write / call), and it authenticates using
// credentials held at the Group level — it requests a scoped, revocable
// credential HANDLE and never holds the raw secret. Binds to ../../core/PacificCore.
//
// NOTE: the SwiftPM module is named `SystemObject`, not `System`, because a
// module named `System` collides with the platform `System` clang module and
// creates a Foundation import cycle. The directory stays objects/System and the
// public object type stays `System`; only the module name differs.
let package = Package(
    name: "SystemObject",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "SystemObject", targets: ["SystemObject"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "SystemObject", dependencies: ["PacificCore"], path: "Sources/System"),
        .testTarget(name: "SystemObjectTests", dependencies: ["SystemObject"], path: "Tests/SystemTests")
    ]
)
