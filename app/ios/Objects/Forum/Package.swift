// swift-tools-version:5.10
import PackageDescription

// Forum — a multi-user thread, like a Slack channel. Holds a keyed set of
// topic_areas (sub-threads/channels) and a keyed set of members (each a Group).
// Each post is a Matter, authored & sealed via Identity (B) and synced over the
// Network (E) as an MLS group; Permissions (C) gate which member sees which
// topic_area. Stores multimedia as content-addressed blobs (FieldValue.blob)
// referenced by posts. Binds to ../../core/PacificCore.
let package = Package(
    name: "Forum",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "Forum", targets: ["Forum"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "Forum", dependencies: ["PacificCore"]),
        .testTarget(name: "ForumTests", dependencies: ["Forum"])
    ]
)
