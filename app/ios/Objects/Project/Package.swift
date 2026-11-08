// swift-tools-version:5.10
import PackageDescription

// Project — a project-management hub. It owns and manages task Matters,
// carries optional additional members (each a Group, in a ROLE), and runs a
// Core Loop Agent (CLA): an agent hook that fires whenever one of the Project's
// subscriptions registers a change. A Project subscribes to Topics (deep-research
// objects) and to connectors (Systems). On a change event the CLA may make
// recommendations and — within an explicit, Permission- and Identity-gated
// autonomy boundary — take autonomous action. Binds to ../../core/PacificCore.
let package = Package(
    name: "Project",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "Project", targets: ["Project"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "Project", dependencies: ["PacificCore"]),
        .testTarget(name: "ProjectTests", dependencies: ["Project"])
    ]
)
