//  Outline.swift
//  The Notes tab's OUTLINE segment (relocated from the old Work tab): nested
//  outline + flat-search rows over the entity graph. Long-press any row to EDIT;
//  tactical "+" buttons add people/actions. Everything mutates the observable
//  OutlineModel, so refining the (deliberately rough) inferences updates live.

import SwiftUI

private let store = OutlineModel.shared

// MARK: - Nested outline (All + no search)

struct OutlineList: View {
    var present: (OutlineSheet) -> Void
    @ObservedObject private var model = OutlineModel.shared
    @State private var collapsed: Set<String> = []

    var body: some View {
        LazyVStack(spacing: 12) {
            ForEach(model.topLevel()) { e in
                SpaceSection(entity: e, collapsed: $collapsed, present: present)
            }
        }
        .padding(.horizontal)
        .padding(.bottom)
    }
}

struct SpaceSection: View {
    let entity: EntityVM
    @Binding var collapsed: Set<String>
    var present: (OutlineSheet) -> Void

    private var isOpen: Bool { !collapsed.contains(entity.id) }
    private var kids: [EntityVM] { store.children(of: entity.id) }
    private var people: [EntityVM] { kids.filter { $0.kind == .person } }
    private var spaces: [EntityVM] { kids.filter { $0.kind != .person } }
    private var directActions: [EntityVM] { store.actions(of: entity.id) }
    private var hasContent: Bool { !directActions.isEmpty || !kids.isEmpty }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Button {
                    if isOpen { collapsed.insert(entity.id) } else { collapsed.remove(entity.id) }
                } label: {
                    Image(systemName: isOpen ? "chevron.down" : "chevron.right")
                        .font(.caption).foregroundStyle(.secondary).frame(width: 16)
                }
                .buttonStyle(.plain)

                NavigationLink(value: entity) {
                    HStack(spacing: 10) {
                        LogoBadge(initials: entity.initials, tint: entity.tint, size: 34)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(entity.name).font(.headline)
                            Label(entity.descriptor ?? entity.kind.word, systemImage: entity.kind.glyph)
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                    }
                }
                .buttonStyle(.plain)

                Menu {
                    Button { present(.addAction(entity)) } label: { Label("Add action", systemImage: "checkmark.circle") }
                    Button { present(.addChild(entity)) } label: { Label("Add person / place", systemImage: "person.badge.plus") }
                    Button { present(.edit(entity)) } label: { Label("Edit", systemImage: "pencil") }
                } label: {
                    Image(systemName: "plus.circle").font(.body).foregroundStyle(Brand.accent)
                }
            }
            .padding()
            .contextMenu { editMenu(entity, present: present) }

            if isOpen && hasContent {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(directActions) { ActionRow(action: $0, present: present) }
                    ForEach(people) { PersonRow(person: $0, present: present) }
                    ForEach(spaces) { SpaceRow(entity: $0, present: present) }
                }
                .padding(12)
            }
        }
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

// MARK: - Rows

struct PersonRow: View {
    let person: EntityVM
    var present: (OutlineSheet) -> Void = { _ in }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            NavigationLink(value: person) {
                HStack(spacing: 10) {
                    LogoBadge(initials: person.initials, tint: person.tint, size: 30)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(person.name).font(.subheadline.weight(.medium))
                        if let a = person.attributes.first {
                            Text(a).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 4)
                    if person.waiting && store.actions(of: person.id).isEmpty { WaitingPill() }
                    if let av = person.availability { TimeChipView(chip: av) }
                    Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)
            .contextMenu {
                Button { present(.edit(person)) } label: { Label("Edit", systemImage: "pencil") }
                Button { present(.addAction(person)) } label: { Label("Add action", systemImage: "checkmark.circle") }
            }
            ForEach(store.actions(of: person.id)) { ActionRow(action: $0, present: present).padding(.leading, 40) }
        }
    }
}

struct SpaceRow: View {
    let entity: EntityVM
    var present: (OutlineSheet) -> Void = { _ in }
    var body: some View {
        NavigationLink(value: entity) {
            HStack(spacing: 10) {
                LogoBadge(initials: entity.initials, tint: entity.tint, size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(entity.name).font(.subheadline.weight(.medium))
                    Label(entity.descriptor ?? entity.kind.word, systemImage: entity.kind.glyph)
                        .font(.caption2).foregroundStyle(.secondary)
                }
                Spacer()
                if let w = entity.window { WindowChip(window: w) }
                else {
                    let n = entity.childIDs.count + entity.actionIDs.count
                    if n > 0 { Text("\(n)").font(.caption2).foregroundStyle(.secondary) }
                }
                Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
            }
        }
        .buttonStyle(.plain)
        .contextMenu { editMenu(entity, present: present) }
    }
}

struct ActionRow: View {
    let action: EntityVM
    var present: (OutlineSheet) -> Void = { _ in }
    private var waiting: Bool { action.actionWaiting }
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            NavigationLink(value: action) {
                HStack(spacing: 8) {
                    Image(systemName: waiting ? "hourglass" : "circle")
                        .font(.footnote).foregroundStyle(waiting ? Color.orange : .secondary)
                    Text(action.name)
                        .font(.subheadline).foregroundStyle(waiting ? .secondary : .primary)
                    if waiting { WaitingPill() }
                    Spacer(minLength: 4)
                    if let d = action.due { TimeChipView(chip: d) }
                    Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)
            .contextMenu {
                Button { present(.edit(action)) } label: { Label("Edit", systemImage: "pencil") }
            }
            if !action.linkIDs.isEmpty {
                WrapLayout(spacing: 6) {
                    ForEach(action.linkIDs, id: \.self) { EntityChip(id: $0) }
                }
                .padding(.leading, 26)
            }
        }
    }
}

@ViewBuilder
func editMenu(_ entity: EntityVM, present: @escaping (OutlineSheet) -> Void) -> some View {
    Button { present(.edit(entity)) } label: { Label("Edit", systemImage: "pencil") }
    if entity.kind != .event {
        Button { present(.addAction(entity)) } label: { Label("Add action", systemImage: "checkmark.circle") }
    }
    if entity.kind == .org || entity.kind == .place {
        Button { present(.addChild(entity)) } label: { Label("Add person / place", systemImage: "person.badge.plus") }
    }
}

// MARK: - Flat search / type-filter results

struct FlatResults: View {
    let query: String
    let filter: String
    var present: (OutlineSheet) -> Void
    @ObservedObject private var model = OutlineModel.shared
    private var results: [EntityVM] { model.search(query, filter: filter) }

    var body: some View {
        LazyVStack(spacing: 0) {
            if results.isEmpty {
                ContentUnavailableView("No matches", systemImage: "magnifyingglass")
                    .padding(.top, 40)
            } else {
                VStack(spacing: 0) {
                    ForEach(results) { e in
                        NavigationLink(value: e) { FlatRow(entity: e) }
                            .buttonStyle(.plain)
                            .contextMenu { editMenu(e, present: present) }
                    }
                }
                .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                .padding(.horizontal)
            }
        }
        .padding(.bottom)
    }
}

struct FlatRow: View {
    let entity: EntityVM
    var body: some View {
        HStack(spacing: 12) {
            LogoBadge(initials: entity.initials, tint: entity.tint, size: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(entity.name).font(.subheadline.weight(.medium))
                Label(entity.descriptor ?? entity.attributes.first ?? entity.kind.word,
                      systemImage: entity.kind.glyph)
                    .font(.caption2).foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            if entity.actionWaiting || entity.waiting { WaitingPill() }
            if let w = entity.window { WindowChip(window: w) }
            else if let d = entity.due { TimeChipView(chip: d) }
            Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
        }
        .padding(.vertical, 10).padding(.horizontal, 12)
    }
}

// MARK: - Chips

struct WaitingPill: View {
    var body: some View {
        Label("waiting", systemImage: "hourglass")
            .font(.system(size: 9, weight: .semibold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Capsule().fill(Color.orange.opacity(0.16)))
            .foregroundStyle(.orange)
    }
}

struct TimeChipView: View {
    let chip: TimeChip
    var body: some View {
        Text(chip.label)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Capsule().fill(chip.tint.opacity(0.16)))
            .foregroundStyle(chip.tint)
    }
}

struct WindowChip: View {
    let window: EventWindow
    private var tint: Color { TimeChip(label: "", urgency: window.urgency).tint }
    var body: some View {
        Label(window.display, systemImage: "calendar")
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Capsule().fill(tint.opacity(0.16)))
            .foregroundStyle(tint)
    }
}

struct EntityChip: View {
    let id: String
    var body: some View {
        if let e = store.entity(id) {
            NavigationLink(value: e) {
                HStack(spacing: 3) {
                    Image(systemName: e.unminted ? "plus.circle" : e.kind.glyph).font(.system(size: 9))
                    Text(e.name)
                }
                .font(.caption2.weight(.medium))
                .padding(.horizontal, 7).padding(.vertical, 3)
                .background {
                    if e.unminted {
                        Capsule().strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [3]))
                            .foregroundStyle(.secondary)
                    } else {
                        Capsule().fill(Brand.accent.opacity(0.14))
                    }
                }
                .foregroundStyle(e.unminted ? Color.secondary : Brand.accent)
            }
            .buttonStyle(.plain)
        }
    }
}
