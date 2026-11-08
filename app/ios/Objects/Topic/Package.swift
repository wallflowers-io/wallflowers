// swift-tools-version:5.10
import PackageDescription

// Topic — an agent-managed object for deep research into one particular topic.
// A Topic is kept alive by a ResearchAgent that runs on a PERIODIC schedule and
// over ACTIVE connections (connectors / Systems and the Network E). It grounds
// its research in the user's OWN corpus by reading the on-device vector index
// (Search A: SearchEngine / VectorIndex) and blends in external sources, curating
// the result into a living projection — a TopicDigest, a research dossier of
// Findings. Every Finding is honest-by-default: it CITES its source or is left
// blank — a finding is NEVER fabricated. Projects subscribe to a Topic, so a
// Topic emits a change event (over the Network E or a local event channel) that
// a subscribing Project's Core Loop Agent consumes. Binds to ../../core/PacificCore.
let package = Package(
    name: "Topic",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "Topic", targets: ["Topic"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "Topic", dependencies: ["PacificCore"]),
        .testTarget(name: "TopicTests", dependencies: ["Topic"])
    ]
)
