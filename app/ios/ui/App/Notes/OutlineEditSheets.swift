//  OutlineEditSheets.swift
//  Edit / add sheets for the Notes tab's outline — the refine-and-enrich loop.
//  Pacific's first pass is rough on purpose; correcting it here is how the user
//  makes the model match (and understand) their actual world. All changes mutate
//  the observable OutlineModel.

import SwiftUI

/// What sheet to present from a long-press / + affordance.
enum OutlineSheet: Identifiable {
    case edit(EntityVM), addAction(EntityVM), addChild(EntityVM), addTopLevel
    var id: String {
        switch self {
        case .edit(let e):      return "edit-\(e.id)"
        case .addAction(let e): return "act-\(e.id)"
        case .addChild(let e):  return "child-\(e.id)"
        case .addTopLevel:      return "top"
        }
    }
}

@ViewBuilder
func outlineSheet(_ s: OutlineSheet) -> some View {
    switch s {
    case .edit(let e):      EntityEditSheet(entity: e)
    case .addAction(let e): AddActionSheet(ownerID: e.id, ownerName: e.name)
    case .addChild(let e):  AddEntitySheet(containerID: e.id, containerName: e.name)
    case .addTopLevel:      AddEntitySheet(containerID: nil, containerName: nil)
    }
}

struct EntityEditSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var model = OutlineModel.shared
    let original: EntityVM

    @State private var name: String
    @State private var kind: EntityKind
    @State private var descriptor: String
    @State private var attributes: String
    @State private var notes: String
    @State private var waiting: Bool

    init(entity: EntityVM) {
        original = entity
        _name = State(initialValue: entity.name)
        _kind = State(initialValue: entity.kind)
        _descriptor = State(initialValue: entity.descriptor ?? "")
        _attributes = State(initialValue: entity.attributes.joined(separator: ", "))
        _notes = State(initialValue: entity.notes ?? "")
        _waiting = State(initialValue: entity.waiting)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Name") { TextField("Name", text: $name) }
                Section("Type") {
                    Picker("Type", selection: $kind) {
                        ForEach(EntityKind.allCases, id: \.self) { k in
                            Label(k.word, systemImage: k.glyph).tag(k)
                        }
                    }
                }
                Section("Descriptor") { TextField("e.g. a tailor", text: $descriptor) }
                Section("Attributes") {
                    TextField("comma, separated", text: $attributes, axis: .vertical).lineLimit(1...3)
                }
                Section("Notes") {
                    TextField("Notes", text: $notes, axis: .vertical).lineLimit(2...6)
                }
                if original.kind == .person {
                    Section { Toggle("Waiting on them (~)", isOn: $waiting) }
                }
                Section {
                    Label("Your work lives on this device.", systemImage: "lock.fill")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Edit")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) { Button("Save") { save() }.fontWeight(.semibold) }
            }
        }
    }

    private func save() {
        var e = original
        e.name = name
        e.kind = kind
        e.descriptor = descriptor.isEmpty ? nil : descriptor
        e.attributes = attributes.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        e.notes = notes.isEmpty ? nil : notes
        e.waiting = waiting
        model.save(e)
        dismiss()
    }
}

struct AddActionSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var model = OutlineModel.shared
    let ownerID: String
    let ownerName: String

    @State private var verb = ""
    @State private var waiting = false
    @State private var due = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Action") { TextField("What needs doing?", text: $verb) }
                Section {
                    Toggle("Waiting on \(ownerName) (~)", isOn: $waiting)
                } footer: {
                    Text(waiting ? "The ball is in their court." : "Your move.")
                }
                Section("When (optional)") { TextField("e.g. Fri · ASAP · 3rd Jul", text: $due) }
            }
            .navigationTitle("New action")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Add") {
                        model.addAction(to: ownerID, verb: verb, waiting: waiting,
                                        due: due.isEmpty ? nil : TimeChip(label: due, urgency: .soon))
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    .disabled(verb.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
    }
}

struct AddEntitySheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var model = OutlineModel.shared
    let containerID: String?
    let containerName: String?

    @State private var name = ""
    @State private var kind: EntityKind = .person

    var body: some View {
        NavigationStack {
            Form {
                Section(containerName.map { "Add to \($0)" } ?? "New space") {
                    TextField("Name", text: $name)
                }
                Section("Type") {
                    Picker("Type", selection: $kind) {
                        ForEach([EntityKind.person, .org, .place, .event], id: \.self) { k in
                            Label(k.word, systemImage: k.glyph).tag(k)
                        }
                    }
                    .pickerStyle(.segmented)
                }
            }
            .navigationTitle(containerName == nil ? "New space" : "Add")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Add") {
                        model.addEntity(to: containerID, name: name, kind: kind)
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
    }
}
