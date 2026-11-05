//  Deliberation.swift
//  The vTaiwan-grade deliberation surface for a Project GroupObject, made navigable:
//  a slick 5-stage pipeline — Frame → Gather → Distil → Decide → Deliver — the
//  deliberation op-groups given a face.
//
//  Frame (the objectives spine) and Deliver (the plan + deliverables) read the REAL
//  project projections. Gather / Distil / Decide drive the deliberation ops
//  (poseStatement · vote · synthesizeFinding · propose/castAssent/closeRatification)
//  through an on-device `DeliberationStore` — durable per project — until the core
//  `OG_Deliberation` reducer lands and this store swaps for the Delta path unchanged.

import SwiftUI
import UIKit
import PacificCore

// MARK: - The five stages

enum DeliberationStage: Int, CaseIterable, Identifiable, Hashable {
    case frame, gather, distil, decide, deliver
    var id: Int { rawValue }
    var title: String { ["Frame", "Gather", "Distil", "Decide", "Deliver"][rawValue] }
    var icon: String {
        ["point.3.connected.trianglepath.dotted", "bubble.left.and.bubble.right.fill",
         "text.bubble.fill", "checklist", "calendar"][rawValue]
    }
    var tint: Color {
        [Color(hex: "6366f1"), Color(hex: "06b6d4"), Color(hex: "f59e0b"), Color(hex: "ec4899"), Color(hex: "10b981")][rawValue]
    }
    /// The GroupObject kind each stage's nodes ARE — the graph is typed by construction.
    /// (Display words: the hosted-room kind reads CHANNEL in the UI; the wire kind
    /// stays "forum".)
    var nodeKind: String {
        ["Group · Place", "Channel", "Channel", "Poll", "Event"][rawValue]
    }
    var caption: String {
        ["Wire the who & where — Group and Place nodes.",
         "A Channel — post statements, vote on them.",
         "A Channel — distil the discussion into findings.",
         "A Poll — ratify; dissent kept, not erased.",
         "Event nodes — the plan and its pieces."][rawValue]
    }
}

private enum DColor {
    static let agree = Color(hex: "10b981")
    static let disagree = Color(hex: "f43f5e")
    static let pass = Color(hex: "94a3b8")
}

// MARK: - Model + on-device store

enum DStance: String, Codable, CaseIterable { case agree, disagree, pass }
enum DAssent: String, Codable, CaseIterable { case assent, abstain, dissent }

struct DStatement: Identifiable, Codable, Hashable { let id: String; var text: String; var author: String; var ts: Double }
struct DFinding:   Identifiable, Codable, Hashable { let id: String; var text: String; var from: [String]; var ts: Double; var superseded: Bool = false }
struct DRatification: Identifiable, Codable, Hashable {
    let id: String; var findingId: String; var threshold: String; var quorum: Int
    var closed: Bool = false; var outcome: String = ""
}

@MainActor
final class DeliberationStore: ObservableObject {
    @Published private(set) var statements: [DStatement] = []
    @Published private(set) var votes: [String: [String: DStance]] = [:]     // statementId → author → stance
    @Published private(set) var findings: [DFinding] = []
    @Published private(set) var ratifications: [DRatification] = []
    @Published private(set) var assents: [String: [String: DAssent]] = [:]    // ratId → author → assent

    let objectId: String
    var me: String = "me"

    init(objectId: String) { self.objectId = objectId; load() }

    // MARK: Gather

    func poseStatement(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        statements.insert(.init(id: UUID().uuidString, text: t, author: me, ts: Date().timeIntervalSince1970), at: 0)
        save()
    }
    func vote(_ sid: String, _ stance: DStance) {
        var m = votes[sid] ?? [:]
        if m[me] == stance { m[me] = nil } else { m[me] = stance }   // tap-again clears (per-author LWW)
        votes[sid] = m
        save()
    }
    func myVote(_ sid: String) -> DStance? { votes[sid]?[me] }
    func tally(_ sid: String) -> (agree: Int, disagree: Int, pass: Int) {
        let v = votes[sid] ?? [:]
        return (v.values.filter { $0 == .agree }.count,
                v.values.filter { $0 == .disagree }.count,
                v.values.filter { $0 == .pass }.count)
    }
    /// Agree-ratio over cast (non-pass) votes — the cheap stand-in for the lodedb consensus lens.
    func agreeRatio(_ sid: String) -> Double {
        let t = tally(sid); let cast = t.agree + t.disagree
        return cast == 0 ? 0 : Double(t.agree) / Double(cast)
    }
    var totalVotes: Int { votes.values.reduce(0) { $0 + $1.count } }
    /// Statements ordered by agreement — the bridging read.
    var consensus: [DStatement] {
        statements.filter { !(votes[$0.id]?.isEmpty ?? true) }
            .sorted { agreeRatio($0.id) > agreeRatio($1.id) }
    }

    // MARK: Distil

    func synthesize(_ text: String, from: [String]) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        findings.insert(.init(id: UUID().uuidString, text: t, from: from, ts: Date().timeIntervalSince1970), at: 0)
        save()
    }
    func supersede(_ id: String) {
        if let i = findings.firstIndex(where: { $0.id == id }) { findings[i].superseded.toggle(); save() }
    }

    // MARK: Decide

    func propose(findingId: String, threshold: String, quorum: Int) {
        guard !ratifications.contains(where: { $0.findingId == findingId && !$0.closed }) else { return }
        ratifications.insert(.init(id: UUID().uuidString, findingId: findingId, threshold: threshold, quorum: quorum), at: 0)
        save()
    }
    func cast(_ rid: String, _ a: DAssent) {
        var m = assents[rid] ?? [:]
        if m[me] == a { m[me] = nil } else { m[me] = a }
        assents[rid] = m
        save()
    }
    func myAssent(_ rid: String) -> DAssent? { assents[rid]?[me] }
    func ratTally(_ rid: String) -> (assent: Int, abstain: Int, dissent: Int) {
        let v = assents[rid] ?? [:]
        return (v.values.filter { $0 == .assent }.count,
                v.values.filter { $0 == .abstain }.count,
                v.values.filter { $0 == .dissent }.count)
    }
    func dissenters(_ rid: String) -> [String] { (assents[rid] ?? [:]).filter { $0.value == .dissent }.map(\.key) }
    func close(_ rid: String) {
        guard let i = ratifications.firstIndex(where: { $0.id == rid }) else { return }
        let t = ratTally(rid)
        ratifications[i].closed = true
        ratifications[i].outcome = t.assent > t.dissent ? "carried" : "not carried"
        save()
    }
    func finding(_ id: String) -> DFinding? { findings.first { $0.id == id } }

    // MARK: Persistence (durable; swaps for the Delta log later)

    private struct Snapshot: Codable {
        var statements: [DStatement]; var votes: [String: [String: DStance]]
        var findings: [DFinding]; var ratifications: [DRatification]; var assents: [String: [String: DAssent]]
    }
    private var key: String { "pacific.deliberation." + objectId }
    private func save() {
        let s = Snapshot(statements: statements, votes: votes, findings: findings, ratifications: ratifications, assents: assents)
        if let d = try? JSONEncoder().encode(s) { UserDefaults.standard.set(d, forKey: key) }
    }
    private func load() {
        guard let d = UserDefaults.standard.data(forKey: key),
              let s = try? JSONDecoder().decode(Snapshot.self, from: d) else { return }
        statements = s.statements; votes = s.votes; findings = s.findings
        ratifications = s.ratifications; assents = s.assents
    }
}

// MARK: - The stage rail (slick pipeline navigator)

struct ProjectStageRail: View {
    @Binding var stage: DeliberationStage
    private let stages = DeliberationStage.allCases

    var body: some View {
        VStack(spacing: 7) {
            // A CYCLE, not a linear bar: a loop glyph leads, chevrons imply "go round again",
            // and there is no progress fill — every stage is reachable from any other.
            HStack(spacing: 1) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.system(size: 12, weight: .bold)).foregroundStyle(stage.tint)
                    .padding(.trailing, 3)
                    .animation(.easeInOut, value: stage)
                ForEach(Array(stages.enumerated()), id: \.element) { i, s in
                    node(s)
                    if i < stages.count - 1 {
                        Image(systemName: "chevron.compact.right")
                            .font(.system(size: 11, weight: .semibold)).foregroundStyle(.tertiary)
                    }
                }
            }
            Text("↻ " + stage.caption)
                .font(.system(size: 11.5, weight: .medium)).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity).multilineTextAlignment(.center)
                .id(stage).transition(.opacity)
        }
        .padding(.horizontal, 10).padding(.top, 8).padding(.bottom, 9)
        .background(Brand.groupBG)
        .overlay(Rectangle().fill(Theme.hairline).frame(height: 1), alignment: .bottom)
    }

    private func node(_ s: DeliberationStage) -> some View {
        let on = s == stage
        return Button {
            UISelectionFeedbackGenerator().selectionChanged()
            withAnimation(.spring(response: 0.42, dampingFraction: 0.8)) { stage = s }
        } label: {
            VStack(spacing: 4) {
                ZStack {
                    Circle().fill(on ? s.tint : Theme.fill)
                        .frame(width: 34, height: 34)
                        .overlay(Circle().strokeBorder(on ? .clear : s.tint.opacity(0.55), lineWidth: 1.5))
                        .shadow(color: on ? s.tint.opacity(0.55) : .clear, radius: 8, y: 2)
                    Image(systemName: s.icon)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(on ? .white : s.tint)
                }
                Text(s.title)
                    .font(.system(size: 9.5, weight: on ? .bold : .medium))
                    .foregroundStyle(on ? s.tint : .secondary)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Shared bits

private struct StageHeader: View {
    let stage: DeliberationStage; let subtitle: String
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: stage.icon).font(.system(size: 15, weight: .bold)).foregroundStyle(stage.tint)
            VStack(alignment: .leading, spacing: 1) {
                Text(stage.title.uppercased()).font(.system(size: 12, weight: .bold)).tracking(1.2).foregroundStyle(stage.tint)
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }
}

private struct TallyBar: View {
    let agree: Int, disagree: Int, pass: Int
    private var total: Int { max(1, agree + disagree + pass) }
    var body: some View {
        GeometryReader { g in
            HStack(spacing: 2) {
                seg(agree, DColor.agree, g.size.width)
                seg(disagree, DColor.disagree, g.size.width)
                seg(pass, DColor.pass, g.size.width)
            }
        }
        .frame(height: 6)
        .clipShape(Capsule())
        .background(Capsule().fill(Theme.fill))
    }
    private func seg(_ n: Int, _ c: Color, _ w: CGFloat) -> some View {
        Capsule().fill(c).frame(width: max(0, w * CGFloat(n) / CGFloat(total)))
    }
}

// MARK: - GATHER

struct GatherStage: View {
    @ObservedObject var store: DeliberationStore
    @State private var draft = ""
    @FocusState private var composing: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                StageHeader(stage: .gather, subtitle: "\(store.statements.count) statements · \(store.totalVotes) votes")

                composer

                if store.statements.isEmpty {
                    empty
                } else {
                    ForEach(store.statements) { s in card(s) }
                }
            }
            .padding()
        }
        .background(Brand.groupBG)
    }

    private var composer: some View {
        HStack(spacing: 8) {
            Image(systemName: "quote.opening").foregroundStyle(DeliberationStage.gather.tint)
            TextField("Pose a statement…", text: $draft, axis: .vertical)
                .focused($composing).lineLimit(1...3)
            Button {
                store.poseStatement(draft); draft = ""; composing = false
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
            } label: { Image(systemName: "arrow.up.circle.fill").font(.title2) }
                .disabled(draft.trimmingCharacters(in: .whitespaces).isEmpty)
                .foregroundStyle(DeliberationStage.gather.tint)
        }
        .padding(12)
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func card(_ s: DStatement) -> some View {
        let t = store.tally(s.id)
        return VStack(alignment: .leading, spacing: 10) {
            Text(s.text).font(.subheadline).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                voteButton(s, .agree, "hand.thumbsup.fill", DColor.agree)
                voteButton(s, .disagree, "hand.thumbsdown.fill", DColor.disagree)
                voteButton(s, .pass, "minus", DColor.pass)
                Spacer()
                Text("\(t.agree + t.disagree + t.pass)").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                Text("votes").font(.caption2).foregroundStyle(.tertiary)
            }
            TallyBar(agree: t.agree, disagree: t.disagree, pass: t.pass)
        }
        .padding(13)
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func voteButton(_ s: DStatement, _ stance: DStance, _ icon: String, _ c: Color) -> some View {
        let on = store.myVote(s.id) == stance
        return Button {
            store.vote(s.id, stance); UISelectionFeedbackGenerator().selectionChanged()
        } label: {
            Image(systemName: icon).font(.system(size: 13, weight: .semibold))
                .frame(width: 40, height: 30)
                .background(on ? c : Theme.fill, in: RoundedRectangle(cornerRadius: 9))
                .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(on ? .clear : c.opacity(0.4), lineWidth: 1))
                .foregroundStyle(on ? .white : c)
        }
        .buttonStyle(.plain)
    }

    private var empty: some View {
        VStack(spacing: 6) {
            Image(systemName: "bubble.left.and.bubble.right").font(.system(size: 26)).foregroundStyle(.tertiary)
            Text("Pose the first statement").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
            Text("Anyone can add one; anyone can vote agree · disagree · pass.").font(.caption).foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity).padding(.vertical, 34)
    }
}

// MARK: - DISTIL

struct DistilStage: View {
    @ObservedObject var store: DeliberationStore
    var onDecide: () -> Void = {}
    @State private var draft = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                StageHeader(stage: .distil, subtitle: "\(store.findings.count) findings")

                if !store.consensus.isEmpty {
                    VStack(alignment: .leading, spacing: 7) {
                        Label("Most agreed", systemImage: "chart.bar.fill").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                        ForEach(store.consensus.prefix(3)) { s in
                            HStack(spacing: 8) {
                                Text("\(Int(store.agreeRatio(s.id) * 100))%").font(.caption2.weight(.bold)).foregroundStyle(DColor.agree)
                                    .frame(width: 38, alignment: .leading)
                                Text(s.text).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            }
                        }
                    }
                    .padding(12).background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                }

                HStack(spacing: 8) {
                    Image(systemName: "sparkles").foregroundStyle(DeliberationStage.distil.tint)
                    TextField("Distil a finding from the discussion…", text: $draft, axis: .vertical).lineLimit(1...3)
                    Button {
                        store.synthesize(draft, from: Array(store.consensus.prefix(3).map(\.id))); draft = ""
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    } label: { Image(systemName: "arrow.up.circle.fill").font(.title2) }
                        .disabled(draft.trimmingCharacters(in: .whitespaces).isEmpty)
                        .foregroundStyle(DeliberationStage.distil.tint)
                }
                .padding(12).background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))

                if store.findings.isEmpty {
                    empty
                } else {
                    ForEach(store.findings) { f in card(f) }
                }
            }
            .padding()
        }
        .background(Brand.groupBG)
    }

    private func card(_ f: DFinding) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(f.text).font(.subheadline.weight(.medium))
                .strikethrough(f.superseded).foregroundStyle(f.superseded ? .secondary : .primary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) {
                Label("\(f.from.count) statements", systemImage: "link").font(.caption2).foregroundStyle(.tertiary)
                Spacer()
                Button { store.supersede(f.id) } label: {
                    Text(f.superseded ? "Restore" : "Supersede").font(.caption.weight(.medium))
                }.buttonStyle(.plain).foregroundStyle(.secondary)
                if !f.superseded {
                    Button(action: onDecide) {
                        Label("Put to the group", systemImage: "arrow.right").font(.caption.weight(.semibold))
                    }.buttonStyle(.plain).foregroundStyle(DeliberationStage.decide.tint)
                }
            }
        }
        .padding(13).background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private var empty: some View {
        VStack(spacing: 6) {
            Image(systemName: "sparkles").font(.system(size: 26)).foregroundStyle(.tertiary)
            Text("No findings yet").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
            Text("Synthesise the discussion into a sourced insight — the report spine.").font(.caption).foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity).padding(.vertical, 34)
    }
}

// MARK: - DECIDE

struct DecideStage: View {
    @ObservedObject var store: DeliberationStore
    @State private var proposeFor: DFinding?

    private var ratifiable: [DFinding] {
        store.findings.filter { f in !f.superseded && !store.ratifications.contains { $0.findingId == f.id } }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                StageHeader(stage: .decide, subtitle: "\(store.ratifications.filter { !$0.closed }.count) open · \(store.ratifications.filter(\.closed).count) closed")

                if !ratifiable.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("READY TO PUT TO THE GROUP").font(.system(size: 11, weight: .bold)).tracking(1).foregroundStyle(.secondary)
                        ForEach(ratifiable) { f in
                            HStack(spacing: 10) {
                                Text(f.text).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                Spacer()
                                Button { proposeFor = f } label: {
                                    Text("Propose").font(.caption.weight(.semibold))
                                        .padding(.horizontal, 11).padding(.vertical, 6)
                                        .background(DeliberationStage.decide.tint, in: Capsule()).foregroundStyle(.white)
                                }.buttonStyle(.plain)
                            }
                            .padding(11).background(Brand.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                        }
                    }
                }

                if store.ratifications.isEmpty && ratifiable.isEmpty {
                    empty
                } else {
                    ForEach(store.ratifications) { r in ratCard(r) }
                }
            }
            .padding()
        }
        .background(Brand.groupBG)
        .sheet(item: $proposeFor) { f in ProposeSheet(store: store, finding: f) }
    }

    private func ratCard(_ r: DRatification) -> some View {
        let t = store.ratTally(r.id)
        let diss = store.dissenters(r.id)
        return VStack(alignment: .leading, spacing: 11) {
            HStack {
                Text(store.finding(r.findingId)?.text ?? "—").font(.subheadline.weight(.medium)).fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
            }
            HStack(spacing: 6) {
                policyPill("threshold: \(r.threshold)")
                policyPill("quorum: \(r.quorum)")
                Spacer()
                if r.closed {
                    Text(r.outcome.uppercased()).font(.caption2.weight(.bold))
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .background((r.outcome == "carried" ? DColor.agree : DColor.disagree).opacity(0.18), in: Capsule())
                        .foregroundStyle(r.outcome == "carried" ? DColor.agree : DColor.disagree)
                }
            }
            if !r.closed {
                HStack(spacing: 8) {
                    assentButton(r, .assent, "Assent", DColor.agree)
                    assentButton(r, .abstain, "Abstain", DColor.pass)
                    assentButton(r, .dissent, "Dissent", DColor.disagree)
                }
            }
            HStack(spacing: 14) {
                count("\(t.assent)", "assent", DColor.agree)
                count("\(t.abstain)", "abstain", DColor.pass)
                count("\(t.dissent)", "dissent", DColor.disagree)
                Spacer()
                if !r.closed {
                    Button { store.close(r.id); UINotificationFeedbackGenerator().notificationOccurred(.success) } label: {
                        Text("Close").font(.caption.weight(.semibold))
                    }.buttonStyle(.plain).foregroundStyle(DeliberationStage.decide.tint)
                }
            }
            if !diss.isEmpty {
                // Dissent preserved — never averaged away.
                Label("Dissent recorded: \(diss.map { $0 == store.me ? "You" : String($0.prefix(6)) }.joined(separator: ", "))",
                      systemImage: "exclamationmark.bubble")
                    .font(.caption2).foregroundStyle(DColor.disagree)
            }
        }
        .padding(13).background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(DeliberationStage.decide.tint.opacity(r.closed ? 0 : 0.35), lineWidth: 1))
    }

    private func assentButton(_ r: DRatification, _ a: DAssent, _ label: String, _ c: Color) -> some View {
        let on = store.myAssent(r.id) == a
        return Button { store.cast(r.id, a); UISelectionFeedbackGenerator().selectionChanged() } label: {
            Text(label).font(.caption.weight(.semibold)).frame(maxWidth: .infinity).padding(.vertical, 8)
                .background(on ? c : Theme.fill, in: RoundedRectangle(cornerRadius: 9))
                .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(on ? .clear : c.opacity(0.4), lineWidth: 1))
                .foregroundStyle(on ? .white : c)
        }.buttonStyle(.plain)
    }

    private func policyPill(_ s: String) -> some View {
        Text(s).font(.system(size: 9.5, weight: .semibold)).padding(.horizontal, 7).padding(.vertical, 3)
            .background(Theme.fill, in: Capsule()).foregroundStyle(.secondary)
    }
    private func count(_ v: String, _ l: String, _ c: Color) -> some View {
        HStack(spacing: 4) { Text(v).font(.subheadline.weight(.bold)).foregroundStyle(c); Text(l).font(.caption2).foregroundStyle(.secondary) }
    }
    private var empty: some View {
        VStack(spacing: 6) {
            Image(systemName: "checkmark.seal").font(.system(size: 26)).foregroundStyle(.tertiary)
            Text("Nothing put to the group yet").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
            Text("Distil a finding, then propose it here. Ratify with quorum — dissent is kept.").font(.caption).foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity).padding(.vertical, 34)
    }
}

private struct ProposeSheet: View {
    @ObservedObject var store: DeliberationStore
    let finding: DFinding
    @Environment(\.dismiss) private var dismiss
    @State private var threshold = "consensus"
    @State private var quorum = 1

    var body: some View {
        NavigationStack {
            Form {
                Section("Finding") { Text(finding.text).font(.subheadline) }
                Section("Decision rule") {
                    Picker("Threshold", selection: $threshold) {
                        Text("Consensus").tag("consensus")
                        Text("Supermajority").tag("supermajority")
                        Text("Majority").tag("majority")
                    }
                    Stepper("Quorum: \(quorum)", value: $quorum, in: 1...50)
                }
            }
            .navigationTitle("Propose ratification").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Propose") { store.propose(findingId: finding.id, threshold: threshold, quorum: quorum); dismiss() }
                }
            }
        }
    }
}

// MARK: - FRAME — the typed node / edge editor (Group · Group · Place)
//
// The nodes this deliberation is wired to, by construction: Participants & Stakeholders are
// Group nodes, Locations are Place nodes. Every + / × authors a REAL project delta (setRole ·
// add/removeStakeholder · add/removeLocation · setKpi), so an edge added here syncs to members.

private struct NodeChipModel: Identifiable { let id: String; let label: String; var removable: Bool = false }

struct FrameStage: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String

    @State private var obj: ObjectivesRow?
    @State private var members: [String] = []
    @State private var headline = ""
    @State private var goal = ""
    @State private var editObjective = false
    @State private var showAddPeople = false
    @State private var addStake = false
    @State private var addLoc = false
    @State private var addKpi = false

    private let groupTint = Color(hex: "8b5cf6")
    private let placeTint = Color(hex: "10b981")
    private let kpiTint = Color(hex: "f59e0b")

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                StageHeader(stage: .frame, subtitle: "the nodes this deliberation is wired to")

                objectiveCard

                nodeSection("Participants", "Group", "person.2.fill", groupTint,
                            chips: members.map { NodeChipModel(id: $0, label: $0 == core.myIdentityHex ? "You" : String($0.prefix(8))) },
                            onAdd: { showAddPeople = true })

                nodeSection("Stakeholders", "Group", "building.2.fill", groupTint,
                            chips: (obj?.stakeholders ?? []).map { NodeChipModel(id: $0.id, label: $0.name, removable: true) },
                            onRemove: { author(projectRemoveStakeholder(stakeholderId: $0)) }, onAdd: { addStake = true })

                nodeSection("Locations", "Place", "mappin.circle.fill", placeTint,
                            chips: (obj?.locations ?? []).map { NodeChipModel(id: $0.id, label: $0.name, removable: true) },
                            onRemove: { author(projectRemoveLocation(locationId: $0)) }, onAdd: { addLoc = true })

                nodeSection("Measures", "KPI", "target", kpiTint,
                            chips: (obj?.kpis ?? []).map { NodeChipModel(id: $0.id, label: $0.label + ($0.target.isEmpty ? "" : " · " + $0.target), removable: true) },
                            onRemove: { author(projectRemoveKpi(kpiId: $0)) }, onAdd: { addKpi = true })

                Text("Each chip is a node; + and × add and remove edges — real project deltas, synced to members.")
                    .font(.caption2).foregroundStyle(.tertiary).padding(.top, 2)
            }
            .padding()
        }
        .background(Brand.groupBG)
        .sheet(isPresented: $showAddPeople) { AddPeopleSheet(objectId: objectId, label: "this project") { refresh() } }
        .sheet(isPresented: $addStake) { SmallAddSheet(title: "Stakeholder · Group", fields: ["Name", "Note"]) { v in
            author(projectAddStakeholder(stakeholderId: UUID().uuidString, name: v[0], note: v.count > 1 ? v[1] : "")) } }
        .sheet(isPresented: $addLoc) { SmallAddSheet(title: "Location · Place", fields: ["Name"]) { v in
            author(projectAddLocation(locationId: UUID().uuidString, name: v[0])) } }
        .sheet(isPresented: $addKpi) { SmallAddSheet(title: "Measure · KPI", fields: ["Label", "Target"]) { v in
            author(projectSetKpi(kpiId: UUID().uuidString, label: v[0], target: v.count > 1 ? v[1] : "")) } }
        .sheet(isPresented: $editObjective) { objectiveEditor }
        .onAppear(perform: refresh)
        .onChange(of: core.syncTick) { _, _ in refresh() }
    }

    private var objectiveCard: some View {
        Button { editObjective = true } label: {
            VStack(alignment: .leading, spacing: 4) {
                Text((obj?.headline.isEmpty == false) ? obj!.headline : "Set the objective…")
                    .font(.headline).foregroundStyle((obj?.headline.isEmpty == false) ? .primary : .secondary)
                if let g = obj?.goal, !g.isEmpty { Text(g).font(.subheadline).foregroundStyle(.secondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14).background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(DeliberationStage.frame.tint.opacity(0.35), lineWidth: 1))
        }.buttonStyle(.plain)
    }

    private var objectiveEditor: some View {
        NavigationStack {
            Form {
                TextField("Headline", text: $headline)
                TextField("Goal — what does done look like?", text: $goal, axis: .vertical).lineLimit(2...6)
            }
            .navigationTitle("Objective").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { editObjective = false } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { author(projectSetObjective(headline: headline, goal: goal)); editObjective = false }
                }
            }
            .onAppear { headline = obj?.headline ?? ""; goal = obj?.goal ?? "" }
        }
    }

    @ViewBuilder
    private func nodeSection(_ title: String, _ kind: String, _ icon: String, _ tint: Color,
                             chips: [NodeChipModel], onRemove: ((String) -> Void)? = nil, onAdd: @escaping () -> Void) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 7) {
                Image(systemName: icon).font(.system(size: 12, weight: .bold)).foregroundStyle(tint)
                Text(title.uppercased()).font(.system(size: 12, weight: .bold)).tracking(0.8)
                Text(kind).font(.system(size: 10, weight: .semibold)).foregroundStyle(tint)
                    .padding(.horizontal, 6).padding(.vertical, 2).background(tint.opacity(0.14), in: Capsule())
                Spacer()
            }
            WrapLayout(spacing: 7) {
                ForEach(chips) { c in NodeChip(label: c.label, tint: tint, onRemove: c.removable ? { onRemove?(c.id) } : nil) }
                Button(action: onAdd) {
                    Label("add", systemImage: "plus").font(.caption.weight(.semibold))
                        .padding(.horizontal, 11).padding(.vertical, 7)
                        .background(tint.opacity(0.12), in: Capsule())
                        .overlay(Capsule().strokeBorder(tint.opacity(0.45), style: StrokeStyle(lineWidth: 1, dash: [3])))
                        .foregroundStyle(tint)
                }.buttonStyle(.plain)
            }
        }
    }

    private func refresh() { obj = core.projectObjectives(objectId); members = core.objectMembers(objectId) }
    private func author(_ draft: DeltaDraft) { Task { @MainActor in _ = await core.projectAuthor(objectId, draft); refresh() } }
}

private struct NodeChip: View {
    let label: String; var tint: Color = .secondary; var onRemove: (() -> Void)? = nil
    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(tint).frame(width: 7, height: 7)
            Text(label).font(.system(size: 13, weight: .medium)).lineLimit(1)
            if let onRemove {
                Button(action: onRemove) { Image(systemName: "xmark.circle.fill").font(.system(size: 14)) }
                    .buttonStyle(.plain).foregroundStyle(.tertiary)
            }
        }
        .padding(.leading, 11).padding(.trailing, onRemove == nil ? 12 : 6).padding(.vertical, 7)
        .background(Brand.card, in: Capsule())
        .overlay(Capsule().strokeBorder(tint.opacity(0.28), lineWidth: 1))
    }
}

private struct SmallAddSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String; let fields: [String]; let onSave: ([String]) -> Void
    @State private var values: [String]
    init(title: String, fields: [String], onSave: @escaping ([String]) -> Void) {
        self.title = title; self.fields = fields; self.onSave = onSave
        _values = State(initialValue: Array(repeating: "", count: fields.count))
    }
    var body: some View {
        NavigationStack {
            Form {
                ForEach(fields.indices, id: \.self) { i in
                    TextField(fields[i], text: Binding(get: { values[i] }, set: { values[i] = $0 }))
                }
            }
            .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { onSave(values); dismiss() }
                        .disabled(values.first?.trimmingCharacters(in: .whitespaces).isEmpty ?? true)
                }
            }
        }
        .presentationDetents([.medium])
    }
}
