// swift-tools-version:5.10
import PackageDescription

// Group — a user, a group of users, a person or a group of people, ON or OFF
// the platform. The unit of MEMBERSHIP everywhere in Pacific (Project / Forum /
// Poll / Field members are Groups), maps to a Pacific Identity SpaceID, and
// holds the credential vault that System and Field objects draw scoped,
// revocable access from. Binds to ../../core/PacificCore.
let package = Package(
    name: "Group",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "Group", targets: ["Group"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "Group", dependencies: ["PacificCore"]),
        .testTarget(name: "GroupTests", dependencies: ["Group"])
    ]
)
