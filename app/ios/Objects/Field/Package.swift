// swift-tools-version:5.10
import PackageDescription

// Field — a set of key:index pairs, and the BASE UNIT of Pacific's parametric
// modelling. A Field is a small, addressable cell: each `key` names a slot and
// each `.index` addresses into a structured source. A Field's value may be a
// literal, a live value DEEP-LINKED into a networked data source (pulled/pushed
// through a System using a scoped, revocable credential GRANT drawn from the
// Group level — never the raw secret), or a PARAMETRIC expression: a function of
// other Fields. Systems are composed of Fields; a parametric model is a graph of
// Fields with a dependency edge for every reference, recomputed on change. Each
// Field carries its own members with per-member visibility/edit Roles
// (Permissions C, at field granularity). Honest by default: an unresolved
// networked value stays nil, never fabricated. Binds to ../../core/PacificCore.
let package = Package(
    name: "Field",
    platforms: [.iOS("18.0"), .macOS(.v14)],
    products: [.library(name: "Field", targets: ["Field"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "Field", dependencies: ["PacificCore"]),
        .testTarget(name: "FieldTests", dependencies: ["Field"])
    ]
)
