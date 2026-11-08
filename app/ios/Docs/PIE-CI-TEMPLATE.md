# PIE⊗CI — the per-module strategy convention

Every object module under `objects/<Name>/` ships a `PIE-CI.md` that states its
strategy along five axes:

> **PIE** = Purpose ⊗ Interface ⊗ Encapsulation  
> **CI**  = Connection ⊗ Implementation

`⊗` (tensor) is deliberate: the five are not a checklist but a product — each axis
must be coherent *with respect to* the others. A module's Interface only makes
sense given its Purpose; its Connection only given what it Encapsulates; and so on.

## The five axes

1. **Purpose** — the user's job this object does, in one or two sentences, then the
   "why this and not a neighbouring object". What is true of this object that is
   true of nothing else in the seven.
2. **Interface** — the public surface. The Swift protocol(s) and value types other
   modules and the three UI rails touch; what the **left rail** (selection), the
   **centre** (contextual detail) and the **right rail** (interaction) each see and
   do; and how sibling objects reference it (by `MatterID`, by shape, by event).
3. **Encapsulation** — what the module hides and the invariants it guarantees:
   internal state, what may never leak across the boundary, the rules that always
   hold (e.g. "a Poll's output is always derivable from its responses; the output
   is never stored as an independent truth").
4. **Connection** — which Pacific blocks (A Search, B Identity, C Permissions,
   D Dictionary, E Network/MLS, F Records) it binds, and exactly how; plus its
   edges to other objects (which objects it contains, references, subscribes to,
   or is contained by).
5. **Implementation** — how it is actually realised: the Matter projection (which
   fields, which `ObjectKind`), storage, embeddings/vector use, MLS group shape,
   any agent hooks (CLA for Project, the research loop for Topic), and the
   honest-by-default rule (unknown stays `nil`, never fabricated).

## Document shape

```markdown
# <Object> — PIE⊗CI

*One-line role of the object.*

## Purpose
## Interface
## Encapsulation
## Connection
## Implementation
```

Keep it candid and concrete (British spelling, matching the repo). Where a design
question is genuinely open, say so rather than inventing a false certainty.

## Swift stub convention

Alongside the doc, each module is a local Swift package:

```
objects/<Name>/
├── Package.swift                       # swift-tools 5.10; iOS 17 / macOS 14;
│                                       # depends on ../../core/PacificCore
├── Sources/<Name>/<Name>.swift         # the public interface stub
└── PIE-CI.md
```

`<Name>.swift` should define, at minimum:

- the object value type conforming to `NovaObject` (so it projects from a `Matter`);
- a module descriptor conforming to `NovaObjectModule` (`kind`, `blurb`);
- the object's own core value types and a protocol describing its interface surface
  (the operations the right rail / sibling objects invoke).

Stubs compile but need not be complete — bodies may be `fatalError("unimplemented")`
or `// TODO`. The point is a navigable, type-checked skeleton the docs describe.

`Package.swift` template:

```swift
// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "<Name>",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "<Name>", targets: ["<Name>"])],
    dependencies: [.package(path: "../../core/PacificCore")],
    targets: [
        .target(name: "<Name>", dependencies: ["PacificCore"]),
        .testTarget(name: "<Name>Tests", dependencies: ["<Name>"])
    ]
)
```
