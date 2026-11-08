//  EnrichedText.swift
//  Renders text with inline enrichment. Two variants share one matcher:
//   • EnrichedLabel — a SwiftUI Text(AttributedString), supports lineLimit/
//     truncation, NON-interactive. Used for row snippets / titles.
//   • EnrichedText  — a UITextView-backed view that supports a reliable per-run
//     LONG-PRESS hit-test (pure SwiftUI Text cannot), used for reading surfaces
//     (note bodies, chat bubbles, work notes). Long-press a matched span → the
//     app-level entity info panel.
//
//  Both resolve matches through EnrichmentService (from the environment) and
//  restyle only the matched spans: bold + accent colour. Unmatched text is
//  untouched. Empty catalogue ⇒ plain text (honest).

import SwiftUI
import UIKit

private let kEntityId = NSAttributedString.Key("pacific.entityId")

// MARK: - Non-interactive label (truncatable)

struct EnrichedLabel: View {
    let text: String
    var font: Font = .body
    var color: Color = .primary
    var accent: Color = Brand.accent

    @EnvironmentObject private var enrich: EnrichmentService

    init(_ text: String, font: Font = .body, color: Color = .primary, accent: Color = Brand.accent) {
        self.text = text; self.font = font; self.color = color; self.accent = accent
    }

    var body: some View {
        // `enrich.version` is read implicitly via the matches call re-running when
        // the observed object publishes.
        Text(attributed)
            .font(font)
            .foregroundStyle(color)
    }

    private var attributed: AttributedString {
        var attr = AttributedString(text)
        let matches = enrich.allMatches(in: text)
        guard !matches.isEmpty else { return attr }
        for m in matches {
            let lo = text.distance(from: text.startIndex, to: m.range.lowerBound)
            let len = text.distance(from: m.range.lowerBound, to: m.range.upperBound)
            let start = attr.index(attr.startIndex, offsetByCharacters: lo)
            let end = attr.index(start, offsetByCharacters: len)
            attr[start..<end].foregroundColor = enrich.entity(m.entityId)?.kind.tint ?? accent
            attr[start..<end].font = font.weight(.semibold)
        }
        return attr
    }
}

// MARK: - Interactive text (long-press → info panel)

struct EnrichedText: View {
    let text: String
    var font: UIFont
    var color: Color
    var accent: Color
    var selectable: Bool
    /// Reports, AT TOUCH-DOWN, whether the finger landed on a matched entity span —
    /// ~350ms before any long-press can fire. Hosts whose own long-press must yield
    /// to the entity panel (the chat react bar) read this instead of racing gesture
    /// arbitration against SwiftUI's private recognizers.
    var onEntityPress: ((Bool) -> Void)?

    @EnvironmentObject private var enrich: EnrichmentService

    init(_ text: String,
         font: UIFont = .preferredFont(forTextStyle: .body),
         color: Color = .primary,
         accent: Color = Brand.accent,
         selectable: Bool = false,
         onEntityPress: ((Bool) -> Void)? = nil) {
        self.text = text; self.font = font; self.color = color; self.accent = accent
        self.selectable = selectable
        self.onEntityPress = onEntityPress
    }

    var body: some View {
        _EnrichedTextView(
            attributed: attributed,
            selectable: selectable,
            onSelect: { id in
                if let e = enrich.entity(id) { enrich.inspected = e }
            },
            onEntityPress: onEntityPress
        )
    }

    private var attributed: NSAttributedString {
        let ns = NSMutableAttributedString(
            string: text,
            attributes: [.font: font, .foregroundColor: UIColor(color)]
        )
        let matches = enrich.allMatches(in: text)
        guard !matches.isEmpty else { return ns }
        let bold = boldVariant(of: font)
        for m in matches {
            let nsRange = NSRange(m.range, in: text)
            let color = UIColor(enrich.entity(m.entityId)?.kind.tint ?? accent)
            ns.addAttributes(
                [.font: bold, .foregroundColor: color, kEntityId: m.entityId],
                range: nsRange
            )
        }
        return ns
    }

    private func boldVariant(of f: UIFont) -> UIFont {
        let traits = f.fontDescriptor.symbolicTraits.union(.traitBold)
        if let d = f.fontDescriptor.withSymbolicTraits(traits) {
            return UIFont(descriptor: d, size: f.pointSize)
        }
        return f
    }
}

/// The UITextView bridge: non-editable, non-scrolling, self-sizing, with a
/// long-press recognizer that maps the pressed glyph → its entity id.
private struct _EnrichedTextView: UIViewRepresentable {
    let attributed: NSAttributedString
    var selectable: Bool = false
    let onSelect: (String) -> Void
    var onEntityPress: ((Bool) -> Void)? = nil

    func makeCoordinator() -> Coordinator { Coordinator(onSelect: onSelect) }

    func makeUIView(context: Context) -> EnrichedTextView {
        let tv = EnrichedTextView()
        tv.isEditable = false
        tv.isSelectable = selectable       // note/work read surfaces allow select/copy
        tv.isScrollEnabled = false
        tv.backgroundColor = .clear
        tv.textContainerInset = .zero
        tv.textContainer.lineFragmentPadding = 0
        tv.adjustsFontForContentSizeCategory = true
        tv.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let lp = UILongPressGestureRecognizer(target: context.coordinator,
                                              action: #selector(Coordinator.handleLongPress(_:)))
        lp.minimumPressDuration = 0.35
        lp.delegate = context.coordinator   // fire alongside UITextView's own selection gesture
        lp.cancelsTouchesInView = false     // recognition must not cancel the touch mid-press —
                                            // touchesEnded/Cancelled drive the onEntityPress reset
        tv.addGestureRecognizer(lp)
        context.coordinator.textView = tv
        return tv
    }

    func updateUIView(_ tv: EnrichedTextView, context: Context) {
        tv.attributedText = attributed
        tv.onEntityPress = onEntityPress
        context.coordinator.onSelect = onSelect
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: EnrichedTextView, context: Context) -> CGSize? {
        let maxW = proposal.width ?? UIScreen.main.bounds.width
        let fit = uiView.sizeThatFits(CGSize(width: maxW, height: .greatestFiniteMagnitude))
        return CGSize(width: min(ceil(fit.width), maxW), height: ceil(fit.height))
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var onSelect: (String) -> Void
        weak var textView: EnrichedTextView?
        init(onSelect: @escaping (String) -> Void) { self.onSelect = onSelect }

        func gestureRecognizer(_ g: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

        /// The touch-down stamp point. Delegate-level (recognizer touch delivery) rather
        /// than the view's touchesBegan, which UITextView does not reliably forward.
        func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            if let tv = textView { tv.stampPress(at: touch.location(in: tv)) }
            return true
        }

        @objc func handleLongPress(_ g: UILongPressGestureRecognizer) {
            guard let tv = textView else { return }
            switch g.state {
            case .began:
                // Consume the id captured at touch-down (never a fresh hit-test): the
                // react-bar gate and this panel must agree on what was pressed, drift
                // or no drift.
                if let id = tv.pressedEntityId {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    onSelect(id)
                }
            case .ended, .cancelled, .failed:
                tv.pressEnded()
            default:
                break
            }
        }
    }
}

/// A UITextView that exposes ONE accessibility element per matched entity span, so each
/// resolved entity is individually addressable (VoiceOver, and XCUITest, which targets
/// `otherElements["enriched:<entityId>"]` and long-presses its centre — the same point the
/// gesture recognizer hit-tests). Built lazily on access so glyph layout is complete.
private final class EnrichedTextView: UITextView {
    /// See EnrichedText.onEntityPress. Stamped true/false at touch-down; reset on the
    /// NEXT runloop tick after the touch ends, so a long-press closure firing in the
    /// same tick as the touch's end/cancel still reads the value from ITS touch-down.
    var onEntityPress: ((Bool) -> Void)?

    /// The entity under the CURRENT press, hit-tested ONCE at touch-down (the LP
    /// delegate's shouldReceive). The long-press action consumes this id rather than
    /// re-hit-testing at fire time: both recognizers tolerate ~10pt of drift over the
    /// 350ms hold, so a fresh hit-test could disagree with the touch-down stamp
    /// (panel + react bar both opening, or neither).
    private(set) var pressedEntityId: String?
    /// Stamp generation. A queued reset only lands if no newer touch re-stamped —
    /// a stale touch's reset can never wipe a press still in flight.
    private var pressSeq = 0

    /// The entity span under `point` (view coordinates), or nil.
    func entityId(at point: CGPoint) -> String? {
        let p = CGPoint(x: point.x - textContainerInset.left,
                        y: point.y - textContainerInset.top)
        var frac: CGFloat = 0
        let idx = layoutManager.characterIndex(for: p, in: textContainer,
                                               fractionOfDistanceBetweenInsertionPoints: &frac)
        guard idx >= 0, idx < textStorage.length else { return nil }
        return textStorage.attribute(kEntityId, at: idx, effectiveRange: nil) as? String
    }

    /// Touch-down: one hit-test, stamped for both the react-bar gate and the panel.
    func stampPress(at point: CGPoint) {
        pressSeq += 1
        pressedEntityId = entityId(at: point)
        onEntityPress?(pressedEntityId != nil)
    }

    /// Touch over (LP ended/cancelled/failed, or view-level touch end when UIKit
    /// delivers it). Reset on the NEXT tick so a long-press closure firing in the
    /// same runloop drain still reads its own stamp.
    func pressEnded() {
        let seq = pressSeq
        DispatchQueue.main.async { [weak self] in
            guard let self, self.pressSeq == seq else { return }   // a newer press re-stamped
            self.pressedEntityId = nil
            self.onEntityPress?(false)
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        pressEnded()
        super.touchesEnded(touches, with: event)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        pressEnded()
        super.touchesCancelled(touches, with: event)
    }

    // Per-span accessibility: one REAL (transparent, non-interactive) overlay subview per
    // matched entity span, so each resolved entity is individually addressable (VoiceOver,
    // and XCUITest, which targets "enriched:<entityId>" and long-presses its centre — the
    // press falls through to the text view, whose touch-down hit-test does the rest).
    // NOT a UIAccessibilityElement container: UITextView's own accessibility machinery
    // mangles a custom accessibilityElements array with multiple spans (observed: only the
    // last span per view survives, with ids/labels crossed between views). Real subviews
    // sidestep that entirely.
    private var spanOverlays: [UIView] = []

    override var attributedText: NSAttributedString! {
        didSet { setNeedsLayout() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        rebuildSpanOverlays()
    }

    private func rebuildSpanOverlays() {
        spanOverlays.forEach { $0.removeFromSuperview() }
        spanOverlays = []
        layoutManager.ensureLayout(for: textContainer)
        let ns = textStorage.string as NSString
        textStorage.enumerateAttribute(kEntityId, in: NSRange(location: 0, length: textStorage.length)) { value, range, _ in
            guard let id = value as? String else { return }
            let glyphs = layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let rect = layoutManager.boundingRect(forGlyphRange: glyphs, in: textContainer)
                .offsetBy(dx: textContainerInset.left, dy: textContainerInset.top)
            let v = UIView(frame: rect)
            v.backgroundColor = .clear
            v.isUserInteractionEnabled = false        // touches flow through to the text view
            v.isAccessibilityElement = true
            v.accessibilityIdentifier = "enriched:\(id)"
            v.accessibilityLabel = ns.substring(with: range)
            v.accessibilityTraits = .button
            addSubview(v)
            spanOverlays.append(v)
        }
    }
}
