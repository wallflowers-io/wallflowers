//  ProjectDetailView.swift
//  The REAL Project screen — a vertical Gantt folded from a live Project
//  GroupObject (pacific-core `ProjectType`), NOT mockup data. Every timeline item,
//  its derived `blocked` state, assignees, schedule, dependency edges and
//  deliverables come from the structured projections over the FFI, and every
//  control authors a real delta through `CoreService.projectAuthor` — so an edit
//  syncs to members over the same MLS+relay transport as the rest of the app.
//
//  Write-completeness: the toolbar and the item editor expose ALL of the Project
//  op set — configure, schedule, assignee, add/remove dependency, custom fields,
//  suppress, subscribe/unsubscribe, add-people — so no built op is left with a
//  field that renders but can't be authored. Enforced by scripts/capability-parity.sh.

import SwiftUI
import PacificCore

/// Identifies the item whose editor sheet is open (String isn't Identifiable).
struct EditTarget: Identifiable { let id: String }

/// The Project screen: three swipeable tabs — Objectives (the human-authored spine),
/// Timeline (the plan), and the linked ChatRoom.
struct ProjectDetailView: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String

    /// The deliberation model (Gather / Distil / Decide). Durable per project; swaps for the
    /// Delta path when the core OG_Deliberation reducer lands.
    @StateObject private var delib: DeliberationStore
    @State private var stage: DeliberationStage = .frame
    @State private var header: ProjectHeaderRow?
    @State private var showSettings = false
    @State private var showAddPeople = false
    @State private var showChat = false

    init(objectId: String) {
        self.objectId = objectId
        _delib = StateObject(wrappedValue: DeliberationStore(objectId: objectId))
    }

    var body: some View {
        VStack(spacing: 0) {
            ProjectStageRail(stage: $stage)

            TabView(selection: $stage) {
                FrameStage(objectId: objectId).tag(DeliberationStage.frame)   // FRAME — typed Group·Place nodes
                GatherStage(store: delib).tag(DeliberationStage.gather)
                DistilStage(store: delib, onDecide: { withAnimation(.spring(response: 0.4, dampingFraction: 0.8)) { stage = .decide } })
                    .tag(DeliberationStage.distil)
                DecideStage(store: delib).tag(DeliberationStage.decide)
                ProjectTimelineTab(objectId: objectId).tag(DeliberationStage.deliver) // DELIVER — real plan
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
        }
        .background(Brand.groupBG)
        .navigationTitle((header?.title.isEmpty == false) ? header!.title : "Untitled project")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button { showChat = true } label: { Label("Channel", systemImage: "bubble.left.and.bubble.right") }
                    Button { showAddPeople = true } label: { Label("Add people", systemImage: "person.badge.plus") }
                    Button { showSettings = true } label: { Label("Project settings", systemImage: "gearshape") }
                } label: { Image(systemName: "ellipsis.circle") }
                    .accessibilityLabel("Project options")
            }
        }
        .sheet(isPresented: $showSettings) {
            ProjectSettingsSheet(objectId: objectId, current: header) { header = core.projectHeader(objectId) }
        }
        .sheet(isPresented: $showAddPeople) { AddPeopleSheet(objectId: objectId, label: "this project") {} }
        .sheet(isPresented: $showChat) {
            NavigationStack {
                ProjectChatRoomTab(objectId: objectId)
                    .navigationTitle("Channel").navigationBarTitleDisplayMode(.inline)
            }
        }
        .onAppear { header = core.projectHeader(objectId); delib.me = core.myIdentityHex }
        .onChange(of: core.syncTick) { _, _ in header = core.projectHeader(objectId) }
        .onReceive(NotificationCenter.default.publisher(for: .pacificProjectSheet)) { note in
            guard note.userInfo?["object"] as? String == objectId else { return }
            switch note.userInfo?["sheet"] as? String {
            case "settings": showSettings = true
            case "people":   showAddPeople = true
            default: break
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificProjectTab)) { note in
            guard note.userInfo?["object"] as? String == objectId,
                  let n = note.userInfo?["n"] as? Int, let s = DeliberationStage(rawValue: n) else { return }
            withAnimation(.spring(response: 0.4, dampingFraction: 0.8)) { stage = s }
        }
    }
}

/// Tab 2 — the timeline: the Gantt of items + deliverables. (Brick 2
/// reorganizes this into per-stakeholder swimlanes; today it is the flat Gantt.)
struct ProjectTimelineTab: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String

    @State private var items: [ProjectItemRow] = []
    @State private var deliverables: [DeliverableRow] = []
    @State private var header: ProjectHeaderRow?
    @State private var showAdd = false
    @State private var showAddDeliverable = false
    @State private var editTarget: EditTarget?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if items.isEmpty {
                    Text("No timeline items yet — add one to watch the Gantt fold from real deltas.")
                        .font(.callout).foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity).padding(.vertical, 28)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                            ProjectTimelineRow(
                                item: item,
                                isLast: idx == items.count - 1,
                                onTapNode: { cycleStatus(item) },
                                onEdit: { editTarget = EditTarget(id: item.id) }
                            )
                        }
                    }
                }

                Button { showAdd = true } label: { Label("Add timeline item", systemImage: "plus.circle") }
                    .font(.subheadline)

                SectionHeader(title: "Deliverables", accessory: "\(header?.pieceCount ?? 0) pieces")
                ForEach(deliverables, id: \.subId) { d in
                    HStack(spacing: 8) {
                        NavigationLink(value: d.target) { DeliverableRowView(deliverable: d) }
                            .buttonStyle(.plain)
                        Button(role: .destructive) { author(projectUnsubscribe(subId: d.subId)) } label: {
                            Image(systemName: "minus.circle").foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                Button { showAddDeliverable = true } label: {
                    Label("Add deliverable", systemImage: "plus.circle")
                }
                .font(.subheadline).padding(.top, 2)

                TrustLine(text: "Every row is a real delta — tap a node to set progress, tap a card to edit; it syncs to members")
            }
            .padding()
        }
        .background(Brand.groupBG)
        .sheet(isPresented: $showAdd) { AddItemSheet(objectId: objectId) { reload() } }
        .sheet(isPresented: $showAddDeliverable) { AddDeliverableSheet(objectId: objectId, items: items) { reload() } }
        .sheet(item: $editTarget) { t in ItemEditSheet(objectId: objectId, itemId: t.id) { reload() } }
        .onAppear { reload() }
        .onChange(of: core.syncTick) { _, _ in reload() }
        .onReceive(NotificationCenter.default.publisher(for: .pacificProjectSheet)) { note in
            guard note.userInfo?["object"] as? String == objectId,
                  note.userInfo?["sheet"] as? String == "item",
                  let first = items.first else { return }
            editTarget = EditTarget(id: first.id)
        }
    }

    private func reload() {
        items = core.projectItems(objectId)
        deliverables = core.projectDeliverables(objectId)
        header = core.projectHeader(objectId)
    }

    /// Author one Project delta, then reload once it's folded. The single write path
    /// every control funnels through.
    private func author(_ draft: DeltaDraft) {
        let oid = objectId
        Task { @MainActor in
            _ = await core.projectAuthor(oid, draft)
            reload()
        }
    }

    /// Tap a node → advance status (open → in_progress → done → open). The core
    /// stamps the LWW `gen`, so the UI passes 0.
    private func cycleStatus(_ item: ProjectItemRow) {
        let next: String
        switch item.status {
        case "in_progress": next = "done"
        case "done":        next = "open"
        default:            next = "in_progress"
        }
        author(projectSetItemProgress(itemId: item.id, status: next, gen: 0))
    }
}

// MARK: - One timeline row (rail + card; tap node = status, tap card = edit)

private struct ProjectTimelineRow: View {
    let item: ProjectItemRow
    let isLast: Bool
    var onTapNode: () -> Void
    var onEdit: () -> Void

    private var tint: Color {
        if item.blocked { return .orange }
        switch item.status {
        case "in_progress": return Brand.accent
        case "done":        return Color(.systemGray3)
        default:            return Color(.systemGray2)
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(spacing: 0) {
                Button(action: onTapNode) { node }
                    .buttonStyle(.plain)
                    .frame(width: 20, height: 20)
                if !isLast {
                    Rectangle().fill(Color(.systemGray4)).frame(width: 2).frame(maxHeight: .infinity)
                }
            }
            .frame(width: 20)
            card
                .contentShape(Rectangle())
                .onTapGesture(perform: onEdit)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var node: some View {
        switch item.kind {
        case "gate":
            Image(systemName: "diamond.fill").font(.system(size: 12)).foregroundStyle(tint)
        case "event":
            RoundedRectangle(cornerRadius: 3).fill(tint).frame(width: 12, height: 12)
        default:
            Circle()
                .strokeBorder(tint, lineWidth: 2)
                .background(Circle().fill(item.status == "done" ? tint : .clear))
                .frame(width: 12, height: 12)
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(item.title).font(.subheadline.weight(.semibold))
                Spacer(minLength: 8)
                if let at = item.at {
                    Text(Self.dateStr(at)).font(.caption2).foregroundStyle(.secondary).fixedSize()
                }
            }
            HStack(spacing: 6) {
                statusPill
                if item.blocked {
                    Text("blocked")
                        .font(.system(size: 9, weight: .semibold))
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Capsule().fill(Color.orange.opacity(0.16)))
                        .foregroundStyle(.orange)
                }
                Spacer()
                ForEach(item.assignees.prefix(4), id: \.self) { a in
                    Avatar(initials: String(a.prefix(2)).uppercased(), tint: Brand.accent, size: 20)
                }
            }
        }
        .padding(12)
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .padding(.bottom, 10)
    }

    @ViewBuilder private var statusPill: some View {
        let label = item.status.isEmpty ? "—" : item.status.replacingOccurrences(of: "_", with: " ")
        Text(label)
            .font(.system(size: 9, weight: .semibold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Capsule().fill(tint.opacity(0.16)))
            .foregroundStyle(tint)
    }

    private static func dateStr(_ epochSecs: Int64) -> String {
        let f = DateFormatter(); f.dateFormat = "d MMM"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(epochSecs)))
    }
}

// MARK: - One deliverable (a child-Project subscription), collapsed

private struct DeliverableRowView: View {
    let deliverable: DeliverableRow

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "shippingbox.fill").foregroundStyle(Brand.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(deliverable.target.prefix(12) + "…")
                    .font(.system(.subheadline, design: .monospaced))
                Text("piece · \(deliverable.disclosure)")
                    .font(.caption2).foregroundStyle(.secondary)
            }
            Spacer()
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
        }
        .padding(12)
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

// MARK: - Add a timeline item (project.addTimelineItem)

private struct AddItemSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    var onDone: () -> Void

    @State private var title = ""
    @State private var kind = "action"

    var body: some View {
        NavigationStack {
            Form {
                TextField("Title", text: $title)
                    .accessibilityIdentifier("item-title-field")
                Picker("Kind", selection: $kind) {
                    Text("Action").tag("action")
                    Text("Event").tag("event")
                    Text("Gate").tag("gate")
                }
            }
            .navigationTitle("New timeline item")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add", action: add).disabled(title.isEmpty)
                }
            }
        }
    }

    private func add() {
        let draft = projectAddTimelineItem(itemId: UUID().uuidString, kind: kind, title: title)
        let oid = objectId
        Task { @MainActor in
            _ = await core.projectAuthor(oid, draft)
            onDone(); dismiss()
        }
    }
}

// MARK: - Project settings (project.configure — title + status)

private struct ProjectSettingsSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    let current: ProjectHeaderRow?
    var onDone: () -> Void

    @State private var title = ""
    @State private var status = "active"

    var body: some View {
        NavigationStack {
            Form {
                TextField("Project name", text: $title)
                    .accessibilityIdentifier("project-name-field")
                Picker("Status", selection: $status) {
                    Text("Active").tag("active")
                    Text("Paused").tag("paused")
                    Text("Archived").tag("archived")
                }
            }
            .navigationTitle("Project settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Save", action: save) }
            }
            .onAppear {
                title = current?.title ?? ""
                status = (current?.status.isEmpty == false) ? current!.status : "active"
            }
        }
    }

    private func save() {
        let draft = projectConfigure(title: title, status: status)
        let oid = objectId
        Task { @MainActor in
            _ = await core.projectAuthor(oid, draft)
            onDone(); dismiss()
        }
    }
}

// MARK: - Add a deliverable (project.subscribe — a child-Project edge)

private struct AddDeliverableSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    let items: [ProjectItemRow]
    var onDone: () -> Void

    @State private var target = ""
    @State private var disclosure = "summary"
    @State private var originItem = ""

    private var candidates: [ObjectRow] { core.projectObjects.filter { $0.id != objectId } }

    var body: some View {
        NavigationStack {
            Form {
                if candidates.isEmpty {
                    Text("A deliverable is a child project. Create another project first, then link it here.")
                        .foregroundStyle(.secondary)
                } else {
                    Picker("Deliverable (a project)", selection: $target) {
                        Text("Choose…").tag("")
                        ForEach(candidates, id: \.id) { p in
                            Text(label(p)).tag(p.id)
                        }
                    }
                    Picker("Disclosure", selection: $disclosure) {
                        Text("Existence").tag("existence")
                        Text("Summary").tag("summary")
                        Text("Full").tag("full")
                    }
                    Picker("Pin to timeline item", selection: $originItem) {
                        Text("None").tag("")
                        ForEach(items, id: \.id) { it in Text(it.title).tag(it.id) }
                    }
                }
            }
            .navigationTitle("Add deliverable")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add", action: add).disabled(target.isEmpty)
                }
            }
        }
    }

    private func label(_ p: ObjectRow) -> String {
        let t = core.projectHeader(p.id)?.title ?? ""
        return t.isEmpty ? String(p.id.prefix(10)) + "…" : t
    }

    private func add() {
        let draft = projectSubscribe(
            subId: UUID().uuidString,
            target: target,
            kind: "project",
            disclosure: disclosure,
            originItem: originItem.isEmpty ? nil : originItem
        )
        let oid = objectId
        Task { @MainActor in
            _ = await core.projectAuthor(oid, draft)
            onDone(); dismiss()
        }
    }
}

// MARK: - Edit one item (schedule · assignees · dependencies · fields · suppress)

private struct ItemEditSheet: View {
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    let objectId: String
    let itemId: String
    var onChange: () -> Void

    @State private var item: ProjectItemRow?
    @State private var allItems: [ProjectItemRow] = []
    @State private var members: [String] = []
    @State private var deps: [DependencyRow] = []

    @State private var scheduled = false
    @State private var startDate = Date()
    @State private var hasEnd = false
    @State private var endDate = Date()
    @State private var blockerId = ""
    @State private var fieldKey = ""
    @State private var fieldValue = ""

    var body: some View {
        NavigationStack {
            Form {
                scheduleSection
                assigneeSection
                blockedBySection
                fieldsSection
                Section {
                    Button(role: .destructive) {
                        author(projectSuppressItem(itemId: itemId, suppressed: true))
                        dismiss()
                    } label: { Label("Remove from timeline", systemImage: "trash") }
                }
            }
            .navigationTitle(item?.title ?? "Item")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .onAppear(perform: reload)
        }
    }

    // MARK: sections

    private var scheduleSection: some View {
        Section("Schedule") {
            Toggle("Scheduled", isOn: $scheduled)
            if scheduled {
                DatePicker("Starts", selection: $startDate, displayedComponents: .date)
                Toggle("Has end date", isOn: $hasEnd)
                if hasEnd { DatePicker("Ends", selection: $endDate, displayedComponents: .date) }
                Button("Save schedule") {
                    let at = Int64(startDate.timeIntervalSince1970)
                    let end = hasEnd ? Int64(endDate.timeIntervalSince1970) : nil
                    author(projectSetItemSchedule(itemId: itemId, at: at, endDate: end))
                }
            }
        }
    }

    private var assigneeSection: some View {
        Section("Assignees") {
            if members.isEmpty {
                Text("Add people to the project to assign work.").foregroundStyle(.secondary)
            }
            ForEach(members, id: \.self) { m in
                Button {
                    let assigned = !(item?.assignees.contains(m) ?? false)
                    author(projectSetAssignee(itemId: itemId, member: m, assigned: assigned))
                } label: {
                    HStack {
                        Text(m == core.myIdentityHex ? "You" : String(m.prefix(8)))
                            .font(.system(.subheadline, design: m == core.myIdentityHex ? .default : .monospaced))
                        Spacer()
                        if item?.assignees.contains(m) == true {
                            Image(systemName: "checkmark").foregroundStyle(Brand.accent)
                        }
                    }
                }
                .tint(.primary)
            }
        }
    }

    private var incomingDeps: [DependencyRow] { deps.filter { $0.to == itemId } }

    private var blockerCandidates: [ProjectItemRow] {
        let taken = Set(incomingDeps.map { $0.from })
        return allItems.filter { $0.id != itemId && !taken.contains($0.id) }
    }

    private var blockedBySection: some View {
        Section("Blocked by") {
            ForEach(incomingDeps, id: \.edgeId) { d in
                HStack {
                    Text(title(of: d.from)).font(.subheadline)
                    Spacer()
                    Button(role: .destructive) { author(projectRemoveDependency(edgeId: d.edgeId)) } label: {
                        Image(systemName: "minus.circle.fill")
                    }
                    .buttonStyle(.plain).foregroundStyle(.red)
                }
            }
            if !blockerCandidates.isEmpty {
                Picker("Add a blocker", selection: $blockerId) {
                    Text("Choose…").tag("")
                    ForEach(blockerCandidates, id: \.id) { it in Text(it.title).tag(it.id) }
                }
                if !blockerId.isEmpty {
                    Button("Block until that's done") {
                        author(projectAddDependency(edgeId: UUID().uuidString, from: blockerId, to: itemId, kind: "blocks"))
                        blockerId = ""
                    }
                }
            }
        }
    }

    private var fieldsSection: some View {
        Section("Fields") {
            if let f = item?.fields, !f.isEmpty {
                ForEach(f.sorted(by: { $0.key < $1.key }), id: \.key) { k, v in
                    HStack { Text(k).foregroundStyle(.secondary); Spacer(); Text(v) }
                        .font(.subheadline)
                }
            }
            TextField("Field", text: $fieldKey)
            TextField("Value", text: $fieldValue)
            Button("Set field") {
                author(projectSetItemField(itemId: itemId, field: fieldKey, value: fieldValue, gen: 0))
                fieldKey = ""; fieldValue = ""
            }
            .disabled(fieldKey.isEmpty)
        }
    }

    // MARK: helpers

    private func title(of id: String) -> String {
        allItems.first { $0.id == id }?.title ?? (String(id.prefix(6)) + "…")
    }

    private func reload() {
        allItems = core.projectItems(objectId)
        item = allItems.first { $0.id == itemId }
        members = core.objectMembers(objectId)
        deps = core.projectDependencies(objectId)
        if let at = item?.at {
            scheduled = true
            startDate = Date(timeIntervalSince1970: TimeInterval(at))
        }
        if let end = item?.endDate {
            hasEnd = true
            endDate = Date(timeIntervalSince1970: TimeInterval(end))
        }
    }

    private func author(_ draft: DeltaDraft) {
        let oid = objectId
        Task { @MainActor in
            _ = await core.projectAuthor(oid, draft)
            reload()
            onChange()
        }
    }
}
