//  SystemService.swift
//  A System GroupObject, scoped to a shared credential. Because each Aries
//  employee has their own Sysiphus login, each app instance holds ONE System —
//  the signed-in employee's (a group-of-1). Signing in pulls that credential's
//  Fields from Sysiphus and hands them to the resolver to index. The token lives
//  in the Keychain (at-rest protection — never the plaintext pacific.db path).

import SwiftUI

@MainActor
final class SystemService: ObservableObject {
    enum Status: Equatable { case signedOut, connecting, connected(Int) }

    @Published private(set) var employee: Employee?      // the credential this System is scoped to
    @Published private(set) var fields: [SysiphusField] = []
    @Published private(set) var status: Status = .signedOut
    @Published var lastError: String?

    private let client = SysiphusClient()
    private let resolution: ResolutionService
    private var token: String?
    private var handle: CredentialHandle?

    private static let handleKey = "sysiphus.credentialHandle"
    private static let employeeKey = "sysiphus.employee"

    init(resolution: ResolutionService) {
        self.resolution = resolution
        restoreSession()
    }

    var isSignedIn: Bool { employee != nil }

    func signIn(username: String, password: String) async {
        status = .connecting
        lastError = nil
        do {
            let result = try await client.login(username: username, password: password)
            token = result.token
            employee = result.employee

            // The credential goes into the device vault keyed by a content-free handle
            // (the Group `storeCredential` shape); the secret never leaves the Keychain.
            let h = CredentialHandle.mint(kind: .bearerToken, label: "Sysiphus · \(username)")
            CredentialVault.store(result.token, for: h)
            handle = h
            persistHandle(h)
            persistEmployee(result.employee)

            await refresh()
        } catch {
            status = .signedOut
            lastError = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
        }
    }

    /// Re-pull this credential's Fields from Sysiphus and re-index.
    func refresh() async {
        guard let token else { status = .signedOut; return }
        status = .connecting
        do {
            let pulled = try await client.designs(token: token)
            fields = pulled
            resolution.index(pulled)
            status = .connected(pulled.count)
            lastError = nil
        } catch SysiphusHTTPError.unauthorized {
            signOut()
            lastError = "Session expired — sign in again."
        } catch {
            status = .signedOut
            lastError = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
        }
    }

    func signOut() {
        token = nil
        employee = nil
        fields = []
        resolution.clear()
        status = .signedOut
        if let h = handle { CredentialVault.remove(h) }
        handle = nil
        UserDefaults.standard.removeObject(forKey: Self.handleKey)
        UserDefaults.standard.removeObject(forKey: Self.employeeKey)
    }

    /// LOOKUP live tier — re-read one design from source (Sysiphus) with THIS credential,
    /// so the panel shows current stock / open orders. Cached tier is ResolutionService.field.
    func lookupLive(_ id: String) async -> SysiphusField? {
        guard let token else { return nil }
        return try? await client.design(token: token, id: id)
    }

    private func restoreSession() {
        guard let h = loadHandle(),
              let emp = loadEmployee(),
              let t = CredentialVault.secret(for: h) else { return }
        handle = h
        token = t
        employee = emp
        Task { await refresh() }
    }

    // MARK: - Persistence of the content-free handle + employee (the secret is in the vault)

    private func persistHandle(_ h: CredentialHandle) {
        if let data = try? JSONEncoder().encode(h) {
            UserDefaults.standard.set(data, forKey: Self.handleKey)
        }
    }

    private func loadHandle() -> CredentialHandle? {
        guard let data = UserDefaults.standard.data(forKey: Self.handleKey) else { return nil }
        return try? JSONDecoder().decode(CredentialHandle.self, from: data)
    }

    private func persistEmployee(_ e: Employee) {
        if let data = try? JSONEncoder().encode(e) {
            UserDefaults.standard.set(data, forKey: Self.employeeKey)
        }
    }

    private func loadEmployee() -> Employee? {
        guard let data = UserDefaults.standard.data(forKey: Self.employeeKey) else { return nil }
        return try? JSONDecoder().decode(Employee.self, from: data)
    }
}
