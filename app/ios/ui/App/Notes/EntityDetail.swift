//  EntityDetail.swift
//  One card per entity (person / org / place / event / action). Reads the LIVE
//  entity from the observable model, so edits reflect immediately. Long-press the
//  header (or the toolbar pencil) to edit; "+" buttons add actions / people.

import SwiftUI
import PacificCore   // ObjectRow

struct EntityDetailView: View {
    private let entityValue: EntityVM
    @ObservedObject private var model = OutlineModel.shared
    @ObservedObject private var scanStore = ScanStore.shared
    /// Places are GroupObjects, not outline rows, so their access posture and join QR come
    /// from the core rather than from `model`.
    @EnvironmentObject private var core: CoreService
    @State private var sheet: OutlineSheet?
    @State private var showAddPeople = false

    init(entity: EntityVM) { entityValue = entity }
    private var entity: EntityVM { model.entity(entityValue.id) ?? entityValue }

    private var inbound: [EntityVM] { model.inbound(to: entity.id) }
    private var container: EntityVM? { entity.containerID.flatMap { model.entity($0) } }
    private var children: [EntityVM] { model.children(of: entity.id) }
    private var myActions: [EntityVM] { model.actions(of: entity.id) }
    private var scans: [Scan] { scanStore.scans(forPlace: entity.id) }

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                header

                if entity.isAction { assignmentSection }
                if let w = entity.window { whenSection(w) }
                if entity.unminted { mintBanner }
                if container != nil || !inbound.isEmpty { contextSection }
                if !entity.attributes.isEmpty || entity.notes != nil || entity.availability != nil { notesSection }

                if let obj = mintedObject { membersSection(obj) }
                if !children.isEmpty || entity.kind == .org || entity.kind == .place { childrenSection }
                if entity.kind == .place { publicSection }
                if entity.kind == .place { scansSection }
                if !myActions.isEmpty || entity.kind != .event { actionsSection }

                mentionedSection

                if entity.isAction {
                    TrustLine(text: "An action — an unresolved event, assigned to a Group")
                } else if entity.kind == .place {
                    TrustLine(text: "A place — a first-class entity, local to your device (no peer identity)")
                } else if entity.kind == .event {
                    TrustLine(text: "An event — a window with a start and an end")
                } else if entity.offPlatform {
                    TrustLine(text: "Off-platform — this card lights up when they join WallFlowers")
                }
            }
            .padding()
        }
        .background(Brand.groupBG)
        .navigationTitle(entity.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { sheet = .edit(entity) } label: { Image(systemName: "pencil") }
            }
        }
        .sheet(item: $sheet) { outlineSheet($0) }
    }

    private var header: some View {
        VStack(spacing: 8) {
            LogoBadge(initials: entity.initials, tint: entity.tint, size: 72)
            Text(entity.name).font(.title2.bold())
            Label(entity.descriptor ?? entity.kind.word, systemImage: entity.kind.glyph)
                .font(.subheadline).foregroundStyle(.secondary)
            if entity.waiting { WaitingPill() }
            Text("Long-press or tap ✎ to refine")
                .font(.caption2).foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 4)
        .contentShape(Rectangle())
        .onLongPressGesture { sheet = .edit(entity) }
    }

    private var assignmentSection: some View {
        ProfileSection(title: "Action", systemImage: entity.actionWaiting ? "hourglass" : "circle") {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("Assigned to").foregroundStyle(.secondary)
                    Spacer()
                    if let a = entity.assignee, a != "me" { EntityChip(id: a) }
                    else { Text("You").fontWeight(.medium) }
                }
                if let d = entity.due {
                    HStack { Text("When").foregroundStyle(.secondary); Spacer(); TimeChipView(chip: d) }
                }
                Text(entity.actionWaiting ? "Waiting on them — the ball is in their court." : "Your move.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .font(.subheadline)
        }
    }

    private func whenSection(_ w: EventWindow) -> some View {
        ProfileSection(title: "When", systemImage: "calendar") {
            VStack(spacing: 8) {
                HStack { Text("Starts").foregroundStyle(.secondary); Spacer(); Text(w.start).fontWeight(.medium) }
                HStack { Text("Ends").foregroundStyle(.secondary); Spacer(); Text(w.end).fontWeight(.medium) }
            }
            .font(.subheadline)
        }
    }

    private var mintBanner: some View {
        Label("Not added yet — tap to create this card", systemImage: "plus.circle")
            .font(.caption).foregroundStyle(.orange)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(Color.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private var contextSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let c = container {
                HStack(spacing: 6) {
                    Text("in").font(.caption).foregroundStyle(.secondary)
                    EntityChip(id: c.id)
                }
            }
            if !inbound.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Referenced from").font(.caption).foregroundStyle(.secondary)
                    WrapLayout(spacing: 6) { ForEach(inbound) { EntityChip(id: $0.id) } }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private var notesSection: some View {
        ProfileSection(title: "Notes", systemImage: "text.alignleft") {
            VStack(alignment: .leading, spacing: 8) {
                if let av = entity.availability {
                    HStack(spacing: 6) { Image(systemName: "clock").foregroundStyle(.secondary); TimeChipView(chip: av) }
                }
                if !entity.attributes.isEmpty { ChipFlow(items: entity.attributes) }
                if let n = entity.notes { Text(n).font(.subheadline) }
            }
        }
    }

    private var childrenSection: some View {
        let people = children.filter { $0.kind == .person }
        let spaces = children.filter { $0.kind != .person }
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(entity.kind == .place ? "Inside" : "People & parts").font(.subheadline.weight(.semibold))
                Spacer()
                Button { sheet = .addChild(entity) } label: { Image(systemName: "plus.circle") }
                    .foregroundStyle(Brand.accent)
            }
            if children.isEmpty {
                Text("Nothing added yet.").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(spaces) { SpaceRow(entity: $0, present: { sheet = $0 }) }
            ForEach(people) { PersonRow(person: $0, present: { sheet = $0 }) }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// Open this Place to the public, and show the poster once it is open.
    ///
    /// The copy says "while this phone is running" because that is the one thing about
    /// auto-accept a host can be surprised by: MLS admits nobody without a member
    /// committing them, so somebody's device does the letting-in. Saying "anyone can
    /// join" flat would promise something that silently does not happen with the phone
    /// off (see PublicPlace.swift).
    private var publicSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Open to the public").font(.subheadline.weight(.semibold))
                Spacer()
                Toggle("", isOn: Binding(
                    get: { isPublicPlace },
                    set: { core.setPlaceAccess(entity.id, public: $0) }))
                    .labelsHidden()
            }
            if isPublicPlace, let card = core.placeJoinCard(entity.id) {
                HStack { Spacer(); QRView(payload: card, size: 200); Spacer() }
                Text("Print this. Anyone who scans it joins automatically — while this phone is running.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else if !isPublicPlace {
                Text("Private. Only people you add can see this place.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// Read off the folded GroupObject, not the passed-in row, so the toggle reflects what
    /// the core actually holds after a set — including a set that failed.
    private var isPublicPlace: Bool {
        core.places.first { $0.id == entity.id }?.access == "public"
    }

    private var scansSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Scans").font(.subheadline.weight(.semibold))
                Spacer()
                Image(systemName: "cube.transparent").foregroundStyle(.secondary)
            }
            if scans.isEmpty {
                Text("No scans yet — captured 3D spaces of this place will appear here.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(scans) { scan in
                ScanRow(scan: scan) { scanStore.delete(scan) }
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// The minted GroupObject behind this card, when there is one (a projected LIFE
    /// entity's id IS its object id). nil for outline rows and unminted ghosts.
    private var mintedObject: ObjectRow? {
        core.objects.first { $0.id == entity.id }
    }

    /// A minted card IS an MLS group, so sharing it is adding members: everyone added
    /// folds the same log and holds the same card — a shared object, never a copy.
    private func membersSection(_ obj: ObjectRow) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Members").font(.subheadline.weight(.semibold))
                Spacer()
                Button { showAddPeople = true } label: { Image(systemName: "plus.circle") }
                    .foregroundStyle(Brand.accent)
            }
            ForEach(core.objectMembers(obj.id), id: \.self) { hex in
                Text(core.displayName(forSpace: IdentityKey.display(hex))).font(.subheadline)
            }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .sheet(isPresented: $showAddPeople) {
            AddPeopleSheet(objectId: obj.id, label: entity.name) {}
        }
    }

    private var actionsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Actions").font(.subheadline.weight(.semibold))
                Spacer()
                Button { sheet = .addAction(entity) } label: { Image(systemName: "plus.circle") }
                    .foregroundStyle(Brand.accent)
            }
            if myActions.isEmpty {
                Text("No actions yet — add one with ⊕.").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(myActions) { ActionRow(action: $0, present: { sheet = $0 }) }
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    @ViewBuilder
    private var mentionedSection: some View {
        let outbound = Array(Set(entity.todos.flatMap(\.linkIDs) + entity.linkIDs))
            .compactMap { model.entity($0) }
            .sorted { $0.name < $1.name }
        if !outbound.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("Linked").font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                WrapLayout(spacing: 6) { ForEach(outbound) { EntityChip(id: $0.id) } }
            }
            .padding()
            .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
    }
}
