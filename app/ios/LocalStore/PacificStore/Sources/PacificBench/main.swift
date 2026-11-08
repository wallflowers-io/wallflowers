//  main.swift — pacific-bench
//  The benchmark runner: standard ER rows in, the REAL engines' verdicts out.
//
//  usage: pacific-bench <rows.jsonl> <pred.json>
//
//  Pipeline per invocation (deterministic end to end):
//    1 · read rows {id, source, label, text, vector}
//    2 · SpaceCalibration.fit over the corpus (components: 0 — mean-only)
//    3 · GraphResolution.resolve with the bench "Record" rules — two-source
//        provenance (.reference) so the disjointness veto is structurally live
//    4 · CoarseningPyramid duplicateSets (semantic kernel, calibrated)
//    5 · emit {identified, proposed, pyramid} pair lists for score.py

import Foundation
import PacificStore

struct BenchRow: Decodable {
    let id: String
    let source: String
    let label: String
    let text: String
    let meta: [String: String]?
    let vector: [Float]
}

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write(Data("usage: pacific-bench <rows.jsonl> <pred.json>\n".utf8))
    exit(2)
}

let decoder = JSONDecoder()
var rows: [BenchRow] = []
for line in try String(contentsOfFile: args[1], encoding: .utf8).split(separator: "\n") {
    rows.append(try decoder.decode(BenchRow.self, from: Data(line.utf8)))
}
FileHandle.standardError.write(Data("rows: \(rows.count)\n".utf8))

let calibration = SpaceCalibration.fit(rows.map(\.vector))

// The bench rule set: untuned, generic — and NO standalone-sem rule. The first
// run proved the logistic z-scale saturates: calibrated 0.95 is only ≈3σ, and
// across 12M pairs thousands of merely-similar records clear 3σ. Semantic
// evidence corroborates the lexical arm; it never matches alone here.
var params = ResolutionParams()
params.rules["Record"] = [
    MatchRule(name: "lex", requires: [.lexExact]),
    MatchRule(name: "lex+sem", requires: [.lexOverlapAtLeast(0.5), .semAtLeast(0.85)]),
]

let nodes = rows.map {
    GraphNodeRow(id: $0.id, kind: "Record", label: $0.label,
                 provenance: .reference(source: $0.source, key: $0.id),
                 time: BiTime(validAt: 0, createdAt: 0),
                 vector: $0.vector)
}

let t0 = Date()
let view = GraphResolution.resolve(nodes: nodes, edges: [], t: 1, tau: 1,
                                   params: params, calibration: calibration)
FileHandle.standardError.write(Data(String(format: "calculus: %.1fs · identified %d · proposed %d\n",
                                           -t0.timeIntervalSinceNow,
                                           view.identifications.count,
                                           view.proposals.count).utf8))

let t1 = Date()
let pyramid = CoarseningPyramid.build(ids: rows.map(\.id), vectors: rows.map(\.vector),
                                      calibration: calibration)
let sets = pyramid.duplicateSets
FileHandle.standardError.write(Data(String(format: "pyramid: %.1fs · %d duplicate sets\n",
                                           -t1.timeIntervalSinceNow, sets.count).utf8))

func pairs(_ idents: [Identification]) -> [[String]] {
    idents.map { [$0.a, $0.b].sorted() }
}
var pyramidPairs: [[String]] = []
for set in sets {
    for i in 0..<set.count {
        for j in (i + 1)..<set.count { pyramidPairs.append([set[i], set[j]].sorted()) }
    }
}

// The HYDRATED pyramid: scenarios built from the exported structural metadata
// (KernelHydration shapes the ladder to actual coverage). One family per level
// so the table shows where context pays.
let hydratedRows = rows.map { r in
    HydratedRow(id: r.id, vector: r.vector,
                meta: (r.meta ?? [:]).mapValues { Set([$0]) })
}
let ladder = KernelHydration.ladder(rows: hydratedRows,
                                    sigmas: [0.05, 0.15, 0.3],
                                    coverage: 0.5)
FileHandle.standardError.write(Data("ladder: \(ladder.map(\.name).joined(separator: " → "))\n".utf8))
let t2 = Date()
let hydrated = CoarseningPyramid.build(hydrated: hydratedRows, ladder: ladder,
                                       calibration: calibration)
FileHandle.standardError.write(Data(String(format: "hydrated: %.1fs · %d levels\n",
                                           -t2.timeIntervalSinceNow, hydrated.levels.count).utf8))
var hydratedFamilies: [String: [[String]]] = [:]
for (l, level) in hydrated.levels.enumerated() where level.grains.count > 1 {
    var levelPairs: [[String]] = []
    for g in level.grains where g.members.count > 1 {
        for i in 0..<g.members.count {
            for j in (i + 1)..<g.members.count {
                levelPairs.append([g.members[i], g.members[j]].sorted())
            }
        }
    }
    hydratedFamilies["hydrated-L\(l)-\(ladder[l].name)"] = levelPairs
}

var blob: [String: [[String]]] = [
    "identified": pairs(view.identifications),
    "proposed": pairs(view.proposals),
    "pyramid": pyramidPairs,
]
blob.merge(hydratedFamilies) { a, _ in a }
let out = try JSONSerialization.data(withJSONObject: blob, options: [.sortedKeys])
try out.write(to: URL(fileURLWithPath: args[2]))
FileHandle.standardError.write(Data("wrote \(args[2])\n".utf8))
