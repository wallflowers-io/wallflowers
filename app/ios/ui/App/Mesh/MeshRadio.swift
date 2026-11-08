//  MeshRadio.swift
//  The Bluetooth half of the device-to-device mesh — and ONLY the Bluetooth half.
//
//  Everything that decides anything lives in Rust (`pacific-core/src/mesh*.rs`): framing,
//  reassembly, dedupe, what to carry, what to evict, what to say next, who to dial, when to scan.
//  This file moves bytes between Core Bluetooth and a `MeshLink`, and asks the policy before it
//  acts. That split is deliberate — mesh logic tested against a real radio is barely tested at
//  all, and every rule worth getting right is one that can be proven in `cargo test` instead.
//
//  WHY BLUETOOTH AND NOT PEER-TO-PEER WI-FI. MultipeerConnectivity and Network.framework's AWDL
//  are far faster, and both stop when the app is backgrounded — which would make this a link
//  between two open apps rather than a mesh. Core Bluetooth is the only iOS radio that keeps
//  running in the background, and background is the entire proposition: messages hopping through
//  pockets, between people who never open the app at the same moment.
//
//  THE BACKGROUND RULES (iOS 26.5 SDK, CBCentralManager.h / CBPeripheralManager.h):
//   • Scanning backgrounded needs the `bluetooth-central` mode AND an explicit service UUID —
//     a nil scan finds nothing. `AllowDuplicates` is ignored, so we get one callback per device
//     per scan session and must not depend on repeated discoveries.
//   • Advertising backgrounded needs the `bluetooth-peripheral` mode. The local name is dropped
//     and every service UUID moves to the "overflow" area, which "can only be discovered by an
//     iOS device that is explicitly scanning for them."
//  Those two compose into the property the mesh depends on: two BACKGROUNDED phones still find
//  each other, precisely because both sides know `Self.service` a priori. It is also why that
//  UUID is a hard-coded constant and not something negotiated or configured.
//
//  PACING IS NOT OPTIONAL. Writes are spaced by `meshWriteSpacingMs`, not fired as fast as
//  `canSendWriteWithoutResponse` allows. iOS's own back-pressure signal is not sufficient in the
//  field (bitchat: "Aggressive pacing causes packet loss; needs 25-30ms between fragments"), and a
//  dropped chunk here does not merely lose a message — it desynchronises a length-prefixed stream
//  and poisons the link.
//
//  Every device runs BOTH roles. Two phones that meet will each try to connect; whichever central
//  wins carries the exchange, and the duplicate link falls away when one side drops.

import CoreBluetooth
import Foundation
import PacificCore

@MainActor
final class MeshRadio: NSObject, ObservableObject {
    /// The Pacific mesh GATT service. Fixed and public: a backgrounded advertiser's UUIDs are in
    /// the overflow area, reachable only by a scanner that already knows what to ask for.
    static let service = CBUUID(string: "7ACF0D1E-3B41-4E6A-9E2C-2A61F0C7D5B1")
    /// One characteristic carries both directions: centrals write to it, the peripheral notifies
    /// on it. A single pipe keeps ordering trivially correct in both directions.
    static let inbox = CBUUID(string: "7ACF0D1F-3B41-4E6A-9E2C-2A61F0C7D5B1")

    /// Blobs this device is carrying right now — its own and other people's.
    @Published private(set) var carried: UInt64 = 0
    /// Peers currently in range and exchanging.
    @Published private(set) var peers: Int = 0
    @Published private(set) var poweredOn = false

    private var central: CBCentralManager?
    private var peripheral: CBPeripheralManager?
    private var characteristic: CBMutableCharacteristic?

    /// Every scheduling decision — dial or skip, how many links, how weak is too weak, when to
    /// hang up, when to scan — is made in Rust (`mesh_policy`). This class asks and obeys.
    private let policy = MeshPolicyHandle()
    private let spacing = TimeInterval(meshWriteSpacingMs()) / 1000
    private var tick: Timer?
    private var scanning = false

    /// One conversation per peer. Reassembly is per-peer: two peers chunk independently, so a
    /// shared buffer would interleave their bytes and corrupt both streams.
    private var pipes: [UUID: PeerPipe] = [:]
    private var connected: [UUID: CBPeripheral] = [:]
    private var subscribers: [UUID: CBCentral] = [:]

    func start() {
        guard central == nil else { return }
        // The restore identifiers are what let iOS relaunch us into the background when a peer
        // turns up — without them the mesh only runs while someone is looking at the app.
        central = CBCentralManager(
            delegate: self, queue: nil,
            options: [CBCentralManagerOptionRestoreIdentifierKey: "network.pacific.mesh.central"])
        peripheral = CBPeripheralManager(
            delegate: self, queue: nil,
            options: [CBPeripheralManagerOptionRestoreIdentifierKey: "network.pacific.mesh.peripheral"])
    }

    func stop() {
        tick?.invalidate()
        tick = nil
        central?.stopScan()
        scanning = false
        peripheral?.stopAdvertising()
        connected.values.forEach { central?.cancelPeripheralConnection($0) }
        connected.removeAll()
        subscribers.removeAll()
        pipes.removeAll()
        peers = 0
    }

    // MARK: - Heartbeat

    /// One timer, so the wake-ups are counted and bounded: hang up finished conversations, and
    /// open or close the scan window. A second, because the duty cycle turns over in five.
    private func startTick() {
        guard tick == nil else { return }
        tick = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.onTick() }
        }
    }

    private func onTick() {
        for peer in policy.hangups() {
            if let id = UUID(uuidString: peer) { close(id, .completed) }
        }
        applyScanPolicy()
    }

    /// Start or stop scanning to match the policy — duty-cycled in every state, including the
    /// background, which is the mode that runs all day.
    private func applyScanPolicy() {
        guard let central, central.state == .poweredOn else { return }
        let want = policy.shouldScan()
        guard want != scanning else { return }
        scanning = want
        if want {
            // The explicit service UUID is mandatory to scan while backgrounded, and is also what
            // makes a backgrounded peer's overflow advertisement visible at all.
            central.scanForPeripherals(withServices: [Self.service], options: nil)
        } else {
            central.stopScan()
        }
    }

    // MARK: - Byte plumbing

    /// Hand received bytes to the peer's link and queue whatever it answers.
    private func pump(_ id: UUID, _ data: Data) {
        guard let pipe = pipes[id] else { return }
        policy.activity(peer: id.uuidString)
        do {
            let reply = try pipe.link.receive(bytes: data)
            if pipe.link.isPoisoned() { close(id, .dropped); return }
            if reply.isEmpty {
                // Nothing left to say. The policy starts a short linger rather than hanging up
                // now — the peer may still be mid-delivery, and truncating that would cost the
                // whole exchange to save two seconds of radio.
                policy.quiet(peer: id.uuidString)
            } else {
                enqueue(id, reply)
            }
        } catch {
            close(id, .dropped)
        }
        carried = meshCarried()
    }

    private func enqueue(_ id: UUID, _ payload: Data) {
        guard let pipe = pipes[id] else { return }
        pipe.queue.append(contentsOf: meshChunk(bytes: payload, mtu: UInt32(max(mtu(for: id), 1))))
        drain(id)
    }

    /// Write ONE chunk, then come back after `spacing`. Deliberately not a tight loop gated on
    /// `canSendWriteWithoutResponse`: that is exactly the aggressive pacing that drops chunks in
    /// the field, and a dropped chunk poisons a length-prefixed stream rather than merely losing
    /// a message.
    private func drain(_ id: UUID) {
        guard let pipe = pipes[id] else { return }
        guard !pipe.draining else { return }
        pipe.draining = true
        step(id)
    }

    private func step(_ id: UUID) {
        guard let pipe = pipes[id] else { return }
        guard let next = pipe.queue.first else { pipe.draining = false; return }
        if write(next, to: id) {
            pipe.queue.removeFirst()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + spacing) { [weak self] in
            Task { @MainActor in self?.step(id) }
        }
    }

    /// Push one chunk to a peer, whichever role we hold with them. Returns whether it went.
    private func write(_ chunk: Data, to id: UUID) -> Bool {
        if let p = connected[id] {
            guard let ch = inboxCharacteristic(of: p), p.canSendWriteWithoutResponse else { return false }
            p.writeValue(chunk, for: ch, type: .withoutResponse)
            return true
        }
        if let c = subscribers[id], let ch = characteristic, let manager = peripheral {
            return manager.updateValue(chunk, for: ch, onSubscribedCentrals: [c])
        }
        return false
    }

    private func mtu(for id: UUID) -> Int {
        if let p = connected[id] { return p.maximumWriteValueLength(for: .withoutResponse) }
        if let c = subscribers[id] { return c.maximumUpdateValueLength }
        return 20
    }

    private func inboxCharacteristic(of p: CBPeripheral) -> CBCharacteristic? {
        p.services?.first { $0.uuid == Self.service }?
            .characteristics?.first { $0.uuid == Self.inbox }
    }

    /// End a conversation, telling the policy WHY. The distinction decides how long before this
    /// peer is worth dialling again: one that finished waits a while, one that walked out of range
    /// is welcomed straight back, one that never answered backs off progressively.
    private func close(_ id: UUID, _ reason: CloseReason) {
        pipes[id] = nil
        subscribers[id] = nil
        if let p = connected.removeValue(forKey: id) { central?.cancelPeripheralConnection(p) }
        policy.closed(peer: id.uuidString, reason: reason.rawValue)
        peers = pipes.count
    }

    private enum CloseReason: String {
        case completed, dropped, timeout
    }

    /// Begin a conversation with a peer we have just linked to.
    private func begin(_ id: UUID) {
        let pipe = PeerPipe()
        pipes[id] = pipe
        peers = pipes.count
        policy.opened(peer: id.uuidString)
        if let greeting = try? pipe.link.greeting() {
            enqueue(id, greeting)
        }
    }

    /// A peer's link plus its paced send queue.
    private final class PeerPipe {
        let link = MeshLink()
        var queue: [Data] = []
        var draining = false
    }
}

// MARK: - Central role (scan, connect, write)

extension MeshRadio: CBCentralManagerDelegate {
    nonisolated func centralManagerDidUpdateState(_ central: CBCentralManager) {
        Task { @MainActor in
            poweredOn = central.state == .poweredOn
            guard central.state == .poweredOn else { return }
            startTick()
            applyScanPolicy()
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, willRestoreState dict: [String: Any]) {
        // Relaunched into the background by a BLE event: re-adopt the peripherals iOS kept for us.
        let restored = dict[CBCentralManagerRestoredStatePeripheralsKey] as? [CBPeripheral] ?? []
        Task { @MainActor in
            for p in restored {
                p.delegate = self
                connected[p.identifier] = p
            }
            startTick()
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                                    advertisementData: [String: Any], rssi RSSI: NSNumber) {
        let rssi = Int16(clamping: RSSI.intValue)
        Task { @MainActor in
            guard connected[peripheral.identifier] == nil else { return }
            // Cooldown, signal floor, link cap and connect rate limit all live in the policy. A
            // peer too weak to answer costs more in a timed-out connect than the exchange would.
            guard policy.admit(peer: peripheral.identifier.uuidString, rssi: rssi) else { return }
            peripheral.delegate = self
            connected[peripheral.identifier] = peripheral
            central.connect(peripheral, options: nil)
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        Task { @MainActor in peripheral.discoverServices([Self.service]) }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral,
                                    error: Error?) {
        // They went away. Usually they come back — a brief rediscovery debounce, not a punishment.
        Task { @MainActor in close(peripheral.identifier, .dropped) }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral,
                                    error: Error?) {
        // We dialled and got nothing. This one earns a real backoff.
        Task { @MainActor in close(peripheral.identifier, .timeout) }
    }
}

extension MeshRadio: CBPeripheralDelegate {
    nonisolated func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        Task { @MainActor in
            guard let svc = peripheral.services?.first(where: { $0.uuid == Self.service }) else {
                close(peripheral.identifier, .timeout); return
            }
            peripheral.discoverCharacteristics([Self.inbox], for: svc)
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService,
                                error: Error?) {
        Task { @MainActor in
            guard let ch = service.characteristics?.first(where: { $0.uuid == Self.inbox }) else {
                close(peripheral.identifier, .timeout); return
            }
            peripheral.setNotifyValue(true, for: ch)   // their replies come back as notifications
            begin(peripheral.identifier)
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic,
                                error: Error?) {
        guard let data = characteristic.value else { return }
        Task { @MainActor in pump(peripheral.identifier, data) }
    }

    nonisolated func peripheralIsReady(toSendWriteWithoutResponse peripheral: CBPeripheral) {
        // The paced drain is already running on its own schedule; this only matters when a write
        // was refused and the queue is otherwise idle.
        Task { @MainActor in drain(peripheral.identifier) }
    }
}

// MARK: - Peripheral role (advertise, receive, notify)

extension MeshRadio: CBPeripheralManagerDelegate {
    nonisolated func peripheralManagerDidUpdateState(_ manager: CBPeripheralManager) {
        Task { @MainActor in
            guard manager.state == .poweredOn else { return }
            let ch = CBMutableCharacteristic(
                type: Self.inbox,
                properties: [.writeWithoutResponse, .notify],
                value: nil,
                permissions: [.writeable])
            let svc = CBMutableService(type: Self.service, primary: true)
            svc.characteristics = [ch]
            characteristic = ch
            manager.add(svc)
            // Backgrounded, the local name is dropped and this UUID moves to the overflow area —
            // still discoverable, but only by a central explicitly scanning for it. Which ours is.
            manager.startAdvertising([CBAdvertisementDataServiceUUIDsKey: [Self.service]])
        }
    }

    nonisolated func peripheralManager(_ manager: CBPeripheralManager, willRestoreState dict: [String: Any]) {
        Task { @MainActor in
            let services = dict[CBPeripheralManagerRestoredStateServicesKey] as? [CBMutableService] ?? []
            characteristic = services
                .compactMap { $0.characteristics?.first { $0.uuid == Self.inbox } as? CBMutableCharacteristic }
                .first
        }
    }

    nonisolated func peripheralManager(_ manager: CBPeripheralManager, central: CBCentral,
                                       didSubscribeTo characteristic: CBCharacteristic) {
        Task { @MainActor in
            subscribers[central.identifier] = central
            // The central opens with its greeting; ours goes back the same way, so both sides
            // learn what the other holds without either having to lead.
            begin(central.identifier)
        }
    }

    nonisolated func peripheralManager(_ manager: CBPeripheralManager, central: CBCentral,
                                       didUnsubscribeFrom characteristic: CBCharacteristic) {
        Task { @MainActor in close(central.identifier, .dropped) }
    }

    nonisolated func peripheralManager(_ manager: CBPeripheralManager,
                                       didReceiveWrite requests: [CBATTRequest]) {
        Task { @MainActor in
            for r in requests {
                guard let data = r.value else { continue }
                if subscribers[r.central.identifier] == nil { subscribers[r.central.identifier] = r.central }
                pump(r.central.identifier, data)
            }
        }
    }

    nonisolated func peripheralManagerIsReady(toUpdateSubscribers manager: CBPeripheralManager) {
        Task { @MainActor in
            for id in subscribers.keys { drain(id) }
        }
    }
}
