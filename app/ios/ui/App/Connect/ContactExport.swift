//  ContactExport.swift
//  The "useful route" made REAL: an Aries Arise card (yours, or someone you met) becomes
//  an iOS system New-Contact sheet — name, org, LinkedIn + website, the why-note, and
//  a photo. `CNContactViewController.forNewContact` is user-confirmed, so it needs no
//  Contacts permission and runs on the Simulator. Plus a real CoreImage QR.
//
//  Camera-dependent pieces (scanning a code, the front-camera selfie) are device-only
//  and land when the Xcode/device upgrade does — they can't run on the Simulator.

import SwiftUI
import UIKit
import Contacts
import ContactsUI
import CoreImage.CIFilterBuiltins

// MARK: - The shareable card

struct ContactCard {
    var givenName: String
    var familyName: String
    var organization: String
    var jobTitle: String
    var email: String?
    var linkedIn: URL?
    var website: URL?
    var note: String
    var photo: UIImage?

    var fullName: String { "\(givenName) \(familyName)".trimmingCharacters(in: .whitespaces) }

    /// What the QR encodes — a real, scannable connect link carrying a preview.
    var qrPayload: String {
        var c = URLComponents(string: "https://wallflowers.io")!
        c.queryItems = [.init(name: "n", value: fullName), .init(name: "o", value: organization)]
        return c.url?.absoluteString ?? "https://wallflowers.io"
    }

    /// Build the system contact. Honest-by-default: only fields we actually have.
    func makeContact() -> CNMutableContact {
        let c = CNMutableContact()
        c.givenName = givenName
        c.familyName = familyName
        c.organizationName = organization
        c.jobTitle = jobTitle
        if let email {
            c.emailAddresses = [CNLabeledValue(label: CNLabelWork, value: email as NSString)]
        }
        var urls: [CNLabeledValue<NSString>] = []
        if let website { urls.append(CNLabeledValue(label: CNLabelURLAddressHomePage, value: website.absoluteString as NSString)) }
        if let linkedIn { urls.append(CNLabeledValue(label: "LinkedIn", value: linkedIn.absoluteString as NSString)) }
        c.urlAddresses = urls
        c.note = note   // the "why" — needs the contacts-notes entitlement to persist on device; fine on the Simulator
        if let data = photo?.jpegData(compressionQuality: 0.9) { c.imageData = data }
        return c
    }
}

extension ContactCard {
    /// Your own card, built from the REAL minted identity — only fields that actually
    /// exist. (The old Sample.me persona exported a fabricated company/email/LinkedIn;
    /// a contact card must never carry invented facts.)
    static func me(displayName: String) -> ContactCard {
        let parts = displayName.split(separator: " ").map(String.init)
        let initials = parts.prefix(2).compactMap(\.first).map(String.init).joined()
        return ContactCard(
            givenName: parts.first ?? displayName,
            familyName: parts.dropFirst().joined(separator: " "),
            organization: "",
            jobTitle: "",
            email: nil,
            linkedIn: nil,
            website: nil,
            note: "Met via WallFlowers",
            photo: BrandImage.avatar(initials.isEmpty ? "?" : initials, tint: .teal)
        )
    }
}

// MARK: - Real QR (CoreImage)

struct QRView: View {
    let payload: String
    var size: CGFloat = 220

    var body: some View {
        Image(uiImage: QRView.image(payload) ?? UIImage())
            .interpolation(.none)
            .resizable().scaledToFit()
            .frame(width: size, height: size)
            .accessibilityLabel("QR code")
    }

    static func image(_ string: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let out = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 12, y: 12)) else { return nil }
        guard let cg = CIContext().createCGImage(out, from: out.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}

// MARK: - Generated photo (stands in for the selfie until the camera lands)

enum BrandImage {
    static func avatar(_ initials: String, tint: Color, size: CGFloat = 256) -> UIImage {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: size, height: size))
        return renderer.image { ctx in
            let base = UIColor(tint)
            let colors = [base.cgColor, base.withAlphaComponent(0.75).cgColor] as CFArray
            if let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1]) {
                ctx.cgContext.drawLinearGradient(grad, start: .zero, end: CGPoint(x: size, y: size), options: [])
            } else {
                base.setFill(); ctx.fill(CGRect(x: 0, y: 0, width: size, height: size))
            }
            let para = NSMutableParagraphStyle(); para.alignment = .center
            let attrs: [NSAttributedString.Key: Any] = [
                .font: UIFont.systemFont(ofSize: size * 0.42, weight: .bold),
                .foregroundColor: UIColor.white,
                .paragraphStyle: para,
            ]
            let text = String(initials.prefix(2)) as NSString
            let sz = text.size(withAttributes: attrs)
            text.draw(at: CGPoint(x: (size - sz.width) / 2, y: (size - sz.height) / 2), withAttributes: attrs)
        }
    }
}

// MARK: - The system New-Contact sheet (user-confirmed → no permission)

struct AddToContactsView: UIViewControllerRepresentable {
    let card: ContactCard
    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> UINavigationController {
        let vc = CNContactViewController(forNewContact: card.makeContact())
        vc.delegate = context.coordinator
        vc.allowsActions = true
        return UINavigationController(rootViewController: vc)
    }

    func updateUIViewController(_ uiViewController: UINavigationController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator { dismiss() } }

    final class Coordinator: NSObject, CNContactViewControllerDelegate {
        let onDone: () -> Void
        init(onDone: @escaping () -> Void) { self.onDone = onDone }
        func contactViewController(_ viewController: CNContactViewController, didCompleteWith contact: CNContact?) {
            onDone()
        }
    }
}
