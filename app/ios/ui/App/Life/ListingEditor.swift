//  ListingEditor.swift
//  Putting a Thing on the market — the one surface where a posture, a price, an audience
//  and an area are decided together.
//
//  They belong together because they are ONE act with one consequence. A price is meaningless
//  without knowing who will see it; an area is meaningless without knowing how far it travels.
//  Splitting them across screens would let someone set a price for friends and later widen the
//  audience without re-reading the price they set for a different one.
//
//  THE TWO CONTROLS THAT ARE NOT ORDINARY FORM FIELDS:
//
//  REACH is consent, and it is also identity disclosure — a listing that reaches the network
//  carries the origin signature, so "anyone" means "my identity travels with this". It is
//  stated in those terms, and it defaults to the closed option: an over-share cannot be taken
//  back once a copy is on someone else's device.
//
//  AREA is coarse and TRUNCATED, never jittered. Truncation is idempotent — the same cell for
//  the same home every time — so a seller's history cannot be averaged to a point. It is a
//  DECLARED home, set once, not a live fix, because deriving it per listing would make a
//  sequence of listings a movement trace.

import SwiftUI
import PhotosUI
import PacificCore

struct ListingEditor: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var location: LocationService
    @Environment(\.dismiss) private var dismiss

    /// The thing being listed. nil mints a new one.
    let existing: ThingRow?
    /// The market-face axis (artifact | skill | job). An existing row's own category
    /// wins; this seeds a NEW listing (the group flows preset it per face).
    let category: String
    /// Fired after a successful save with (objectId, title) — the seam the group page
    /// uses to attach the listing as the group's own creation.
    let onSaved: ((String, String) -> Void)?

    @State private var title = ""
    @State private var descriptor = ""
    @State private var posture: MarketIntent = .selling
    @State private var price = ""
    @State private var hasDeadline = false
    @State private var deadline = Date().addingTimeInterval(7 * 86_400)
    @State private var reachNetwork = false
    @State private var area = ""
    @State private var precision = 4
    /// The listing's face. `photoData` is the JPEG bytes shown in the slot (the
    /// existing photo on open); `photoDirty` says the save must author a
    /// `thing.setPhoto` — set on pick AND on clear, so taking a picture down is
    /// a real edit, not a skipped write.
    @State private var photoItem: PhotosPickerItem?
    @State private var photoData: Data?
    @State private var photoDirty = false

    init(_ existing: ThingRow? = nil,
         category: String = "artifact",
         posture seedPosture: MarketIntent? = nil,
         onSaved: ((String, String) -> Void)? = nil) {
        self.existing = existing
        self.category = existing?.category ?? category
        self.onSaved = onSaved
        _title = State(initialValue: existing?.name ?? "")
        _descriptor = State(initialValue: existing?.descriptor ?? "")
        _posture = State(initialValue: existing.flatMap { MarketIntent(rawValue: $0.posture.uppercased()) }
                         ?? seedPosture ?? .selling)
        _price = State(initialValue: existing?.price ?? "")
        _hasDeadline = State(initialValue: (existing?.deadline ?? 0) > 0)
        _deadline = State(initialValue: (existing?.deadline ?? 0) > 0
                          ? Date(timeIntervalSince1970: Double(existing!.deadline) / 1000)
                          : Date().addingTimeInterval(7 * 86_400))
        _reachNetwork = State(initialValue: existing?.reach == "network")
        _area = State(initialValue: existing?.area ?? "")
        _precision = State(initialValue: max(3, existing?.area.count ?? 4))
        _photoData = State(initialValue: existing.flatMap {
            $0.photo.isEmpty ? nil : Data(base64Encoded: $0.photo)
        })
    }

    /// Only an ACTIVE posture takes a price — core rejects a price on a standing intent, so
    /// the field is absent rather than present-and-refused.
    private var takesPrice: Bool { posture.isActive }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Title", text: $title)
                    TextField("Description", text: $descriptor, axis: .vertical).lineLimit(1...3)
                }

                Section("Photo") {
                    if let photoData, let ui = UIImage(data: photoData) {
                        HStack(spacing: 14) {
                            Image(uiImage: ui)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 96, height: 96)
                                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                            Spacer()
                            VStack(alignment: .trailing, spacing: 10) {
                                PhotosPicker(selection: $photoItem, matching: .images) {
                                    Text("Replace")
                                }
                                .buttonStyle(.borderless)
                                Button("Remove", role: .destructive) {
                                    self.photoData = nil
                                    photoDirty = true
                                }
                                .buttonStyle(.borderless)
                            }
                        }
                    } else {
                        PhotosPicker(selection: $photoItem, matching: .images) {
                            Label("Add photo", systemImage: "photo")
                        }
                        .accessibilityIdentifier("listing-add-photo")
                    }
                }

                Section("Posture") {
                    Picker("Posture", selection: $posture) {
                        ForEach(MarketIntent.ordered) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.menu)
                    if takesPrice {
                        TextField("Price", text: $price)
                    }
                }

                Section("Until") {
                    Toggle("Set an end date", isOn: $hasDeadline.animation())
                    if hasDeadline {
                        DatePicker("Ends", selection: $deadline, displayedComponents: .date)
                    }
                }

                Section("Audience") {
                    Picker("Seen by", selection: $reachNetwork) {
                        Text("My connections").tag(false)
                        Text("The network").tag(true)
                    }
                    .pickerStyle(.segmented)
                    // The ONE place a sentence is warranted: this control gives away identity,
                    // and a control whose consequence is invisible is not consent.
                    if reachNetwork {
                        Text("Your connections can pass this on. Your name travels with it.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }

                Section("Area") {
                    Picker("Precision", selection: $precision) {
                        Text("Region").tag(3)
                        Text("Metro").tag(4)
                        Text("District").tag(5)
                    }
                    .pickerStyle(.segmented)
                    HStack {
                        Text(area.isEmpty ? "Not set" : area)
                            .foregroundStyle(area.isEmpty ? .secondary : .primary)
                            .monospaced()
                        Spacer()
                        Button("Use my location") { setArea() }
                            .buttonStyle(.borderless)
                    }
                    if !area.isEmpty {
                        Button("Clear", role: .destructive) { area = "" }
                            .buttonStyle(.borderless)
                    }
                }
            }
            .navigationTitle(existing == nil ? "New listing" : "Listing")
            .navigationBarTitleDisplayMode(.inline)
            // The shared compose discs (Design/ComposeSurface.swift) — same buttons,
            // same press behaviour, as every other "+" on LIFE.
            .safeAreaInset(edge: .bottom) {
                ComposeDiscs(canCommit: !title.trimmingCharacters(in: .whitespaces).isEmpty,
                             commitLabel: "Save listing",
                             cancel: { dismiss() },
                             commit: { save() })
                    .background(.bar)
            }
            .onChange(of: precision) { _, _ in if !area.isEmpty { setArea() } }
            .onChange(of: posture) { _, p in if !p.isActive { price = "" } }
            .onChange(of: photoItem) { _, item in
                guard let item else { return }
                photoItem = nil
                Task {
                    guard let data = try? await item.loadTransferable(type: Data.self) else { return }
                    photoData = data   // fitted under the cap at save (jpegUnderCap)
                    photoDirty = true
                }
            }
        }
    }

    /// Truncate the CURRENT fix once, to the chosen precision, and keep the string. The live
    /// coordinate is never stored and never republished — what is kept is a cell, not a place.
    private func setArea() {
        guard let here = location.coordinate else { return }
        area = core.coarseArea(lat: here.latitude, lon: here.longitude, precision: precision)
    }

    private func save() {
        let name = title.trimmingCharacters(in: .whitespaces)
        let id = existing?.id ?? core.mintThing(name: name, descriptor: descriptor,
                                                category: category)
        guard let id else { return }
        if existing != nil {
            core.renameThing(id, name: name, descriptor: descriptor, category: category)
        }
        core.setPosture(id,
                        posture: posture.rawValue.lowercased(),
                        price: takesPrice ? price : "",
                        deadline: hasDeadline ? Int64(deadline.timeIntervalSince1970 * 1000) : 0,
                        reach: reachNetwork ? "network" : "private",
                        area: area)
        let photo = photoData
        let photoChanged = photoDirty
        Task {
            // The photo delta must LAND before the fan-out, or the first publish
            // goes photoless and the face only follows on the next sync tick.
            if photoChanged { _ = await core.setThingPhoto(id, jpeg: photo) }
            // Fan it out NOW rather than on the next sync tick: listing something and
            // watching nothing happen for three seconds reads as a failure. The sync
            // loop still runs the same pass, so this is a head start, not a second
            // code path.
            core.publishMarket()
        }
        onSaved?(id, name)
        dismiss()
    }
}
